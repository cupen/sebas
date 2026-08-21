//! Integration test: SIGTERM must cleanly shut down a running `sebas`
//! daemon, reap its `fake-claude` ACP child, and persist the session
//! state file. Gated by `#[ignore]` so plain `cargo test --workspace`
//! skips it; opt in with `-- --ignored`.
//!
//! Two design notes that explain why this test is structured unusually:
//!
//! 1. **No live Feishu.** The test config uses empty `app_id`/`app_secret`
//!    plus an empty `owner_id` so sebas skips owner filtering. The HTTP
//!    token fetch and the WebSocket handshake will both fail. To avoid
//!    `sebas` exiting early (it would otherwise try to talk to
//!    `https://open.feishu.cn/...` and exit non-zero before reaching the
//!    select loop), we set two test affordances:
//!
//!      * `SEBAS_TEST_FAKE_TOKEN=1` substitutes a stub token, so the
//!        startup HTTP call to Feishu auth is skipped.
//!      * `SEBAS_TEST_SPAWN_SESSION=1` mints one fake-claude session at
//!        startup, so a child is alive as a direct descendant of the
//!        sebas pid by the time we send SIGTERM.
//!
//!    Both env vars are read in `src/run.rs`; production callers never
//!    set them, so the test path is dormant outside this test.
//!
//! 2. **Config file location.** The daemon is started via the explicit
//!    `run` subcommand, whose `--config` defaults to `./config.toml`.
//!    This test lays the config at that default path and spawns the
//!    daemon with `current_dir` set to the same temp dir, giving the
//!    test full control over the file contents without juggling paths.
//!
//! 3. **Child must be running long enough to be our child.** Spawning
//!    the ACP subprocess inside `acp-claude::SessionManager` takes
//!    ~50-200 ms. We sleep 1.2 s after `spawn()` returns so the child
//!    is registered under our pid before we send SIGTERM. Sleep budget
//!    is bounded (≤ 2 s) so the test stays cheap.
//!
//! The test is skipped (not failed) when `target/debug/fake-claude` is
//! missing, since building that binary is the responsibility of the
//! workspace's regular `cargo build --workspace`.

#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
mod support;
#[cfg(unix)]
use support::TestDir;

/// Locate the workspace `target/debug` directory by walking up from
/// `CARGO_MANIFEST_DIR` (the `sebas` crate root). Assumes the standard
/// cargo workspace layout (`target/debug` at the workspace root).
#[cfg(unix)]
fn workspace_target_debug() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set during cargo test");
    PathBuf::from(manifest_dir).join("target").join("debug")
}

