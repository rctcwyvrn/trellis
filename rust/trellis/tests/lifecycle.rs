//! Step-2 integration tests: everything drives the built `trellis`
//! binary (the plan-02 precedent — downstream consumers see the CLI,
//! so the tests do too). Each test gets its own fake
//! `XDG_RUNTIME_DIR` so daemons never cross-talk, and a guard stops
//! the daemon on drop.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct TestRoot {
    root: PathBuf,
    runtime: PathBuf,
}

impl TestRoot {
    fn new(soil_toml: &str) -> TestRoot {
        let unique = format!(
            "trellis-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        );
        let base = std::env::temp_dir().join(unique);
        let root = base.join("root");
        let runtime = base.join("runtime");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(root.join("soil.toml"), soil_toml).unwrap();
        TestRoot { root, runtime }
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_in(&self.root, args)
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_trellis"))
            .args(args)
            .current_dir(cwd)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .output()
            .expect("binary runs")
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = self.run(&["daemon", "stop"]);
        if let Some(base) = self.root.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

fn good_soil_toml() -> String {
    format!(
        "[toolchain]\ntrellis = \"{}\"\nsoil0_cli = 1\n",
        env!("CARGO_PKG_VERSION")
    )
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn version_flag() {
    let env = TestRoot::new(&good_soil_toml());
    let out = env.run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("\"trellis\""));
    assert!(stdout(&out).contains("\"daemon_contract\":1"));
}

#[test]
fn unknown_command_is_usage() {
    let env = TestRoot::new(&good_soil_toml());
    let out = env.run(&["frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("\"usage\""));
}

#[test]
fn no_root_is_a_diagnostic() {
    let env = TestRoot::new(&good_soil_toml());
    let outside = env.root.parent().unwrap().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    let out = env.run_in(&outside, &["status"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("config-no-root"));
}

#[test]
fn autospawn_status_stop_roundtrip() {
    let env = TestRoot::new(&good_soil_toml());

    // No daemon yet.
    let out = env.run(&["daemon", "status"]);
    assert!(stdout(&out).contains("\"running\":false"));

    // A forwarded command auto-spawns the daemon; `status` is real
    // since step 6 and renders the (empty) table.
    let out = env.run(&["status"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("path"), "{}", stdout(&out));

    let out = env.run(&["daemon", "status"]);
    assert!(stdout(&out).contains("\"running\":true"));

    let out = env.run(&["daemon", "stop"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("\"stopped\":true"));

    let out = env.run(&["daemon", "status"]);
    assert!(stdout(&out).contains("\"running\":false"));
}

#[test]
fn stale_socket_recovers() {
    let env = TestRoot::new(&good_soil_toml());

    // Spawn a daemon, then kill it dead so its socket file lingers.
    let out = env.run(&["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let socket_dir = env.runtime.join("trellis");
    let sockets: Vec<_> = std::fs::read_dir(&socket_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(sockets.len(), 1);
    env.run(&["daemon", "stop"]);
    // Simulate a crash's leftovers: a dead socket file at the path.
    std::fs::write(&sockets[0], b"").unwrap();

    // The next client removes the stale file and respawns.
    let out = env.run(&["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = env.run(&["daemon", "status"]);
    assert!(stdout(&out).contains("\"running\":true"));
}

#[test]
fn concurrent_clients() {
    let env = TestRoot::new(&good_soil_toml());
    env.run(&["check"]); // spawn the daemon
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| env.run(&["daemon", "status"])))
            .collect();
        for handle in handles {
            let out = handle.join().unwrap();
            assert!(stdout(&out).contains("\"running\":true"));
        }
    });
}

#[test]
fn pin_mismatch_refuses_mutating_only() {
    let bad = format!(
        "[toolchain]\ntrellis = \"{}\"\nsoil0_cli = 99\n",
        env!("CARGO_PKG_VERSION")
    );
    let env = TestRoot::new(&bad);

    // Mutating: refused with the structured diagnostic.
    let out = env.run(&["lower", "median"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("toolchain-mismatch"));

    // Read-only: allowed past the pin (`check` runs and the empty
    // root checks clean).
    let out = env.run(&["check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn config_errors_surface_through_the_gate() {
    let env = TestRoot::new("[toolchain]\ntrellis = \"0.0.0\"\nsoil0_cli = 1\n\n[banana]\n");
    let out = env.run(&["lower", "x"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("config-unknown-key"));

    let env = TestRoot::new(&format!("{}\n[deps]\n", good_soil_toml()));
    let out = env.run(&["refresh"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("config-reserved-section"));
}
