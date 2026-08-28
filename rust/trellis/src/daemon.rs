//! The daemon: one process per Soil root, a Unix socket server with
//! one thread per connection (impl plan 03 §8.1–§8.2). Step 2 serves
//! the lifecycle methods; every future command is dispatched here
//! already so the pin gate and the unimplemented discipline are in
//! place from the first commit.

use std::io::BufReader;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config;
use crate::diag::ErrorReport;
use crate::rpc::{self, Request, Response};

/// Commands that will write locks or spend tokens: these refuse under
/// a toolchain mismatch (soil-toml §2.1). Read-only commands run
/// under a mismatch so a foreign-pinned root stays inspectable.
const MUTATING: [&str; 5] = ["lower", "test", "refresh", "answer", "toolchain_update"];
const READ_ONLY: [&str; 6] = ["check", "status", "context", "call", "repl", "skill"];

struct State {
    root: PathBuf,
    socket: PathBuf,
}

pub fn run(root: &Path) -> i32 {
    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(e) => {
            eprintln!(
                "{}",
                ErrorReport::one("io", format!("bad root {}: {e}", root.display())).render()
            );
            return crate::diag::EXIT_USAGE;
        }
    };
    let socket = rpc::socket_path(&root);

    // Another live daemon for this root wins; exit quietly so racing
    // auto-spawns converge on one server.
    if UnixStream::connect(&socket).is_ok() {
        return crate::diag::EXIT_OK;
    }
    let _ = std::fs::remove_file(&socket);
    if let Some(parent) = socket.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!(
                "{}",
                ErrorReport::one("io", format!("cannot bind {}: {e}", socket.display())).render()
            );
            return crate::diag::EXIT_USAGE;
        }
    };

    let state = Arc::new(State { root, socket });
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                std::thread::spawn(move || handle_connection(stream, &state));
            }
            Err(_) => continue,
        }
    }
    crate::diag::EXIT_OK
}

fn handle_connection(stream: UnixStream, state: &State) {
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    while let Ok(Some(request)) = rpc::read_msg::<_, Request>(&mut reader) {
        let shutdown = request.method == "shutdown";
        let response = dispatch(request, state);
        if rpc::write_msg(&mut writer, &response).is_err() {
            return;
        }
        if shutdown {
            let _ = std::fs::remove_file(&state.socket);
            std::process::exit(crate::diag::EXIT_OK);
        }
    }
}

fn dispatch(request: Request, state: &State) -> Response {
    let id = request.id;
    match request.method.as_str() {
        "hello" => Response::ok(
            id,
            serde_json::json!({ "version": config::trellis_version() }),
        ),
        "daemon_status" => Response::ok(
            id,
            serde_json::json!({
                "root": state.root.display().to_string(),
                "version": config::trellis_version(),
            }),
        ),
        "shutdown" => Response::ok(id, serde_json::json!({ "ok": true })),
        method if MUTATING.contains(&method) => {
            // The pin gate, live from the first command (soil-toml §2.1).
            let gate = config::load(&state.root).and_then(|c| config::check_pin(&c));
            match gate {
                Err(report) => diag_error(id, &report),
                Ok(()) => unimplemented(id, method),
            }
        }
        method if READ_ONLY.contains(&method) => unimplemented(id, method),
        method => Response::err(
            id,
            rpc::RPC_METHOD_NOT_FOUND,
            &format!("unknown method `{method}`"),
            None,
        ),
    }
}

fn diag_error(id: u64, report: &ErrorReport) -> Response {
    Response::err(
        id,
        rpc::RPC_DIAG,
        &report.errors[0].code.clone(),
        Some(serde_json::to_value(report).expect("report serializes")),
    )
}

fn unimplemented(id: u64, method: &str) -> Response {
    let report = ErrorReport::one(
        "unimplemented",
        format!("`trellis {method}` arrives with a later step of impl plan 03"),
    );
    Response::err(
        id,
        rpc::RPC_UNIMPLEMENTED,
        "unimplemented",
        Some(serde_json::to_value(&report).expect("report serializes")),
    )
}
