//! Subcommand dispatch, contract §1: seven commands, `--version`,
//! stdout = one compact JSON document, stderr = one diagnostics
//! document, exit codes 0 (success) / 1 (spec violation) / 2 (usage or
//! environment). Argument parsing is hand-rolled (impl plan 02 §8.11).

use crate::diag::{Code, Diagnostic, Report};

/// This document's contract version (`docs/contracts/soil0-cli.md`).
pub const CLI_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Lex {
        file: String,
    },
    Parse {
        file: String,
    },
    Rename {
        manifest: String,
    },
    Infer {
        manifest: String,
        dump_ast: bool,
    },
    Check {
        manifest: String,
    },
    Run {
        manifest: String,
        entry: String,
        args_json: String,
    },
    Test {
        bundle: String,
    },
    Print {
        file: String,
    },
}

const USAGE: &str =
    "usage: soil0 <lex|parse|rename|infer|check|run|test|print> … | soil0 --version";

/// Parses argv (program name excluded). `Ok(None)` means `--version`.
pub fn parse_args(args: &[String]) -> Result<Option<Command>, Report> {
    let usage = |message: String| Report::one(Diagnostic::bare(Code::Usage, message));
    let Some(first) = args.first() else {
        return Err(usage(USAGE.to_string()));
    };
    if first == "--version" {
        return match args.len() {
            1 => Ok(None),
            _ => Err(usage("--version takes no further arguments".to_string())),
        };
    }

    // Split the remainder into positionals and flags.
    let mut positional: Vec<&String> = Vec::new();
    let mut dump_ast = false;
    let mut entry: Option<String> = None;
    let mut args_json: Option<String> = None;
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--dump-ast" => dump_ast = true,
            "--entry" => match rest.next() {
                Some(v) => entry = Some(v.clone()),
                None => return Err(usage("--entry requires a value".to_string())),
            },
            "--args" => match rest.next() {
                Some(v) => args_json = Some(v.clone()),
                None => return Err(usage("--args requires a value".to_string())),
            },
            s if s.starts_with("--") => {
                return Err(usage(format!("unknown flag `{s}`")));
            }
            _ => positional.push(a),
        }
    }

    let one_positional = |what: &str| -> Result<String, Report> {
        match positional.as_slice() {
            [p] => Ok((*p).clone()),
            _ => Err(usage(format!("{first} takes exactly one {what}"))),
        }
    };
    let no_stray_flags = |allowed: &str| -> Result<(), Report> {
        if dump_ast && allowed != "infer" {
            return Err(usage(format!(
                "--dump-ast is only valid for infer, not {first}"
            )));
        }
        if (entry.is_some() || args_json.is_some()) && allowed != "run" {
            return Err(usage(format!(
                "--entry/--args are only valid for run, not {first}"
            )));
        }
        Ok(())
    };

    match first.as_str() {
        "lex" => {
            no_stray_flags("lex")?;
            Ok(Some(Command::Lex {
                file: one_positional("<file.soil>")?,
            }))
        }
        "parse" => {
            no_stray_flags("parse")?;
            Ok(Some(Command::Parse {
                file: one_positional("<file.soil>")?,
            }))
        }
        "rename" => {
            no_stray_flags("rename")?;
            Ok(Some(Command::Rename {
                manifest: one_positional("<program.json>")?,
            }))
        }
        "infer" => {
            if entry.is_some() || args_json.is_some() {
                return Err(usage(
                    "--entry/--args are only valid for run, not infer".to_string(),
                ));
            }
            Ok(Some(Command::Infer {
                manifest: one_positional("<program.json>")?,
                dump_ast,
            }))
        }
        "check" => {
            no_stray_flags("check")?;
            Ok(Some(Command::Check {
                manifest: one_positional("<program.json>")?,
            }))
        }
        "run" => {
            if dump_ast {
                return Err(usage(
                    "--dump-ast is only valid for infer, not run".to_string(),
                ));
            }
            let manifest = one_positional("<program.json>")?;
            let entry = entry.ok_or_else(|| usage("run requires --entry <name>".to_string()))?;
            let args_json =
                args_json.ok_or_else(|| usage("run requires --args <json-array>".to_string()))?;
            Ok(Some(Command::Run {
                manifest,
                entry,
                args_json,
            }))
        }
        "test" => {
            no_stray_flags("test")?;
            Ok(Some(Command::Test {
                bundle: one_positional("<bundle.json>")?,
            }))
        }
        "print" => {
            no_stray_flags("print")?;
            Ok(Some(Command::Print {
                file: one_positional("<file.soil>")?,
            }))
        }
        other => Err(usage(format!("unknown command `{other}`; {USAGE}"))),
    }
}

/// Reads a source file; non-UTF-8 is `malformed-input`, everything else
/// `io` (both class 2).
fn read_source(path: &str) -> Result<String, Report> {
    std::fs::read_to_string(path).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::InvalidData {
            Code::MalformedInput
        } else {
            Code::Io
        };
        Report::one(Diagnostic::bare(code, format!("cannot read `{path}`: {e}")))
    })
}

