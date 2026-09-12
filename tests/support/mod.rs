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
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Duration;

pub struct SandboxDir {
    path: PathBuf,
    keep: AtomicBool,
    /// PIDs spawned as their own process-group leaders (unix). Teardown
    /// killpg's each group so core-spawned routers and watchdog-respawned
    /// cores die with the test even though `kill_on_drop` only reaps the
    /// direct child (sebas-gc7 leak).
    group_leaders: Mutex<Vec<u32>>,
}

impl SandboxDir {
    fn new(test_name: &str, sub: &str) -> Arc<Self> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let stamp = unique_stamp();
        let path = manifest
            .join("target")
            .join("tests")
            .join("sebas")
            .join(test_name)
            .join(format!("{stamp}-{sub}"));
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("create sandbox dir {}: {e}", path.display()));
        Arc::new(Self {
            path,
            keep: AtomicBool::new(false),
            group_leaders: Mutex::new(Vec::new()),
        })
    }

    /// Record a spawned child as its own process-group leader.
    fn register_group_leader(&self, pid: u32) {
        self.group_leaders.lock().unwrap().push(pid);
    }

    /// SIGKILL every spawned process group. Grandchildren inherit the group,
    /// so this takes the whole tree regardless of what already exited
    /// (ESRCH on a fully-dead group is fine). Runs on the keep-path too:
    /// diagnosis needs the logs on disk, not the processes writing them.
    fn kill_process_groups(&self) {
        #[cfg(unix)]
        for pid in self.group_leaders.lock().unwrap().drain(..) {
            unsafe {
                libc::killpg(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        #[cfg(not(unix))]
        self.group_leaders.lock().unwrap().clear();
    }
}

impl Drop for SandboxDir {
    fn drop(&mut self) {
        self.kill_process_groups();
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

/// `sebas-node` 可执行文件：与 `sebas` 同一个 target 目录的兄弟文件。
///
/// `CARGO_BIN_EXE_<name>` 只对**同一个包**的 bin 有效，而 `sebas-node` 是另一个包；
/// 因此按 `sebas` 的位置推。缺文件时给出一条能照做的错误，而不是一句
/// "No such file"。
pub fn sebas_node_bin() -> PathBuf {
    let sebas = PathBuf::from(env!("CARGO_BIN_EXE_sebas"));
    let name = if cfg!(windows) { "sebas-node.exe" } else { "sebas-node" };
    let node = sebas.with_file_name(name);
    assert!(
        node.exists(),
        "找不到 {}：先构建它——cargo build -p sebas-node --bin sebas-node",
        node.display()
    );
    node
}

pub struct Sandbox {
    pub path: PathBuf,
    pub config_path: PathBuf,
    pub webui_port: u16,
    /// Absolute path of the channel socket for test-side fs assertions.
    /// The CONFIG carries the relative name `core-channel.sock` — every
    /// sandbox child runs with cwd = sandbox dir, so the socket path is
    /// independent of the checkout depth (sun_path caps unix socket paths
    /// at 108 bytes; deep `target/tests/…` trees overflow it).
    pub channel_path: PathBuf,
    pub state_file: PathBuf,
    pub core_log: PathBuf,
    pub webui_log: PathBuf,
    /// The one fake secret shared by core and (matching) webui processes.
    pub core_secret: String,
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
        // channel_path is RELATIVE: children run with cwd = sandbox dir, so
        // the unix socket never depends on the checkout depth (sun_path
        // caps paths at 108 bytes — deep target/tests trees overflow).
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
channel_path = "core-channel.sock"

[watchdog.webui]
enabled = true
host = "127.0.0.1"
port = {webui_port}
# auth 默认 true 且缺失凭据时会自动生成（写进真实 ~/.sebas）；API 断言
# 沙箱一律免登录，显式关闭（webui 鉴权旅程由 testsuite-webui 专测）。
auth = false

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
            _dir: dir,
        }
    }

    /// Env overrides every default that would otherwise fall back to the
    /// operator's real `~/.sebas` (AGENTS.md sandbox rule 1). `None` omits
    /// `SEBAS_CORE_SECRET` entirely — the no-secret assembly journeys
    /// (auto-arm + secret-file discovery) need a genuinely unset env.
    fn envs(&self, secret: Option<&str>) -> Vec<(&'static str, String)> {
        let mut envs = vec![
            // 项目注册表默认落在**操作者的** `~/.sebas/projects.json`
            // （`sebas-webui::projects`：`SEBAS_PROJECTS_PATH` → `SEBAS_HOME` →
            // `$HOME`）。沙箱必须改道：远端项目注册旅程会真的写这个文件，漏了
            // 这一条就会去动操作者的注册表（本套件在只读文件系统上跑时才发现，
            // 报的是"写临时文件失败"而不是"写错了地方"）。
            (
                "SEBAS_PROJECTS_PATH",
                forward_slash(&self.path.join("projects.json")),
            ),
            ("SEBAS_STATE_DB", forward_slash(&self.path.join("sebas.db"))),
            (
                "SEBAS_STATE_FILE",
                forward_slash(&self.path.join("state.json")),
            ),
            (
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
                forward_slash(&self.path.join("providers.json")),
            ),
            // archive.json falls back to SEBAS_HOME ($HOME/.sebas) — pin both
            // into the sandbox: without this the suites read AND rewrote the
            // operator's real archive (93-entry test pollution, 2026-09-12).
            ("SEBAS_HOME", forward_slash(&self.path)),
            (
                "SEBAS_ARCHIVE_PATH",
                forward_slash(&self.path.join("archive.json")),
            ),
            // Keep log files plain ASCII so assertions can match them.
            ("NO_COLOR", "1".to_string()),
        ];
        if let Some(secret) = secret {
            envs.push(("SEBAS_CORE_SECRET", secret.to_string()));
        }
        envs
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
        self.spawn_opt(args, Some(secret), extra, log)
    }

    /// Same as [`spawn`](Self::spawn) but without `SEBAS_CORE_SECRET` in the
    /// environment (harden-core-channel-deployment 5.1/5.2: the core then
    /// auto-arms from a generated key, clients discover it from the file).
    pub fn spawn_no_secret(
        &self,
        args: &[&str],
        extra: &[(&str, &str)],
        log: &Path,
    ) -> tokio::process::Child {
        self.spawn_opt(args, None, extra, log)
    }

    fn spawn_opt(
        &self,
        args: &[&str],
        secret: Option<&str>,
        extra: &[(&str, &str)],
        log: &Path,
    ) -> tokio::process::Child {
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .unwrap_or_else(|e| panic!("open log {}: {e}", log.display()));
        let log_err = log_file
            .try_clone()
            .unwrap_or_else(|e| panic!("clone log handle: {e}"));
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"));
        cmd.args(args)
            .current_dir(&self.path)
            .envs(self.envs(secret))
            .envs(extra.iter().copied())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("spawn sebas {args:?}: {e}"));
        self._dir
            .register_group_leader(child.id().expect("freshly spawned child has a pid"));
        child
    }

    /// 打开节点链路（add-remote-execution-node 9.3）：在沙箱内追加 `[node_link]`
    /// 段（监听回环 + 注册表落在沙箱里），返回实际监听端口。
    ///
    /// **不改默认配置**：节点链路默认关着，给它加段才开——既有旅程因此完全不受
    /// 影响（这本身也是「无节点注册时行为与今日一致」的一个旁证）。
    pub fn enable_node_link(&self) -> u16 {
        let port = free_port();
        let registry = forward_slash(&self.path.join("nodes.json"));
        let mut config = std::fs::read_to_string(&self.config_path)
            .unwrap_or_else(|e| panic!("read config {}: {e}", self.config_path.display()));
        config.push_str(&format!(
            "\n[node_link]\nenabled = true\nlisten = \"127.0.0.1:{port}\"\n\
             registry_file = \"{registry}\"\nbootstrap_token_ttl_secs = 600\n"
        ));
        std::fs::write(&self.config_path, config)
            .unwrap_or_else(|e| panic!("write config {}: {e}", self.config_path.display()));
        port
    }

    /// 节点进程的状态目录（与 `spawn_node` 一致）。
    pub fn node_state_dir(&self) -> PathBuf {
        self.path.join("node-state")
    }

    /// 节点进程的默认工作目录（远端项目的路径；`enable_node_link` 的用例里
    /// 必须真实存在，因为路径可用性由**节点**判定）。
    pub fn node_work_dir(&self) -> PathBuf {
        let dir = self.path.join("node-work");
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("mkdir {}: {e}", dir.display()));
        dir
    }

    /// 起一个**真 `sebas-node` 子进程**（9.3：两个进程，不是同一进程里的两张皮）。
    ///
    /// `join_token` 为 `None` 时用状态目录里已存的长期凭据（重启路径）。
    pub fn spawn_node(&self, node_id: &str, join_token: Option<&str>) -> tokio::process::Child {
        let log = self.path.join("node.log");
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .unwrap_or_else(|e| panic!("open log {}: {e}", log.display()));
        let log_err = log_file
            .try_clone()
            .unwrap_or_else(|e| panic!("clone log handle: {e}"));
        let mut args: Vec<String> = vec![
            "--node-id".into(),
            node_id.into(),
            "--control-plane".into(),
            format!("ws://127.0.0.1:{}", self.node_link_port()),
            "--state-dir".into(),
            forward_slash(&self.node_state_dir()),
        ];
        if let Some(token) = join_token {
            args.push("--join-token".into());
            args.push(token.into());
        }
        let mut cmd = tokio::process::Command::new(sebas_node_bin());
        cmd.args(&args)
            .current_dir(&self.path)
            .env("NO_COLOR", "1")
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("spawn sebas-node {args:?}: {e}"));
        self._dir
            .register_group_leader(child.id().expect("freshly spawned child has a pid"));
        child
    }

    /// 节点链路监听端口（`enable_node_link` 之后才有意义）。
    pub fn node_link_port(&self) -> u16 {
        let config = std::fs::read_to_string(&self.config_path).unwrap_or_default();
        config
            .lines()
            .find_map(|l| l.trim().strip_prefix("listen = \"127.0.0.1:"))
            .and_then(|rest| rest.trim_end_matches('"').parse().ok())
            .unwrap_or_else(|| panic!("配置里没有 [node_link] listen（先调 enable_node_link）"))
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

    /// Core with NO `SEBAS_CORE_SECRET`: the auto-arm path generates the
    /// key, writes the secret file and binds the channel (5.1/5.2).
    pub fn spawn_core_no_secret(&self) -> tokio::process::Child {
        self.spawn_no_secret(
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

    /// Standalone webui with NO `SEBAS_CORE_SECRET`: the client discovers
    /// the key from the secret file at connect time (5.1/5.2).
    pub fn spawn_webui_no_secret(&self) -> tokio::process::Child {
        self.spawn_no_secret(
            &["webui", "-c", &forward_slash(&self.config_path)],
            &[],
            &self.webui_log,
        )
    }

    /// Where the core writes the generated channel key (config dir, D1).
    pub fn secret_file(&self) -> PathBuf {
        self.path.join("core.secret")
    }

    /// Watchdog-supervised form affordance (5.3): pins `[storage] data_dir`
    /// inside the sandbox so upgrade/rollback state never touches the host's
    /// shared XDG dirs. core needs no config toggle — since
    /// enable-core-by-default `sebas run` always spawns the core child. Must
    /// run before spawn.
    pub fn enable_supervised_core(&self) {
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let patched = toml.replace(
            "[router]",
            &format!(
                "[storage]\ndata_dir = \"{}\"\n\n[router]",
                forward_slash(&self.path.join("storage"))
            ),
        );
        assert!(
            patched != toml && patched.contains("[storage]"),
            "[router] section not found in config"
        );
        std::fs::write(&self.config_path, patched).expect("write config");
    }

    /// workbench-turn-queue：给 fake-claude 加 `--slow-ms`，让每个 turn 在
    /// 内容帧之后、result 之前停留 `ms` 毫秒——WORKING 窗口因此确定性的长，
    /// 忙中提交必然入队。必须在 spawn 之前调用。
    pub fn slow_fake_agent(&self, ms: u64) {
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let needle = "[acp.agents.claude]\n";
        assert!(
            toml.contains(needle),
            "[acp.agents.claude] section not found in config"
        );
        let patched = toml.replace(
            needle,
            &format!("{needle}args = [\"--slow-ms\", \"{ms}\"]\n"),
        );
        assert_ne!(toml, patched, "slow-agent patch did not apply");
        std::fs::write(&self.config_path, patched).expect("write config");
    }

    /// （add-agent-mode-selection）给 fake-claude 加 `--journal`，让每个
    /// spawn 的 argv 与 in/out 帧都落盘——argv 断言（mode 是否进了子进程
    /// 参数）与运行时切换断言（mode_change 记录）的数据源。必须在 spawn
    /// 之前调用。
    pub fn journal_fake_agent(&self) -> std::path::PathBuf {
        let journal = self.path.join("fake-claude-journal.jsonl");
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let needle = "[acp.agents.claude]\n";
        assert!(
            toml.contains(needle),
            "[acp.agents.claude] section not found in config"
        );
        let arg = format!(
            "{needle}args = [\"--journal\", \"{}\"]\n",
            journal.display()
        );
        let patched = toml.replace(needle, &arg);
        assert_ne!(toml, patched, "journal patch did not apply");
        std::fs::write(&self.config_path, patched).expect("write config");
        journal
    }

    /// Pin the router's HTTP listener to a specific port inside the sandbox
    /// config (the default `127.0.0.1:8787` is fixed — parallel e2e cases
    /// would collide). Must run before spawn.
    pub fn set_router_listen(&self, port: u16) {
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let patched = toml.replace(
            "[router]",
            &format!("[router]\nlisten = \"127.0.0.1:{port}\""),
        );
        assert!(
            patched != toml && patched.contains("[router]"),
            "[router] section not found in config"
        );
        std::fs::write(&self.config_path, patched).expect("write config");
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

    pub fn webui_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.webui_port)
    }

    /// Require a downstream token on the router proxy surface (`auth_token`
    /// inserted into the `[router]` section). Must be called before spawn.
    pub fn set_router_auth_token(&self, token: &str) {
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let patched = toml.replace("[router]", &format!("[router]\nauth_token = \"{token}\""));
        assert_ne!(toml, patched, "[router] section not found in config");
        std::fs::write(&self.config_path, patched).expect("write config");
    }

    /// In-process webui form: `sebas run -c <config> --router --debug
    /// --webui --webui-port <p>`. Returns the child and the dashboard port.
    /// Extra envs may pin the router port (`SEBAS_ROUTER_LISTEN`) so the
    /// native agent can be pointed at it via `SEBAS_AGENT_ROUTER_URL`.
    pub fn spawn_core_inprocess_webui(
        &self,
        extra: &[(&str, &str)],
    ) -> (tokio::process::Child, u16) {
        let dashboard_port = free_port();
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.core_log)
            .unwrap_or_else(|e| panic!("open log {}: {e}", self.core_log.display()));
        let log_err = log_file
            .try_clone()
            .unwrap_or_else(|e| panic!("clone log handle: {e}"));
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"));
        cmd.args([
            "core",
            "-c",
            &forward_slash(&self.config_path),
            "--router",
            "--debug",
            "--webui",
            "--webui-port",
            &dashboard_port.to_string(),
        ])
        .current_dir(&self.path)
        .envs(self.envs(Some(&self.core_secret)))
        .envs(extra.iter().copied())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_err))
        .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("spawn sebas in-process webui: {e}"));
        self._dir
            .register_group_leader(child.id().expect("freshly spawned child has a pid"));
        (child, dashboard_port)
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
    wait_for(
        "core reachability ok",
        Duration::from_secs(30),
        &hint,
        move || {
            let cli = cli.clone();
            let url = url.clone();
            Box::pin(async move {
                get_json(&cli, &url)
                    .await
                    .and_then(|v| v.get("reachability").cloned())
                    .filter(|r| r["ok"].as_bool() == Some(true))
            })
        },
    )
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
                let r = get_json(&cli, &url).await?.get("reachability").cloned()?;
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
    wait_for(
        "router addr in core log",
        Duration::from_secs(15),
        &hint,
        move || {
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
        },
    )
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

