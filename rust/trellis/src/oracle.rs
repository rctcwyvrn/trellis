//! Differential oracles (impl plan 03 §9.9): the interim `python3`
//! subprocess driver for `reference` attachments and the pipe driver
//! for CLI oracles. The lock is transport-agnostic (tier + oracle
//! hashes), so plan 06 swaps in the real FFI without a schema change.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::diag::Diag;
use crate::hash;

fn oracle_err(message: impl Into<String>) -> Diag {
    Diag::new("oracle-error", message)
}

/// Hash a reference file into the `oracles` row (the repo digest
/// convention: length-prefixed SHA-256, `hash::digest`).
pub fn hash_reference(root: &Path, rel: &str) -> Result<String, Diag> {
    let bytes = std::fs::read(root.join(rel))
        .map_err(|e| oracle_err(format!("reference `{rel}` is unreadable: {e}")))?;
    Ok(hash::digest(&[&bytes]))
}

/// Hash a CLI oracle's executable (argv[0] resolved on `PATH`).
pub fn hash_cli(command: &str) -> Result<String, Diag> {
    let argv0 = command
        .split_whitespace()
        .next()
        .ok_or_else(|| oracle_err("empty CLI oracle command"))?;
    let path = if argv0.contains('/') {
        std::path::PathBuf::from(argv0)
    } else {
        std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths).find_map(|dir| {
                    let candidate = dir.join(argv0);
                    candidate.is_file().then_some(candidate)
                })
            })
            .ok_or_else(|| oracle_err(format!("CLI oracle `{argv0}` is not on PATH")))?
    };
    let bytes = std::fs::read(&path)
        .map_err(|e| oracle_err(format!("CLI oracle `{}`: {e}", path.display())))?;
    Ok(hash::digest(&[&bytes]))
}

const PY_DRIVER: &str = "\
import importlib.util, json, sys\n\
spec = importlib.util.spec_from_file_location('trellis_reference', sys.argv[1])\n\
mod = importlib.util.module_from_spec(spec)\n\
spec.loader.exec_module(mod)\n\
result = getattr(mod, sys.argv[2])(*json.load(sys.stdin))\n\
print(json.dumps(result))\n";

/// Run `relpath::symbol` on one JSON argument list; the reply is the
/// reference's result, parsed but not yet canonicalized.
pub fn run_python(
    root: &Path,
    rel: &str,
    symbol: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, Diag> {
    let abs = root.join(rel);
    run_pipe(
        Command::new("python3")
            .arg("-B") // never write __pycache__ beside the reference
            .arg("-c")
            .arg(PY_DRIVER)
            .arg(&abs)
            .arg(symbol),
        &format!("{rel}::{symbol}"),
        args,
    )
}

/// Run a CLI oracle: the JSON argument list on stdin, JSON out.
pub fn run_cli(
    root: &Path,
    command: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, Diag> {
    let mut parts = command.split_whitespace();
    let argv0 = parts
        .next()
        .ok_or_else(|| oracle_err("empty CLI oracle command"))?;
    run_pipe(
        Command::new(argv0).args(parts).current_dir(root),
        command,
        args,
    )
}

fn run_pipe(
    command: &mut Command,
    what: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, Diag> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| oracle_err(format!("oracle `{what}` failed to start: {e}")))?;
    let payload = serde_json::to_string(args).expect("args serialize");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(payload.as_bytes())
        .map_err(|e| oracle_err(format!("oracle `{what}`: stdin: {e}")))?;
    let out = child
        .wait_with_output()
        .map_err(|e| oracle_err(format!("oracle `{what}`: {e}")))?;
    if !out.status.success() {
        return Err(oracle_err(format!(
            "oracle `{what}` exited {}: {}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    serde_json::from_slice(&out.stdout)
        .map_err(|e| oracle_err(format!("oracle `{what}` returned non-JSON output: {e}")))
}
