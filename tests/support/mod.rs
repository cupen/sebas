//! Shared test utilities for the `sebas` integration tests.
//!
//! `TestDir` is the canonical scratch-directory primitive: each instance
//! gets a fresh, unique subdirectory under `target/tests/<crate>/<test>/`
//! and removes it on drop. Tests that need a stable path they can hand to
//! a child process should keep the `TestDir` alive for the duration of
//! the test (or call `keep()` if they need it to outlive the test).
//!
//! Why `target/tests/` instead of `/tmp` or `$HOME`:
//!
//! - **Hermetic.** Runs don't share `/tmp` with the rest of the host, so
//!   parallel CI agents and concurrent local runs can't collide.
//! - **Owned by cargo.** Lives under the workspace `target/`, so a plain
//!   `cargo clean` (which the user just ran) wipes every stale scratch
//!   dir along with the build artefacts. No `~/.local/state` pollution.
//! - **Predictable layout.** `target/tests/<crate>/<test_name>/<unique>/`
//!   means a failing test's leftover state is trivial to find.
//!
//! Layout is computed from `CARGO_MANIFEST_DIR`, which cargo sets for
//! every test binary at compile time.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// RAII scratch directory rooted at `target/tests/<crate>/<test>/<unique>`.
///
/// `Drop` removes the directory and everything under it. Call `keep()`
/// to leak the directory (useful when the test deliberately crashes the
/// daemon and you want the leftover state inspectable afterwards — the
/// next `cargo clean` will still tidy up).
pub struct TestDir {
    path: PathBuf,
    keep: bool,
}

impl TestDir {
    /// Create a fresh scratch dir for `test_name`. `sub` lets one test
    /// claim multiple disjoint dirs (e.g. one for state, one for config).
    pub fn new(test_name: &str, sub: &str) -> Self {
        Self::with_crate(test_name, sub, env!("CARGO_PKG_NAME"))
    }

    /// Same as `new` but with the crate name spelled explicitly. Use
    /// this from `router/tests/support/mod.rs` (a different crate's
    /// `CARGO_PKG_NAME`).
    pub fn with_crate(test_name: &str, sub: &str, crate_name: &str) -> Self {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| {
            panic!("CARGO_MANIFEST_DIR unset — TestDir must be called from a cargo test binary")
        });
        let manifest = PathBuf::from(manifest_dir);
        // Workspace root is the parent of every member crate's manifest dir
        // (sebas's workspace layout: <root>/{router,feishu,...}/Cargo.toml).
        // The fallback `manifest.clone()` covers single-crate checkouts
        // where the test crate IS the workspace root.
        let workspace_root = manifest
            .parent()
            .filter(|p| p.join("Cargo.toml").exists())
            .map(|p| p.to_path_buf())
            .unwrap_or(manifest);
        let stamp = unique_stamp();
        let path = workspace_root
            .join("target")
            .join("tests")
            .join(crate_name)
            .join(test_name)
            .join(format!("{stamp}-{sub}"));
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("create scratch dir {}: {e}", path.display()));
        Self { path, keep: false }
    }

    /// Path to the scratch directory. Created; safe to write into.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Disable auto-cleanup on drop. Use when you want the test's
    /// leftovers to survive a crash for postmortem inspection.
    pub fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        // Best-effort: a parallel test might be holding a handle, or
        // permission might already be revoked (unlikely on target/, but
        // be defensive). Ignore failures — `cargo clean` is the
        // hammer-of-last-resort.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Combination of nanos-since-epoch + a process-local counter, so two
/// `TestDir::new` calls inside the same `#[tokio::test]` don't race on
/// the timestamp and end up sharing a path.
fn unique_stamp() -> u128 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    (t << 16) | (n & 0xFFFF)
}