/// Executes a parsed command, returning the stdout document and exit
/// code (nonzero only for `test` with failing cases, contract §10).
pub fn execute(command: &Command) -> Result<(String, i32), Report> {
    match command {
        Command::Lex { file } => {
            let src = read_source(file)?;
            let tokens = crate::lexer::lex(&src).map_err(|mut d| {
                d.file = crate::diag::Opt(Some(file.clone()));
                Report::one(d)
            })?;
            #[derive(serde::Serialize)]
            struct LexOutput {
                tokens: Vec<crate::span::Spanned<crate::token::Token>>,
            }
            Ok((
                serde_json::to_string(&LexOutput { tokens })
                    .expect("token serialization cannot fail"),
                0,
            ))
        }
        Command::Parse { file } => {
            let src = read_source(file)?;
            let ast = crate::parser::parse_file(&src, file).map_err(|mut d| {
                d.file = crate::diag::Opt(Some(file.clone()));
                Report::one(d)
            })?;
            Ok((
                serde_json::to_string(&ast).expect("AST serialization cannot fail"),
                0,
            ))
        }
        Command::Rename { manifest } => {
            let prog = crate::manifest::load_program(manifest).map_err(Report::one)?;
            let out = crate::rename::rename_program(&prog).map_err(Report::one)?;
            Ok((
                serde_json::to_string(&out).expect("rename output serialization cannot fail"),
                0,
            ))
        }
        Command::Infer { manifest, dump_ast } => {
            let prog = crate::manifest::load_program(manifest).map_err(Report::one)?;
            crate::rename::rename_program(&prog).map_err(Report::one)?;
            let out = crate::infer::infer_program(&prog).map_err(Report::one)?;
            if *dump_ast {
                // Non-contractual debug dump (contract §12).
                return Ok((crate::infer::dump(&prog), 0));
            }
            Ok((
                serde_json::to_string(&out).expect("infer output serialization cannot fail"),
                0,
            ))
        }
        Command::Check { manifest } => {
            let prog = crate::manifest::load_program(manifest).map_err(Report::one)?;
            crate::rename::rename_program(&prog).map_err(Report::one)?;
            let out = crate::exhaust::check_program(&prog).map_err(Report::one)?;
            Ok((
                serde_json::to_string(&out).expect("check output serialization cannot fail"),
                0,
            ))
        }
        Command::Run {
            manifest,
            entry,
            args_json,
        } => {
            let prog = crate::manifest::load_program(manifest).map_err(Report::one)?;
            let doc = crate::interp::cmd_run(prog, entry, args_json).map_err(Report::one)?;
            Ok((doc, 0))
        }
        Command::Test { bundle } => crate::interp::cmd_test(bundle).map_err(Report::one),
        Command::Print { file } => {
            let src = read_source(file)?;
            // Canonical Soil text — the documented exception to the
            // JSON-stdout rule (contract §8.9).
            let doc = crate::print::canonicalize(&src, file).map_err(|mut d| {
                d.file = crate::diag::Opt(Some(file.clone()));
                Report::one(d)
            })?;
            Ok((doc.trim_end().to_string(), 0))
        }
    }
}

/// The binary's whole behavior; returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let outcome = match parse_args(args) {
        Ok(None) => Ok((
            format!(
                "{{\"soil0_cli\":{CLI_VERSION},\"soil0\":\"{}\"}}",
                env!("CARGO_PKG_VERSION")
            ),
            0,
        )),
        Ok(Some(command)) => execute(&command),
        Err(report) => Err(report),
    };
    match outcome {
        Ok((doc, code)) => {
            println!("{doc}");
            code
        }
        Err(report) => {
            eprintln!("{}", report.render());
            report.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn version_parses() {
        assert_eq!(parse_args(&args(&["--version"])).unwrap(), None);
    }

    #[test]
    fn commands_parse() {
        assert_eq!(
            parse_args(&args(&["lex", "a.soil"])).unwrap(),
            Some(Command::Lex {
                file: "a.soil".into()
            })
        );
        assert_eq!(
            parse_args(&args(&["infer", "p.json", "--dump-ast"])).unwrap(),
            Some(Command::Infer {
                manifest: "p.json".into(),
                dump_ast: true
            })
        );
        assert_eq!(
            parse_args(&args(&[
                "run", "p.json", "--entry", "median", "--args", "[[1.0]]"
            ]))
            .unwrap(),
            Some(Command::Run {
                manifest: "p.json".into(),
                entry: "median".into(),
                args_json: "[[1.0]]".into()
            })
        );
    }

    #[test]
    fn usage_errors() {
        for bad in [
            vec!["frobnicate"],
            vec![],
            vec!["lex"],
            vec!["lex", "a.soil", "b.soil"],
            vec!["run", "p.json", "--entry", "median"],
            vec!["run", "p.json", "--args", "[]"],
            vec!["lex", "a.soil", "--dump-ast"],
            vec!["check", "p.json", "--entry", "x"],
            vec!["infer", "p.json", "--unknown"],
            vec!["--version", "extra"],
        ] {
            let report = parse_args(&args(&bad)).unwrap_err();
            assert_eq!(report.errors[0].code, Code::Usage, "case: {bad:?}");
            assert_eq!(report.exit_code(), 2, "case: {bad:?}");
        }
    }
}
