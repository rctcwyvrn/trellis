//! The cram runner (impl plan 03 step 8, tr-grammar §3.5): each block
//! runs in a fresh temp dir with its `with file` fixtures, as one
//! sequential shell session (`cd`/env persist across steps), matching
//! each step's combined output literally and its `[n]` exit code. The
//! `trellis` binary is prepended to `PATH` and `TRELLIS_ROOT` is
//! exported so `trellis call` finds the root from outside the tree.
//! Cram is the real mode: never reachable from the lowering sandbox
//! (the MCP `run_tests` tool has no path here).

use std::path::{Path, PathBuf};

use crate::diag::Diag;
use crate::trfile::tests_blk::CramBlock;

pub struct StepResult {
    pub passed: bool,
    /// The literal mismatch, for the report (never the lock).
    pub detail: Option<String>,
}

pub struct CramOutcome {
    /// One entry per `$` step, in transcript order.
    pub steps: Vec<StepResult>,
    pub errors: Vec<Diag>,
}

fn io_err(message: impl Into<String>) -> Diag {
    Diag::new("io", message)
}

/// The `trellis` binary for the transcript's `PATH`: an explicit
/// `TRELLIS_BIN` override wins (integration tests point it at the
/// cargo-built binary), else the current executable when it *is*
/// trellis (the daemon and CLI cases).
fn trellis_bin() -> Option<PathBuf> {
    if let Some(bin) = std::env::var_os("TRELLIS_BIN") {
        return Some(PathBuf::from(bin));
    }
    std::env::current_exe()
        .ok()
        .filter(|p| p.file_stem().is_some_and(|s| s == "trellis"))
}

pub fn run_block(root: &Path, block: &CramBlock) -> CramOutcome {
    let mut outcome = CramOutcome {
        steps: Vec::new(),
        errors: Vec::new(),
    };
    let fail_all = |outcome: &mut CramOutcome, diag: Diag, n: usize| {
        outcome.errors.push(crate::registry::enrich(diag));
        for _ in 0..n {
            outcome.steps.push(StepResult {
                passed: false,
                detail: None,
            });
        }
    };

    // Fixture paths stay inside the temp dir — an absolute or
    // `..`-escaping path is a spec error.
    for fixture in &block.files {
        let p = Path::new(&fixture.path);
        if p.is_absolute()
            || p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            fail_all(
                &mut outcome,
                Diag::new(
                    "tr-cram-syntax",
                    format!(
                        "fixture path `{}` must stay inside the transcript's temp dir \
                         (tr-grammar §3.5)",
                        fixture.path
                    ),
                ),
                block.steps.len(),
            );
            return outcome;
        }
    }

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "trellis-cram-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let run = (|| -> Result<Vec<StepResult>, Diag> {
        std::fs::create_dir_all(&dir).map_err(|e| io_err(format!("cram temp dir: {e}")))?;
        for fixture in &block.files {
            let path = dir.join(&fixture.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| io_err(format!("fixture `{}`: {e}", fixture.path)))?;
            }
            std::fs::write(&path, &fixture.contents)
                .map_err(|e| io_err(format!("fixture `{}`: {e}", fixture.path)))?;
        }

        // One script, one shell: `exec 2>&1` merges the streams for
        // the whole session, and a salt line after each step carries
        // its exit code without disturbing shell state.
        let salt = format!("__trellis_cram_{}__", std::process::id());
        let mut script = String::from("exec 2>&1\n");
        for step in &block.steps {
            script.push_str(&step.command);
            script.push('\n');
            script.push_str(&format!("printf '%s %s\\n' {salt} $?\n"));
        }
        let script_path = dir.join(".cram.sh");
        std::fs::write(&script_path, script).map_err(|e| io_err(format!("cram script: {e}")))?;

        let mut command = std::process::Command::new("sh");
        command
            .arg(&script_path)
            .current_dir(&dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .env("TRELLIS_ROOT", root);
        if let Some(bin) = trellis_bin().as_ref().and_then(|b| b.parent()) {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![bin.to_path_buf()];
            paths.extend(std::env::split_paths(&path));
            command.env(
                "PATH",
                std::env::join_paths(paths).map_err(|e| io_err(format!("PATH: {e}")))?,
            );
        }
        let out = command
            .output()
            .map_err(|e| io_err(format!("cram shell: {e}")))?;
        let stdout = String::from_utf8_lossy(&out.stdout);

        // Split the combined stream at the salt lines.
        let mut segments: Vec<(Vec<&str>, i64)> = Vec::new();
        let mut current: Vec<&str> = Vec::new();
        for line in stdout.lines() {
            match line.strip_prefix(&salt).and_then(|r| r.strip_prefix(' ')) {
                Some(code) => {
                    segments.push((std::mem::take(&mut current), code.parse().unwrap_or(-1)));
                }
                None => current.push(line),
            }
        }

        let mut steps = Vec::new();
        for (i, step) in block.steps.iter().enumerate() {
            let Some((lines, exit)) = segments.get(i) else {
                steps.push(StepResult {
                    passed: false,
                    detail: Some("the shell session ended before this step".into()),
                });
                continue;
            };
            let expected: Vec<&str> = step.output.iter().map(String::as_str).collect();
            let mut detail = None;
            if *lines != expected {
                detail = Some(format!(
                    "output mismatch: expected {:?}, got {:?}",
                    step.output, lines
                ));
            } else if *exit != step.exit {
                detail = Some(format!("exit {exit}, expected {}", step.exit));
            }
            steps.push(StepResult {
                passed: detail.is_none(),
                detail,
            });
        }
        Ok(steps)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    match run {
        Ok(steps) => outcome.steps = steps,
        Err(diag) => fail_all(&mut outcome, diag, block.steps.len()),
    }
    outcome
}