// ---------------------------------------------------------------------------
// Process-level e2e sandbox (testsuite-process-e2e).
//
// Each `Sandbox` is a fully isolated throwaway instance: config file + every
// default-overriding env var live inside the sandbox dir, the webui binds a
// probed free port (never 9797), and nothing touches the operator's real
// `~/.sebas`. Mirrors the proven manual recipe in AGENTS.md, automated.
//
// Keep-on-failure: on a panicking test thread `Drop` sees
// `std::thread::panicking()` and leaves the dir (with core/webui logs) in
// place for postmortem; `cargo clean` is the hammer of last resort.
// ---------------------------------------------------------------------------

use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

pub struct SandboxDir {
    path: PathBuf,
    keep: AtomicBool,
}

impl SandboxDir {
    fn new(test_name: &str, sub: &str) -> Arc<Self> {
        // Short leaf (`<pid>-<n>-<sub>`): the channel socket path must fit
        // the unix `sun_path` budget (108 bytes incl. NUL) even from
        // long worktree checkouts — a nanos-stamp leaf overflows it and the
        // core bind fails with a misleading startup-failure. Uniqueness:
        // pid across processes, counter within one process.
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let leaf = format!("{}-{n}-{sub}", std::process::id());
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = manifest
            .join("target")
            .join("tests")
            .join("sebas")
            .join(test_name)
            .join(leaf);
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("create sandbox dir {}: {e}", path.display()));
        Arc::new(Self {
            path,
            keep: AtomicBool::new(false),
        })
    }
}

