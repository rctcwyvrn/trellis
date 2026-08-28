//! The thin client (plan 03 scope 8): parse arguments, discover the
//! Soil root, ensure a daemon, forward, render. Argument parsing is
//! hand-rolled (dependency-floor precedent, impl plans 01–02).

use std::path::PathBuf;

use crate::config;
use crate::daemon;
use crate::diag::{ErrorReport, EXIT_OK, EXIT_USAGE};
use crate::rpc;

const USAGE: &str = "usage: trellis <command> [args] | trellis --version\n\
     commands: daemon run|stop|status, check, status, refresh, lower, test,\n\
     call, repl, context, answer, skill, toolchain";

/// Commands forwarded to the daemon as same-named methods (step 2:
/// they dispatch, hit the pin gate where mutating, and report
/// unimplemented — the full path exists before the features do).
const FORWARDED: [&str; 11] = [
    "check",
    "status",
    "refresh",
    "lower",
    "test",
    "call",
    "repl",
    "context",
    "answer",
    "skill",
    "toolchain",
];

pub fn run(args: &[String]) -> i32 {
    let mut args = args.iter().map(String::as_str);
    match args.next() {
        None => usage(),
        Some("--version") => {
            println!(
                "{}",
                serde_json::json!({
                    "trellis": config::trellis_version(),
                    "daemon_contract": 1,
                })
            );
            EXIT_OK
        }
        Some("daemon") => daemon_command(&args.collect::<Vec<_>>()),
        Some(command) if FORWARDED.contains(&command) => {
            forward(command, &args.collect::<Vec<_>>())
        }
        Some(_) => usage(),
    }
}

fn usage() -> i32 {
    eprintln!("{}", ErrorReport::one("usage", USAGE).render());
    EXIT_USAGE
}

fn daemon_command(args: &[&str]) -> i32 {
    match args.first().copied() {
        Some("run") => {
            let root = match parse_root_flag(&args[1..]) {
                Ok(Some(root)) => root,
                Ok(None) => match discovered_root() {
                    Ok(root) => root,
                    Err(code) => return code,
                },
                Err(code) => return code,
            };
            daemon::run(&root)
        }
        Some("stop") => {
            let root = match discovered_root() {
                Ok(root) => root,
                Err(code) => return code,
            };
            match rpc::Client::connect(&root) {
                Ok(mut client) => {
                    let stopped = client.call("shutdown", serde_json::json!({})).is_ok();
                    println!("{}", serde_json::json!({ "stopped": stopped }));
                    EXIT_OK
                }
                Err(_) => {
                    println!("{}", serde_json::json!({ "stopped": false }));
                    EXIT_OK
                }
            }
        }
        Some("status") => {
            let root = match discovered_root() {
                Ok(root) => root,
                Err(code) => return code,
            };
            let status = rpc::Client::connect(&root)
                .ok()
                .and_then(|mut client| client.call("daemon_status", serde_json::json!({})).ok())
                .and_then(Result::ok);
            match status {
                Some(mut value) => {
                    if let Some(object) = value.as_object_mut() {
                        object.insert("running".into(), serde_json::json!(true));
                    }
                    println!("{value}");
                }
                None => println!("{}", serde_json::json!({ "running": false })),
            }
            EXIT_OK
        }
        _ => usage(),
    }
}

fn forward(command: &str, args: &[&str]) -> i32 {
    let root = match discovered_root() {
        Ok(root) => root,
        Err(code) => return code,
    };
    let mut client = match rpc::ensure_daemon(&root) {
        Ok(client) => client,
        Err(report) => return render_error(&report),
    };
    // `trellis toolchain update` is method `toolchain_update`.
    let (method, args) = if command == "toolchain" && args.first() == Some(&"update") {
        ("toolchain_update".to_string(), &args[1..])
    } else {
        (command.to_string(), args)
    };
    let params = serde_json::json!({ "args": args });
    match client.call(&method, params) {
        Ok(Ok(result)) => {
            println!("{result}");
            EXIT_OK
        }
        Ok(Err(error)) => {
            let report = error
                .data
                .and_then(|data| serde_json::from_value::<ErrorReport>(data).ok())
                .unwrap_or_else(|| ErrorReport::one("internal", error.message));
            render_error(&report)
        }
        Err(e) => render_error(&ErrorReport::one(
            "io",
            format!("daemon connection lost: {e}"),
        )),
    }
}

fn render_error(report: &ErrorReport) -> i32 {
    eprintln!("{}", report.render());
    report.exit_code()
}

fn discovered_root() -> Result<PathBuf, i32> {
    let cwd = std::env::current_dir().map_err(|e| {
        render_error(&ErrorReport::one(
            "io",
            format!("no working directory: {e}"),
        ))
    })?;
    config::find_root(&cwd).map_err(|report| render_error(&report))
}

fn parse_root_flag(args: &[&str]) -> Result<Option<PathBuf>, i32> {
    match args {
        [] => Ok(None),
        ["--root", root] => Ok(Some(PathBuf::from(root))),
        _ => Err(usage()),
    }
}
