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
const MUTATING: [&str; 6] = [
    "lower",
    "test",
    "refresh",
    "answer",
    "toolchain_update",
    "decisions_editorial",
];
const READ_ONLY: [&str; 5] = ["check", "status", "context", "repl", "skill"];

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
                // The soil0 interpreter and checker recurse with the
                // program (deep `let rec` evaluation, expression
                // walking); the 2 MiB spawned-thread default is too
                // small for real test runs, so connection threads get
                // a deliberately generous stack.
                let _ = std::thread::Builder::new()
                    .stack_size(256 * 1024 * 1024)
                    .spawn(move || handle_connection(stream, &state));
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
        "status" => match status_report(state) {
            Ok(report) => Response::ok(id, report),
            Err(report) => diag_error(id, &report),
        },
        "check" => {
            let run = config::load(&state.root)
                .and_then(|config| crate::state::scan(&state.root, &config))
                .and_then(|root| crate::state::check(&root));
            match run {
                Ok(out) => Response::ok(id, out),
                Err(report) => diag_error(id, &report),
            }
        }
        "context" => match context_command(state, &request.params) {
            Ok(out) => Response::ok(id, out),
            Err(report) => diag_error(id, &report),
        },
        method if MUTATING.contains(&method) => {
            // The pin gate, live from the first command (soil-toml §2.1).
            let gate = config::load(&state.root).and_then(|c| {
                config::check_pin(&c)?;
                Ok(c)
            });
            let config = match gate {
                Err(report) => return diag_error(id, &report),
                Ok(config) => config,
            };
            match method {
                "test" => {
                    let target = request
                        .params
                        .get("args")
                        .and_then(|a| a.get(0))
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    match crate::testrun::run(&state.root, &config, target.as_deref()) {
                        Ok(report) => Response::ok(
                            id,
                            serde_json::to_value(&report).expect("report serializes"),
                        ),
                        Err(report) => diag_error(id, &report),
                    }
                }
                "refresh" => match crate::state::refresh(&state.root, &config) {
                    Ok(report) => Response::ok(
                        id,
                        serde_json::to_value(&report).expect("report serializes"),
                    ),
                    Err(report) => diag_error(id, &report),
                },
                "decisions_editorial" => {
                    let label = request
                        .params
                        .get("args")
                        .and_then(|a| a.get(0))
                        .and_then(|v| v.as_str())
                        .map(str::to_string);
                    match label {
                        None => Response::err(
                            id,
                            rpc::RPC_DIAG,
                            "usage",
                            Some(
                                serde_json::to_value(ErrorReport::one(
                                    "usage",
                                    "usage: trellis decisions editorial <label>",
                                ))
                                .expect("report serializes"),
                            ),
                        ),
                        Some(label) => {
                            match crate::state::editorial(&state.root, &config, &label) {
                                Ok(restamped) => {
                                    Response::ok(id, serde_json::json!({ "restamped": restamped }))
                                }
                                Err(report) => diag_error(id, &report),
                            }
                        }
                    }
                }
                _ => unimplemented(id, method),
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

/// `trellis context <def> --budget <n> [--out <dir>]` (contract §5):
/// assemble the bundle, write it to the default `.trellis/context/…`
/// (wiped per invocation) or the caller's `--out` (must be empty),
/// return the manifest.
fn context_command(
    state: &State,
    params: &serde_json::Value,
) -> Result<serde_json::Value, ErrorReport> {
    let args: Vec<&str> = params
        .get("args")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let usage = || {
        ErrorReport::one(
            "usage",
            "usage: trellis context <def> --budget <n> [--out <dir>]",
        )
    };
    let mut def: Option<&str> = None;
    let mut budget: Option<u64> = None;
    let mut out: Option<&str> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match *arg {
            "--budget" => {
                budget = Some(iter.next().and_then(|v| v.parse().ok()).ok_or_else(usage)?)
            }
            "--out" => out = Some(iter.next().ok_or_else(usage)?),
            _ if def.is_none() => def = Some(arg),
            _ => return Err(usage()),
        }
    }
    let (Some(def), Some(budget)) = (def, budget) else {
        return Err(usage());
    };

    let config = config::load(&state.root)?;
    let root = crate::state::scan(&state.root, &config)?;
    let entry = root
        .entries
        .values()
        .find(|e| e.tr.name == def || e.rel == def)
        .ok_or_else(|| ErrorReport::one("usage", format!("no definition `{def}` in this root")))?;
    let bundle = crate::bundle::assemble(&root, entry, budget)?;
    let (out_dir, allow_wipe) = match out {
        Some(dir) => (PathBuf::from(dir), false),
        None => (state.root.join(".trellis/context").join(&entry.rel), true),
    };
    crate::bundle::write_to(&bundle, &out_dir, allow_wipe)?;
    Ok(serde_json::json!({
        "out": out_dir.display().to_string(),
        "manifest": bundle.manifest,
        "rendered": bundle.manifest_text,
    }))
}

fn status_report(state: &State) -> Result<serde_json::Value, ErrorReport> {
    let config = config::load(&state.root)?;
    let root = crate::state::scan(&state.root, &config)?;
    let report = match crate::state::compile(&root) {
        Ok(compiled) => crate::state::statuses(&root, &compiled.soil_hashes, &compiled.refs),
        Err(err) => {
            let mut report =
                crate::state::statuses(&root, &Default::default(), &Default::default());
            report.errors = err.errors;
            report
        }
    };
    Ok(serde_json::to_value(&report).expect("report serializes"))
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