impl Drop for SandboxDir {
    fn drop(&mut self) {
        if self.keep.load(Ordering::Relaxed) || std::thread::panicking() {
            eprintln!(
                "[sandbox] kept for diagnosis (logs inside): {}",
                self.path.display()
            );
            return;
        }
        // Children may still be releasing file handles; retry a few times.
        for _ in 0..3 {
            if std::fs::remove_dir_all(&self.path).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

pub fn forward_slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub struct Sandbox {
    pub path: PathBuf,
    pub config_path: PathBuf,
    pub webui_port: u16,
    pub channel_path: PathBuf,
    pub state_file: PathBuf,
    pub core_log: PathBuf,
    pub webui_log: PathBuf,
    /// The one fake secret shared by core and (matching) webui processes.
    pub core_secret: String,
    /// Short XDG dir for watchdog runs (control socket; created on demand
    /// by [`Self::spawn_watchdog`], removed on drop).
    xdg_dir: Option<PathBuf>,
    /// Holds the drop guard (kept alive for the sandbox's whole life).
    _dir: Arc<SandboxDir>,
}

impl Sandbox {
    /// Fresh sandbox with a written config: every path inside the sandbox,
    /// webui on a probed free port, fake-claude from the workspace build.
    pub fn new(test_name: &str, sub: &str) -> Self {
        let dir = SandboxDir::new(test_name, sub);
        let path = dir.path.clone();
        let mkdir = |d: &Path| {
            std::fs::create_dir_all(d).unwrap_or_else(|e| panic!("mkdir {}: {e}", d.display()))
        };
        mkdir(&path.join("work"));
        mkdir(&path.join("claude-sessions"));
        mkdir(&path.join("downloads"));

        let webui_port = free_port();
        let config_path = path.join("config.toml");
        let channel_path = path.join("core-channel.sock");
        let state_file = path.join("sessions.json");
        let core_log = path.join("core.log");
        let webui_log = path.join("webui.log");
        let providers = path.join("providers.json");
        let usage = path.join("router-usage.jsonl");
        let fake_claude = forward_slash(Path::new(env!("CARGO_BIN_EXE_fake-claude")));

        // TOML basic strings reject bare backslashes — normalize to `/`
        // (Windows accepts forward slashes everywhere we touch files).
        let toml = format!(
            r#"[feishu]
enabled = false

[acp.agents.claude]
driver = "claude"
path = "{fake_claude}"
sessions_dir = "{}"
work_dir = "{}"

[dispatch]
state_file = "{}"

[media]
download_dir = "{}"

[watchdog.core]
channel_path = "{}"

[watchdog.webui]
enabled = true
host = "127.0.0.1"
port = {webui_port}

# router validate requires >=1 provider with a base_url; the debug `test`
# provider is injected only after parse. This dummy never dials anything
# in debug mode.
[provider.anthropic]
api_key = "sk-sandbox-dummy"

[router]
provider_overlay = "{}"
usage_file = "{}"
"#,
            forward_slash(&path.join("claude-sessions")),
            forward_slash(&path.join("work")),
            forward_slash(&state_file),
            forward_slash(&path.join("downloads")),
            forward_slash(&channel_path),
            forward_slash(&providers),
            forward_slash(&usage),
        );
        std::fs::write(&config_path, &toml)
            .unwrap_or_else(|e| panic!("write config {}: {e}", config_path.display()));

        Self {
            path,
            config_path,
            webui_port,
            channel_path,
            state_file,
            core_log,
            webui_log,
            core_secret: "sandbox-secret".into(),
            xdg_dir: None,
            _dir: dir,
        }
    }

    /// Env overrides every default that would otherwise fall back to the
    /// operator's real `~/.sebas` (AGENTS.md sandbox rule 1).
    fn envs(&self, secret: &str) -> Vec<(&'static str, String)> {
        vec![
            ("SEBAS_CORE_SECRET", secret.to_string()),
            ("SEBAS_STATE_DB", forward_slash(&self.path.join("sebas.db"))),
            ("SEBAS_STATE_FILE", forward_slash(&self.path.join("state.json"))),
            (
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
                forward_slash(&self.path.join("providers.json")),
            ),
            // Keep log files plain ASCII so assertions can match them.
            ("NO_COLOR", "1".to_string()),
        ]
    }

    /// Spawn an arbitrary subcommand with the sandbox env + extra env vars
    /// (fail-fast-on-startup-errors: startup-failure cases need `run`/`core`
    /// against a garbage config + SEBAS_STARTUP_ERROR_FILE). stdout+stderr are
    /// appended to `log` so the caller can assert on the stderr tail.
    pub fn spawn(
        &self,
        args: &[&str],
        secret: &str,
        extra: &[(&str, &str)],
        log: &Path,
    ) -> tokio::process::Child {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"));
        cmd.args(args)
            .envs(self.envs(secret))
            .envs(extra.iter().copied());
        self.spawn_with(cmd, log)
    }

    /// Same as [`Self::spawn`] but WITHOUT `SEBAS_CORE_SECRET` in the child
    /// env (harden-core-channel-deployment §5: no-secret assembly). The core
    /// auto-arms with a minted secret published to the secret file; clients
    /// discover it from the same `-c` config (env → file → empty-with-warn).
    pub fn spawn_bare(&self, args: &[&str], extra: &[(&str, &str)], log: &Path) -> tokio::process::Child {
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"));
        cmd.args(args)
            .envs(self.envs(""))
            .env_remove("SEBAS_CORE_SECRET")
            .envs(extra.iter().copied());
        self.spawn_with(cmd, log)
    }

    fn spawn_with(&self, mut cmd: tokio::process::Command, log: &Path) -> tokio::process::Child {
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap_or_else(|e| panic!("open log {}: {e}", log.display()));
        let log_err = log_file
            .try_clone()
            .unwrap_or_else(|e| panic!("clone log handle: {e}"));
        cmd.stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true)
            .spawn()
            .unwrap_or_else(|e| panic!("spawn sebas: {e}"))
    }

    /// Core: `sebas run -c <config> --router --debug` (detached: no --webui;
    /// the channel socket comes up because SEBAS_CORE_SECRET is set).
    pub fn spawn_core(&self) -> tokio::process::Child {
        self.spawn_core_extra(&[])
    }

    /// Same as `spawn_core` with extra env vars (test affordances like
    /// `SEBAS_TEST_SPAWN_SESSION=1`).
    pub fn spawn_core_extra(&self, extra: &[(&str, &str)]) -> tokio::process::Child {
        self.spawn(
            &[
                "core",
                "-c",
                &forward_slash(&self.config_path),
                "--router",
                "--debug",
            ],
            &self.core_secret,
            extra,
            &self.core_log,
        )
    }

    /// Core with the router but WITHOUT `--debug` (downstream auth enforced;
    /// no built-in test provider). For auth-rejection journeys.
    pub fn spawn_core_router_auth(&self) -> tokio::process::Child {
        self.spawn(
            &["core", "-c", &forward_slash(&self.config_path), "--router"],
            &self.core_secret,
            &[],
            &self.core_log,
        )
    }

    /// Standalone webui: `sebas webui -c <config>`; `secret` is what the
    /// webui presents to the core channel (pass a different one for
    /// wrong-secret cases).
    pub fn spawn_webui(&self, secret: &str) -> tokio::process::Child {
        self.spawn(
            &["webui", "-c", &forward_slash(&self.config_path)],
            secret,
            &[],
            &self.webui_log,
        )
    }

    /// No-secret assembly (harden-core-channel-deployment §5.1/§5.2): core
    /// and standalone webui started with NO `SEBAS_CORE_SECRET` in env. The
    /// core mints a fresh secret per boot and publishes it to the secret
    /// file; the webui discovers it from the same `-c` config.
    pub fn spawn_core_no_secret(&self) -> tokio::process::Child {
        self.spawn_bare(
            &[
                "core",
                "-c",
                &forward_slash(&self.config_path),
                "--router",
                "--debug",
            ],
            &[],
            &self.core_log,
        )
    }

    pub fn spawn_webui_no_secret(&self) -> tokio::process::Child {
        self.spawn_bare(
            &["webui", "-c", &forward_slash(&self.config_path)],
            &[],
            &self.webui_log,
        )
    }

    /// Resolved secret-file path for this sandbox's config (D1 default:
    /// `<config dir>/core.secret`; mirrors
    /// `sebas::core_channel::secret_file_path` so the test asserts on the
    /// same path the core publishes and the webui reads).
    pub fn secret_file_path(&self) -> PathBuf {
        sebas::core_channel::secret_file_path(None, &self.config_path)
    }

    /// Watchdog-supervised assembly (harden-core-channel-deployment §5.3):
    /// `sebas run -c <config>` supervises core + webui children (both from
    /// the sandbox config; the watchdog mints and injects its own channel
    /// secret). `HOME` is redirected into the sandbox (services persist
    /// file) and `XDG_RUNTIME_DIR` into a short tmp dir: the control socket
    /// default (`$XDG_RUNTIME_DIR/sebas/control.sock`) must neither collide
    /// with the operator's real one (`serve` removes a pre-existing file!)
    /// nor overflow the unix `sun_path` budget from a long checkout path.
    pub fn spawn_watchdog(&mut self) -> tokio::process::Child {
        let xdg = std::env::temp_dir().join(format!(
            "sebas-wd-{}-{}",
            std::process::id(),
            self.webui_port
        ));
        std::fs::create_dir_all(&xdg).expect("mkdir sandbox xdg dir");
        let home = forward_slash(&self.path);
        let xdg_s = forward_slash(&xdg);
        self.xdg_dir = Some(xdg);
        self.spawn(
            &["run", "-c", &forward_slash(&self.config_path)],
            &self.core_secret,
            &[("HOME", &home), ("XDG_RUNTIME_DIR", &xdg_s)],
            &self.core_log,
        )
    }

    /// SIGKILL the `run` watchdog, then any leftover process whose cmdline
    /// references this sandbox dir (linux-only; `kill_on_drop` reaps only
    /// `run` itself — supervised grandchildren would orphan). Scoped to our
    /// own sandbox path — the operator's instance and other sandboxes never
    /// match.
    #[cfg(target_os = "linux")]
    pub async fn reap_watchdog(&self, run: &mut tokio::process::Child) {
        let _ = run.kill().await;
        let _ = run.wait().await;
        let marker = self.path.to_string_lossy().into_owned();
        let own = std::process::id();
        let hint = self.path.clone();
        wait_for(
            "supervised tree to exit",
            Duration::from_secs(10),
            &hint,
            move || {
                let marker = marker.clone();
                Box::pin(async move {
                    let mut left = 0;
                    for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
                        let pid: u32 = match entry.file_name().to_string_lossy().parse() {
                            Ok(p) => p,
                            Err(_) => continue,
                        };
                        if pid == own {
                            continue;
                        }
                        let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                        if String::from_utf8_lossy(&cmd).contains(&marker) {
                            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
                            left += 1;
                        }
                    }
                    (left == 0).then_some(())
                })
            },
        )
        .await;
    }

    pub fn webui_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.webui_port)
    }

    /// Require a downstream token on the router proxy surface (`auth_token`
    /// inserted into the `[router]` section). Must be called before spawn.
    pub fn set_router_auth_token(&self, token: &str) {
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let patched = toml.replace(
            "[router]",
            &format!("[router]\nauth_token = \"{token}\""),
        );
        assert_ne!(toml, patched, "[router] section not found in config");
        std::fs::write(&self.config_path, patched).expect("write config");
    }

    /// Opt the sandbox config into watchdog-supervised core
    /// (`[watchdog.core] enabled = true`; the default is off — only the
    /// webui is supervised by default). Must be called before spawn.
    pub fn set_core_supervised(&self) {
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let patched = toml.replace("[watchdog.core]", "[watchdog.core]\nenabled = true");
        assert_ne!(toml, patched, "[watchdog.core] section not found in config");
        std::fs::write(&self.config_path, patched).expect("write config");
    }

    /// In-process webui form: `sebas run -c <config> --router --debug
    /// --webui --webui-port <p>`. Returns the child and the dashboard port.
    /// Extra envs may pin the router port (`SEBAS_ROUTER_LISTEN`) so the
    /// native agent can be pointed at it via `SEBAS_AGENT_ROUTER_URL`.
    pub fn spawn_core_inprocess_webui(&self, extra: &[(&str, &str)]) -> (tokio::process::Child, u16) {
        let dashboard_port = free_port();
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.core_log)
            .unwrap_or_else(|e| panic!("open log {}: {e}", self.core_log.display()));
        let log_err = log_file
            .try_clone()
            .unwrap_or_else(|e| panic!("clone log handle: {e}"));
        let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"))
            .args([
                "core",
                "-c",
                &forward_slash(&self.config_path),
                "--router",
                "--debug",
                "--webui",
                "--webui-port",
                &dashboard_port.to_string(),
            ])
            .envs(self.envs(&self.core_secret))
            .envs(extra.iter().copied())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true)
            .spawn()
            .unwrap_or_else(|e| panic!("spawn sebas in-process webui: {e}"));
        (child, dashboard_port)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if let Some(xdg) = self.xdg_dir.take() {
            let _ = std::fs::remove_dir_all(xdg);
        }
    }
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("probe free port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Poll until `probe` yields, with an explicit bound (spec: no unbounded
/// waits) and a diagnostic hint on timeout. The probe owns everything it
/// needs (capture clones, not borrows) so each poll is a fresh future.
pub async fn wait_for<T>(
    what: &str,
    timeout: Duration,
    log_hint: &Path,
    mut probe: impl FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<T>> + Send>>,
) -> T {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(v) = probe().await {
            return v;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timeout waiting for {what}; logs at {}", log_hint.display());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("http client")
}

async fn get_json(cli: &reqwest::Client, url: &str) -> Option<serde_json::Value> {
    cli.get(url).send().await.ok()?.json().await.ok()
}

/// GET /health on the sandbox webui; None while it is not serving yet.
pub async fn webui_healthy(cli: &reqwest::Client, sb: &Sandbox) -> Option<bool> {
    let body = cli
        .get(format!("{}/health", sb.webui_url()))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    Some(body.trim() == "ok")
}

/// `/api/summary` `reachability` object (None while webui is not serving).
pub async fn reachability(cli: &reqwest::Client, sb: &Sandbox) -> Option<serde_json::Value> {
    get_json(cli, &format!("{}/api/summary", sb.webui_url()))
        .await
        .and_then(|v| v.get("reachability").cloned())
}

/// Wait until the webui reports the core channel reachable.
pub async fn wait_reachable(cli: &reqwest::Client, sb: &Sandbox) {
    let cli = cli.clone();
    let url = format!("{}/api/summary", sb.webui_url());
    let hint = sb.path.clone();
    wait_for("core reachability ok", Duration::from_secs(30), &hint, move || {
        let cli = cli.clone();
        let url = url.clone();
        Box::pin(async move {
            get_json(&cli, &url)
                .await
                .and_then(|v| v.get("reachability").cloned())
                .filter(|r| r["ok"].as_bool() == Some(true))
        })
    })
    .await;
}

/// Wait until the webui reports unreachable with a non-empty cause
/// (wrong secret / dead core).
pub async fn wait_unreachable_with_cause(cli: &reqwest::Client, sb: &Sandbox) -> String {
    let cli = cli.clone();
    let url = format!("{}/api/summary", sb.webui_url());
    let hint = sb.path.clone();
    wait_for(
        "reachability flip to unreachable with cause",
        Duration::from_secs(20),
        &hint,
        move || {
            let cli = cli.clone();
            let url = url.clone();
            Box::pin(async move {
                let r = get_json(&cli, &url)
                    .await?
                    .get("reachability")
                    .cloned()?;
                if r["ok"].as_bool() == Some(false) {
                    r["cause"]
                        .as_str()
                        .filter(|c| !c.is_empty())
                        .map(String::from)
                } else {
                    None
                }
            })
        },
    )
    .await
}

/// Parse the router bind address from the core log
/// (`router started … addr=127.0.0.1:<port>`).
pub async fn wait_router_addr(sb: &Sandbox) -> String {
    let log_path = sb.core_log.clone();
    let hint = sb.path.clone();
    wait_for("router addr in core log", Duration::from_secs(15), &hint, move || {
        let log_path = log_path.clone();
        Box::pin(async move {
            let log = std::fs::read_to_string(&log_path).ok()?;
            for line in log.lines().rev() {
                let line = strip_ansi(line);
                if line.contains("router started")
                    && let Some(idx) = line.find("addr=")
                {
                    let addr = line[idx + 5..].split_whitespace().next()?;
                    if addr.parse::<std::net::SocketAddr>().is_ok() {
                        return Some(format!("http://{addr}"));
                    }
                }
            }
            None
        })
    })
    .await
}

/// Remove ANSI SGR escape sequences (`ESC [ … m`) a tracing subscriber may
/// have written into log files.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until the terminating 'm' of the SGR sequence.
            for f in chars.by_ref() {
                if f == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// POST a JSON body, return (status, body). Err on transport failure.
pub async fn post_json(
    cli: &reqwest::Client,
    url: &str,
    body: serde_json::Value,
) -> Result<(u16, serde_json::Value), String> {
    let resp = cli
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;
    let status = resp.status().as_u16();
    let json = resp
        .json()
        .await
        .map_err(|e| format!("body of {url}: {e}"))?;
    Ok((status, json))
}

/// Wait until the secret file exists with non-empty content; returns it.
pub async fn wait_secret_file(sb: &Sandbox) -> String {
    let path = sb.secret_file_path();
    let hint = sb.path.clone();
    wait_for(
        "secret file to be published",
        Duration::from_secs(15),
        &hint,
        move || {
            let path = path.clone();
            Box::pin(async move {
                let raw = std::fs::read_to_string(&path).ok()?;
                (!raw.trim().is_empty()).then(|| raw.trim().to_string())
            })
        },
    )
    .await
}

/// Find the supervised `sebas core` child of a `sebas run` watchdog pid by
/// scanning /proc cmdlines (linux-only; §5.3 supervision journey). Returns
/// the core child pid, or panics with a log hint after the bound.
#[cfg(target_os = "linux")]
pub async fn wait_supervised_core_pid(sb: &Sandbox, run_pid: u32) -> u32 {
    let hint = sb.path.clone();
    wait_for(
        "supervised core child pid",
        Duration::from_secs(15),
        &hint,
        move || {
            Box::pin(async move { supervised_core_pid(run_pid) })
        },
    )
    .await
}

/// One poll pass of [`wait_supervised_core_pid`]: a process whose cmdline
/// argv contains `core` and whose parent is `run_pid`.
#[cfg(target_os = "linux")]
fn supervised_core_pid(run_pid: u32) -> Option<u32> {
    let procs = std::fs::read_dir("/proc").ok()?;
    for entry in procs.flatten() {
        let pid: u32 = match entry.file_name().to_string_lossy().parse() {
            Ok(p) => p,
            // /proc also holds non-pid entries (sys, bus, self, ...) —
            // skip them (a `?` here would abort the whole scan).
            Err(_) => continue,
        };
        let cmdline = match std::fs::read(format!("/proc/{pid}/cmdline")) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let parts: Vec<&str> = cmdline
            .split(|b| *b == 0)
            .filter_map(|s| std::str::from_utf8(s).ok())
            .filter(|s| !s.is_empty())
            .collect();
        if pid == run_pid {
            continue;
        }
        if !parts.iter().any(|p| *p == "core") {
            continue;
        }
        // A process may exit between the cmdline and stat reads — skip it,
        // never abort the scan.
        let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // stat: `pid (comm) state ppid ...` — comm may contain spaces/parens,
        // so split after the last ')'.
        let after = match stat.rsplit_once(')') {
            Some((_, a)) => a,
            None => continue,
        };
        let ppid: u32 = match after.split_whitespace().nth(1).and_then(|s| s.parse().ok()) {
            Some(p) => p,
            None => continue,
        };
        if ppid == run_pid {
            return Some(pid);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_unique_paths_and_cleans_up() {
        let a = TestDir::new("self", "alpha");
        let b = TestDir::new("self", "beta");
        assert_ne!(
            a.path(),
            b.path(),
            "unique stamps must produce distinct paths"
        );
        assert!(a.path().exists());
        assert!(b.path().exists());
        let pa = a.path().to_path_buf();
        let pb = b.path().to_path_buf();
        drop(a);
        drop(b);
        assert!(!pa.exists(), "alpha dir must be removed on drop");
        assert!(!pb.exists(), "beta dir must be removed on drop");
    }

    #[test]
    fn keep_survives_drop() {
        let mut d = TestDir::new("self", "keep");
        d.keep();
        let p = d.path().to_path_buf();
        drop(d);
        assert!(p.exists(), "keep() must prevent cleanup");
        // Tidy up so the test itself is hermetic.
        std::fs::remove_dir_all(&p).unwrap();
    }

    #[test]
    fn path_lives_under_target_tests() {
        let d = TestDir::new("self", "layout");
        let path = d.path();
        assert!(
            path.components().any(|c| c.as_os_str() == "tests"),
            "TestDir path must include a `tests/` segment under target/: {}",
            path.display()
        );
        assert!(
            path.to_string_lossy().contains("target"),
            "TestDir path must be under `target/`: {}",
            path.display()
        );
    }
}