// ---- 本地假上游（anthropic 协议应答）------------------------------------------------

/// Spawn a local stub upstream answering any request with a fixed
/// anthropic-style message, recording the model it was asked for. Binds
/// 127.0.0.1 on a probed free port and returns it — no external traffic.
/// Shared by the acceptance journeys and the process-level e2e suite.
pub async fn spawn_stub_upstream(asked_model: Arc<tokio::sync::Mutex<Option<String>>>) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub upstream");
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                continue;
            };
            let asked = asked_model.clone();
            tokio::spawn(async move {
                // Read until end of headers, then exactly content-length bytes.
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 8192];
                let header_end = loop {
                    if buf.len() >= chunk.len() * 4 {
                        return; // runaway request; drop
                    }
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                        // ensure full body arrived too
                        let headers = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let len: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + len {
                            break pos + 4 + len;
                        }
                    }
                };
                let text = String::from_utf8_lossy(&buf[..header_end]);
                if let Some(model_key) = find_json_string(&text, "\"model\":\"") {
                    *asked.lock().await = Some(model_key);
                }
                let body = r#"{"id":"msg_stub","type":"message","role":"assistant","model":"stub-model","content":[{"type":"text","text":"stub reply"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":1}}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    port
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Crude extractor for the first `"model":"…"` value in a JSON body —
/// enough for the stub's recording purposes.
fn find_json_string(text: &str, key_prefix: &str) -> Option<String> {
    let start = text.find(key_prefix)? + key_prefix.len();
    let rest = &text[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
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