/// Write a minimal `config.toml` into `dir` for the spawned sebas to
/// pick up via its default `./config.toml` lookup. The state file path
/// and the agent sessions directory are baked into the config so the
/// daemon writes to the same on-disk locations the test inspects.
/// `sessions_dir` lives under `target/tests/` (NOT `/tmp`) so a stray
/// `cargo clean` removes it along with build artefacts.
#[cfg(unix)]
fn write_config_in(dir: &Path, state_path: &Path, fake_claude_path: &str, sessions_dir: &Path) {
    let path = dir.join("config.toml");
    let body = format!(
        r#"[feishu]
app_id = "fake-app-id"
app_secret = "fake-app-secret"
owner_id = ""

[acp.claude]
path = {fake_claude_path:?}
sessions_dir = {sessions_dir:?}
idle_kill_secs = 60

[router]
state_file = {state_path:?}
channel_buffer = 16
max_concurrent_sessions = 4

[log]
level = "info"
"#,
        fake_claude_path = fake_claude_path,
        state_path = state_path.display().to_string(),
        sessions_dir = sessions_dir.display().to_string(),
    );
    std::fs::write(&path, body).expect("write config.toml");
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "opt-in integration test; run with: cargo test --workspace -- sigterm_cleanup -- --ignored --nocapture"]
async fn sigterm_cleans_up_child_and_persists_state() {
    // ---- 1. Locate binaries in the workspace target dir. -----------------
    let target_dir = workspace_target_debug();
    let sebas_bin = target_dir.join("sebas");
    let fake_claude_bin = target_dir.join("fake-claude");
    if !sebas_bin.exists() || !fake_claude_bin.exists() {
        eprintln!(
            "skipping: required binaries missing ({}, {})",
            sebas_bin.display(),
            fake_claude_bin.display()
        );
        return;
    }

    // ---- 2. Lay down a temp working directory containing both the config
    //         and the state file. Pre-populate the state file with a known
    //         mapping so we can verify the daemon re-serialises it on exit
    //         (rather than trampling the restore path). All scratch lives
    //         under `target/tests/sebas/sigterm_cleanup/` so a stray `cargo
    //         clean` removes it; nothing is written to /tmp or $HOME.
    let work_dir = TestDir::new("sigterm_cleanup", "work");
    let sessions_dir = TestDir::new("sigterm_cleanup", "sessions");
    let state_path = work_dir.path().join("sessions.json");
    let pre_mapping = serde_json::json!({
        "test-sigterm-pre": {
            "session_id": "sess-pre-populated-1",
            "last_active_unix": 1700000000i64,
        }
    });
    std::fs::write(
        &state_path,
        serde_json::to_string(&pre_mapping).expect("serialise pre-populated state"),
    )
    .expect("write state file");
    write_config_in(
        work_dir.path(),
        &state_path,
        fake_claude_bin.to_str().unwrap(),
        sessions_dir.path(),
    );

    // ---- 3. Spawn sebas with cwd=work_dir so it picks up our config. -----
    let mut child = tokio::process::Command::new(&sebas_bin)
        .arg("run")
        .current_dir(work_dir.path())
        .env("SEBAS_TEST_FAKE_TOKEN", "1")
        .env("SEBAS_TEST_SPAWN_SESSION", "1")
        .env("RUST_LOG", "info,sebas=debug,acp_claude=debug")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn sebas");
    let pid = child.id().expect("child has a pid");

    // ---- 4. Wait long enough for the ACP child to be spawned and ---------
    //         registered under our pid. Bounded ≤ 2 s per brief.
    tokio::time::sleep(Duration::from_millis(1200)).await;

    // Sanity: at least one fake-claude child must be alive under sebas
    // before we send SIGTERM, or this test would degenerate into a
    // "vacuous pgrep" assertion.
    let pre_pgrep = tokio::process::Command::new("pgrep")
        .args(["-P", &pid.to_string(), "fake-claude"])
        .output()
        .await
        .expect("pgrep pre");
    let pre_children = String::from_utf8_lossy(&pre_pgrep.stdout).into_owned();
    assert!(
        !pre_children.trim().is_empty(),
        "expected fake-claude child of sebas pid {} before SIGTERM, got none",
        pid
    );
    eprintln!(
        "pre-SIGTERM fake-claude pids under {}: {:?}",
        pid, pre_children
    );

    // ---- 5. Send SIGTERM and wait for clean exit. ------------------------
    // Use libc::kill directly: `libc` is already a runtime dep of sebas,
    // and we don't need the full `nix` crate for a single signal call.
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    assert_eq!(
        rc,
        0,
        "kill(SIGTERM) failed: errno={}",
        std::io::Error::last_os_error()
    );

    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .expect("sebas should exit within 10 s of SIGTERM")
        .expect("child.wait");
    assert!(
        status.success(),
        "sebas did not exit cleanly on SIGTERM: {:?}",
        status
    );

    // ---- 6. Verify the fake-claude child is gone. ------------------------
    // Give the kernel a moment to deliver the SIGKILL that `kill_all`
    // cascades down via `cancel_tx` → `CancelNotification` → SDK drops
    // the child process.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let post_pgrep = tokio::process::Command::new("pgrep")
        .args(["-P", &pid.to_string(), "fake-claude"])
        .output()
        .await
        .expect("pgrep post");
    let post_children = String::from_utf8_lossy(&post_pgrep.stdout).into_owned();
    eprintln!(
        "post-SIGTERM fake-claude pids under {}: {:?}",
        pid, post_children
    );
    assert!(
        post_children.trim().is_empty(),
        "fake-claude still alive after sebas exit: {:?}",
        post_children
    );

    // ---- 7. Verify the state file was persisted. ------------------------
    // The pre-populated entry must still be there (restore + dump round-
    // trip); additionally, the spawned test session should have minted
    // a `session_id` mapping.
    let state_text = std::fs::read_to_string(&state_path).expect("state file readable");
    eprintln!("post-SIGTERM state file contents:\n{state_text}");
    let json: serde_json::Value = serde_json::from_str(&state_text).expect("state file is JSON");
    let obj = json.as_object().expect("state file root is an object");
    assert!(
        obj.values().any(|v| {
            v.get("session_id")
                .and_then(|s| s.as_str())
                .is_some_and(|s| s == "sess-pre-populated-1")
        }),
        "pre-populated mapping lost across restart; state file: {state_text}"
    );
    assert!(
        obj.values().any(|v| v.get("session_id").is_some()),
        "no session_id mapping found in state file: {state_text}"
    );
}
