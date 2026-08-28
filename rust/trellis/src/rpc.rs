//! JSON-RPC 2.0 over the Unix socket, one compact JSON document per
//! line (ndjson — impl plan 03 §8.2). Internal, non-contractual: both
//! ends live in this binary and ship together.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::diag::ErrorReport;

/// JSON-RPC application error codes (`error.data` carries an
/// `ErrorReport` when the failure is a structured diagnostic).
pub const RPC_DIAG: i64 = -32000;
pub const RPC_UNIMPLEMENTED: i64 = -32001;
pub const RPC_METHOD_NOT_FOUND: i64 = -32601;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: u64, code: i64, message: &str, data: Option<serde_json::Value>) -> Self {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.to_string(),
                data,
            }),
        }
    }
}

pub fn write_msg<W: Write, T: Serialize>(writer: &mut W, message: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(message)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

/// `Ok(None)` on clean EOF.
pub fn read_msg<R: BufRead, T: DeserializeOwned>(reader: &mut R) -> io::Result<Option<T>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    serde_json::from_str(&line)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// `$XDG_RUNTIME_DIR/trellis/<root-hash>.sock` (plan 03 resolved);
/// root-hash is the first 16 hex of SHA-256 over the canonicalized
/// absolute root path (impl plan 03 §8.1).
pub fn socket_path(root: &Path) -> PathBuf {
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let digest = Sha256::digest(canon.as_os_str().as_encoded_bytes());
    let hash: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    runtime_dir().join("trellis").join(format!("{hash}.sock"))
}

fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let user = std::env::var("USER").unwrap_or_else(|_| "anon".into());
            std::env::temp_dir().join(format!("trellis-{user}"))
        }
    }
}

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Client {
    pub fn connect(root: &Path) -> io::Result<Client> {
        let stream = UnixStream::connect(socket_path(root))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Client {
            reader,
            writer: stream,
            next_id: 1,
        })
    }

    pub fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> io::Result<Result<serde_json::Value, RpcError>> {
        let id = self.next_id;
        self.next_id += 1;
        write_msg(
            &mut self.writer,
            &Request {
                jsonrpc: "2.0".into(),
                id,
                method: method.to_string(),
                params,
            },
        )?;
        let response: Response = read_msg(&mut self.reader)?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "daemon closed the connection")
        })?;
        match (response.result, response.error) {
            (Some(result), None) => Ok(Ok(result)),
            (None, Some(error)) => Ok(Err(error)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed JSON-RPC response",
            )),
        }
    }
}

/// Connect to the root's daemon, spawning one if needed (impl plan 03
/// §8.1): stale socket files are removed, a client/daemon version
/// mismatch restarts the daemon (one binary — mismatch means an
/// upgrade happened).
pub fn ensure_daemon(root: &Path) -> Result<Client, ErrorReport> {
    for attempt in 0..2 {
        match try_connect(root) {
            Ok(Some(client)) => return Ok(client),
            Ok(None) if attempt == 0 => {} // version mismatch: daemon was told to shut down
            Ok(None) => {
                return Err(ErrorReport::one(
                    "internal",
                    "daemon version mismatch persisted across a restart",
                ))
            }
            Err(_) => {}
        }
        spawn_daemon(root)?;
        // Wait for the freshly spawned daemon to bind.
        for _ in 0..250 {
            if let Ok(Some(client)) = try_connect(root) {
                return Ok(client);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    Err(ErrorReport::one(
        "internal",
        format!("could not reach or start a daemon for {}", root.display()),
    ))
}

/// `Ok(Some)` connected and version-matched; `Ok(None)` connected but
/// mismatched (shutdown was requested); `Err` no daemon reachable.
fn try_connect(root: &Path) -> io::Result<Option<Client>> {
    let mut client = Client::connect(root)?;
    let hello = client
        .call("hello", serde_json::json!({}))?
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.message))?;
    if hello.get("version").and_then(|v| v.as_str()) == Some(crate::config::trellis_version()) {
        Ok(Some(client))
    } else {
        let _ = client.call("shutdown", serde_json::json!({}));
        Ok(None)
    }
}

fn spawn_daemon(root: &Path) -> Result<(), ErrorReport> {
    // A dead daemon leaves its socket file behind; remove it so the
    // new daemon can bind (stale-socket recovery).
    let _ = std::fs::remove_file(socket_path(root));
    let exe = std::env::current_exe()
        .map_err(|e| ErrorReport::one("internal", format!("cannot find own binary: {e}")))?;
    std::process::Command::new(exe)
        .args(["daemon", "run", "--root"])
        .arg(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| ErrorReport::one("internal", format!("cannot spawn daemon: {e}")))?;
    Ok(())
}
