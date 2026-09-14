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
/// `call` is deliberately absent: it executes in the client (resolved
/// 2026-08-28, step 8) so relative paths in the real `Fs` resolve
/// against the caller's working directory — the cram temp dir, not
/// the daemon's cwd.
const FORWARDED: [&str; 11] = [
    "check",
    "status",
    "refresh",
    "lower",
    "test",
    "repl",
    "context",
    "answer",
    "skill",
    "toolchain",
    "decisions",
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
        Some("call") => call_command(&args.collect::<Vec<_>>()),
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

/// `trellis call <def> <json-arg>…` (tr-grammar §3.5, contract §8.6
/// semantics): client-local — compile the root through the linked
/// soil0, inject real `World`-derived capabilities in order, decode
/// the JSON args positionally, print the result's canonical JSON.
/// Read-only: no pin gate, no lock writes, no daemon.
fn call_command(args: &[&str]) -> i32 {
    let Some((def, json_args)) = args.split_first() else {
        eprintln!(
            "{}",
            ErrorReport::one("usage", "usage: trellis call <def> <json-arg>…").render()
        );
        return EXIT_USAGE;
    };
    let mut parsed = Vec::new();
    for arg in json_args {
        match serde_json::from_str::<serde_json::Value>(arg) {
            Ok(v) => parsed.push(v),
            Err(e) => {
                return render_error(&ErrorReport::one(
                    "usage",
                    format!("argument `{arg}` is not JSON: {e}"),
                ))
            }
        }
    }
    let root = match discovered_root() {
        Ok(root) => root,
        Err(code) => return code,
    };
    let run = || -> Result<String, ErrorReport> {
        let config = crate::config::load(&root)?;
        let scanned = crate::state::scan(&root, &config)?;
        let assembly = crate::state::assemble(&scanned)?;
        let soil0_report = |d: soil0::diag::Diagnostic| ErrorReport {
            errors: vec![crate::registry::enrich(crate::diag::Diag::from_soil0(&d))],
        };
        let program =
            soil0::manifest::from_parts(assembly.env, &assembly.ordered).map_err(soil0_report)?;
        let session = soil0::interp::session(program).map_err(soil0_report)?;
        soil0::interp::run_json(&session, def, &parsed).map_err(soil0_report)
    };
    match run() {
        Ok(json) => {
            println!("{json}");
            EXIT_OK
        }
        Err(report) => render_error(&report),
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
    // Subcommand spellings: `trellis toolchain update`,
    // `trellis decisions editorial <label>`.
    let (method, args) = if command == "toolchain" && args.first() == Some(&"update") {
        ("toolchain_update".to_string(), &args[1..])
    } else if command == "decisions" && args.first() == Some(&"editorial") {
        ("decisions_editorial".to_string(), &args[1..])
    } else {
        (command.to_string(), args)
    };
    let json_output = args.contains(&"--json");
    let args: Vec<&str> = args.iter().copied().filter(|a| *a != "--json").collect();
    // `--out` is resolved client-side: the daemon's cwd is not ours.
    let mut resolved: Vec<String> = Vec::with_capacity(args.len());
    let mut absolutize_next = false;
    for arg in &args {
        if absolutize_next {
            absolutize_next = false;
            let path = std::path::Path::new(arg);
            let abs = if path.is_absolute() {
                path.to_path_buf()
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            };
            resolved.push(abs.display().to_string());
        } else {
            absolutize_next = method == "context" && *arg == "--out";
            resolved.push((*arg).to_string());
        }
    }
    let params = serde_json::json!({ "args": resolved });
    match client.call(&method, params) {
        Ok(Ok(result)) => {
            if method == "status" && !json_output {
                render_status(&result);
                EXIT_OK
            } else if method == "test" && !json_output {
                render_test(&result)
            } else if method == "context" && !json_output {
                render_context(&result);
                EXIT_OK
            } else {
                println!("{result}");
                EXIT_OK
            }
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

/// The human table (`--json` for the raw report).
fn render_status(report: &serde_json::Value) {
    let defs = report.get("defs").and_then(|d| d.as_array());
    let Some(defs) = defs else {
        println!("{report}");
        return;
    };
    let field = |v: &serde_json::Value, k: &str| -> String {
        v.get(k).and_then(|x| x.as_str()).unwrap_or("-").to_string()
    };
    let width = defs
        .iter()
        .map(|d| field(d, "path").len())
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{:width$}  {:8}  {:9}  flags", "path", "kind", "status");
    for def in defs {
        let flags: Vec<String> = def
            .get("flags")
            .and_then(|f| f.as_array())
            .map(|f| {
                f.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        println!(
            "{:width$}  {:8}  {:9}  {}",
            field(def, "path"),
            field(def, "kind"),
            field(def, "status"),
            if flags.is_empty() {
                "-".to_string()
            } else {
                flags.join(",")
            }
        );
    }
    if let Some(errors) = report.get("errors").and_then(|e| e.as_array()) {
        for error in errors {
            eprintln!("error: {}", error);
        }
    }
}

/// The human test summary (`--json` for the raw report): one line per
/// definition, failing rows and diagnostics spelled out beneath.
/// Exit 1 when any row fails (or xpasses) or any diagnostic fired —
/// pre-flight findings are spec bugs, not noise.
fn render_test(report: &serde_json::Value) -> i32 {
    let Some(defs) = report.get("defs").and_then(|d| d.as_array()) else {
        println!("{report}");
        return EXIT_OK;
    };
    let mut failing = false;
    for def in defs {
        let path = def.get("path").and_then(|p| p.as_str()).unwrap_or("-");
        let rows = def
            .get("tests")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let mut passed = 0usize;
        let mut bad: Vec<(String, String)> = Vec::new();
        for row in &rows {
            let name = row.get("name").and_then(|n| n.as_str()).unwrap_or("-");
            let result = row.get("result").and_then(|r| r.as_str()).unwrap_or("-");
            match result {
                "pass" | "xfail" => passed += 1,
                _ => bad.push((name.to_string(), result.to_string())),
            }
        }
        let errors = def
            .get("errors")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        // `unsupported-*` refusals are documented v1 tier
        // unavailability (where-filters, invariant properties) — shown
        // but not failing; everything else demands spec attention.
        let blocking = errors.iter().any(|e| {
            !e.get("code")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.starts_with("unsupported-"))
        });
        println!(
            "{path}: {passed}/{} {}",
            rows.len(),
            if bad.is_empty() && !blocking {
                "ok"
            } else {
                failing = true;
                "FAILING"
            }
        );
        for (name, result) in &bad {
            println!("  {name}: {result}");
        }
        if let Some(details) = def.get("details").and_then(|d| d.as_array()) {
            for detail in details {
                println!(
                    "  {}: {}",
                    detail.get("row").and_then(|r| r.as_str()).unwrap_or("-"),
                    detail
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("-")
                );
            }
        }
        for error in &errors {
            println!(
                "  {}: {}",
                error.get("code").and_then(|c| c.as_str()).unwrap_or("-"),
                error.get("message").and_then(|m| m.as_str()).unwrap_or("-")
            );
        }
        for note in def
            .get("notes")
            .and_then(|n| n.as_array())
            .into_iter()
            .flatten()
        {
            println!("  note: {}", note.as_str().unwrap_or("-"));
        }
    }
    if failing {
        1
    } else {
        EXIT_OK
    }
}

/// The packed manifest, then where the bundle landed (`--json` for
/// the raw result).
fn render_context(result: &serde_json::Value) {
    match result.get("rendered").and_then(|r| r.as_str()) {
        Some(text) => print!("{text}"),
        None => println!("{result}"),
    }
    if let Some(out) = result.get("out").and_then(|o| o.as_str()) {
        println!("written to {out}");
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
