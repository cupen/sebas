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
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub struct SandboxDir {
    /// Path the test uses for every sandbox file and env var. Normally the
    /// real directory; on a deep checkout it is a short symlink to it (see
    /// `shorten_socket_host`) so that unix socket paths stay under
    /// `sun_path`'s 108-byte ceiling.
    path: PathBuf,
    /// The real directory (`target/tests/...`), where logs and state stay
    /// for postmortem inspection regardless of which path is in use.
    real_path: PathBuf,
    /// The short symlink to remove on drop, when one was created.
    short_link: Option<PathBuf>,
    keep: AtomicBool,
    /// PIDs spawned as their own process-group leaders (unix). Teardown
    /// killpg's each group so test-spawned routers and watchdog-respawned
    /// cores die with the test even though `kill_on_drop` only reaps the
    /// direct child (sebas-gc7 leak).
    group_leaders: Mutex<Vec<u32>>,
}

/// `sun_path` 上限（108 字节，含结尾 NUL）下的可用字符数。
#[cfg(unix)]
const SUN_PATH_MAX_CHARS: usize = 107;

/// 沙箱路径长到 socket 放不下时，改为经由一个短符号链接使用沙箱。
///
/// 为什么需要：`target/tests/sebas/<test>/<stamp>-<sub>/core-channel.sock`
/// 在深 checkout（如 `/data/workbench/repos-ai/<repo>`）下会超过
/// `sun_path` 的 108 字节上限，core 的 channel bind 与测试侧的 connect 都
/// 会以 "local socket name length exceeds capacity of sun_path" 失败。
/// 内核检查的是**传入字符串**的长度，符号链接的解析发生在检查之后，所以
/// 让测试用短链接路径即可，真实目录（含日志）仍留在 `target/tests/` 下便于
/// 事后诊断。
///
/// 非 unix 平台（named pipe 无此上限）以及链接创建失败时，原样返回真实
/// 路径——最坏情况与今日行为一致，不会更糟。
#[cfg(unix)]
fn shorten_socket_host(real: PathBuf, stamp: u128) -> (PathBuf, Option<PathBuf>) {
    const SOCK_NAME: &str = "core-channel.sock";
    if real.join(SOCK_NAME).as_os_str().as_encoded_bytes().len() <= SUN_PATH_MAX_CHARS {
        return (real, None);
    }
    let link = std::env::temp_dir().join(format!("sebas-sb-{stamp:x}"));
    let _ = std::fs::remove_file(&link);
    match std::os::unix::fs::symlink(&real, &link) {
        Ok(()) => (link.clone(), Some(link)),
        Err(e) => {
            eprintln!(
                "[sandbox] could not shorten sandbox path via {}: {e}; \
                 unix socket paths may exceed sun_path and fail",
                link.display()
            );
            (real, None)
        }
    }
}

#[cfg(not(unix))]
fn shorten_socket_host(real: PathBuf, _stamp: u128) -> (PathBuf, Option<PathBuf>) {
    (real, None)
}

impl SandboxDir {
    fn new(test_name: &str, sub: &str) -> Arc<Self> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let stamp = unique_stamp();
        let real_path = manifest
            .join("target")
            .join("tests")
            .join("sebas")
            .join(test_name)
            .join(format!("{stamp}-{sub}"));
        std::fs::create_dir_all(&real_path)
            .unwrap_or_else(|e| panic!("create sandbox dir {}: {e}", real_path.display()));
        let (path, short_link) = shorten_socket_host(real_path.clone(), stamp);
        Arc::new(Self {
            path,
            real_path,
            short_link,
            keep: AtomicBool::new(false),
            group_leaders: Mutex::new(Vec::new()),
        })
    }

    /// Record a spawned child as its own process-group leader.
    fn register_group_leader(&self, pid: u32) {
        self.group_leaders.lock().unwrap().push(pid);
    }

    /// Kill every spawned process tree. Grandchildren must die with the test:
    /// teardown SIGKILLs each recorded group leader's whole tree. Runs on the
    /// keep-path too: diagnosis needs the logs on disk, not the processes
    /// writing them.
    fn kill_process_groups(&self) {
        for pid in self.group_leaders.lock().unwrap().drain(..) {
            kill_tree(pid);
        }
    }
}

/// Kill a process tree by leader pid — the platform wrapper (same name and
/// signature everywhere; D2). unix: the leader is its own process-group head,
/// so one `killpg` SIGKILL reaches every descendant (ESRCH on a fully-dead
/// group is fine). windows: `kill_on_drop` only reaps the direct child, so
/// `taskkill /T /F` tears down the tree instead — an already-exited target
/// reports failure, which is success for teardown (zero new deps, zero
/// unsafe; the Job-Object upgrade path stays recorded in the change design).
#[cfg(unix)]
fn kill_tree(pid: u32) {
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Give a spawned child its own process group so teardown can address the
/// whole tree — the platform wrapper. windows has no process-group concept
/// in this harness; `kill_tree` walks the tree instead, so it's a no-op.
#[cfg(unix)]
fn set_process_group(cmd: &mut tokio::process::Command) {
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn set_process_group(_cmd: &mut tokio::process::Command) {}

impl Drop for SandboxDir {
    fn drop(&mut self) {
        self.kill_process_groups();
        if self.keep.load(Ordering::Relaxed) || std::thread::panicking() {
            eprintln!(
                "[sandbox] kept for diagnosis (logs inside): {}",
                self.real_path.display()
            );
            return;
        }
        // Children may still be releasing file handles; retry a few times.
        // Remove the real directory (never the link, which `remove_dir_all`
        // would refuse to follow) and then the link itself.
        for _ in 0..3 {
            if std::fs::remove_dir_all(&self.real_path).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if let Some(link) = &self.short_link {
            let _ = std::fs::remove_file(link);
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
    let name = if cfg!(windows) {
        "sebas-node.exe"
    } else {
        "sebas-node"
    };
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
    /// Router 子进程日志（独立进程形态：`sebas router --config … --debug`）。
    pub router_log: PathBuf,
    /// 沙箱配置里钉住的 router 监听端口（probed free port）——默认 8787 是
    /// 固定值，并行的 e2e/验收用例会互踩。
    pub router_port: u16,
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
        let router_port = free_port();
        let config_path = path.join("config.toml");
        let channel_path = path.join("core-channel.sock");
        let state_file = path.join("sessions.json");
        let core_log = path.join("core.log");
        let webui_log = path.join("webui.log");
        let router_log = path.join("router.log");
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

# add-workspace-root 4.1：沙箱钉根——项目注册/列表/会话面/browse-dirs 的唯一
# 边界收敛在沙箱目录内。不配会回退进程 cwd（= 仓库根）并打启动告警。
[workspace]
root = "{}"

# add-agent-skills：skill 仓钉进沙箱。缺省值 ~/.agents/skills 经 expand_tilde
# （dirs::home_dir()，不吃 HOME env 覆写）落到操作员真实仓——必须显式钉。
[skills]
dir = "{}"

[service.core]
channel_path = "core-channel.sock"

[service.webui]
enabled = true
host = "127.0.0.1"
port = {webui_port}
# auth 默认 true；API 断言沙箱一律免登录，显式关闭（webui 登录旅程由
# testsuite-webui 专测）。开启形态的凭据走沙箱内 SEBAS_WEBUI_AUTH_DB +
# env 引导 / webui-passwd，绝不落真实 ~/.sebas。
auth = false

# router validate requires >=1 provider with a base_url; the debug `test`
# provider is injected only after parse. This dummy never dials anything
# in debug mode.
[provider.anthropic]
api_key = "sk-sandbox-dummy"

[router]
# 默认 listen 是固定 8787——并行用例互踩，每个沙箱钉一个 probed 端口。
listen = "127.0.0.1:{router_port}"
provider_overlay = "{}"
usage_file = "{}"
"#,
            forward_slash(&path.join("claude-sessions")),
            forward_slash(&path.join("work")),
            forward_slash(&state_file),
            forward_slash(&path.join("downloads")),
            forward_slash(&path),
            forward_slash(&path.join("agents-skills")),
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
            router_log,
            router_port,
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
            // WebUI 用户库（add-webui-multiuser-rbac）：auth = false 时无人
            // 打开它，钉进沙箱是纵深防御——任何开启 auth 或调 webui-passwd
            // 的变体都不会写到操作者的真实 ~/.sebas/auth.db。
            (
                "SEBAS_WEBUI_AUTH_DB",
                forward_slash(&self.path.join("auth.db")),
            ),
            // archive.json falls back to SEBAS_HOME ($HOME/.sebas) — pin both
            // into the sandbox: without this the suites read AND rewrote the
            // operator's real archive (93-entry test pollution, 2026-09-12).
            ("SEBAS_HOME", forward_slash(&self.path)),
            (
                "SEBAS_ARCHIVE_PATH",
                forward_slash(&self.path.join("archive.json")),
            ),
            // add-agent-skills：skills sync 的 backend 落点（claude →
            // ~/.claude/skills）经 `skills::resolve_home()` 的 env-first
            // （HOME > USERPROFILE > Known Folder）解析——钉进沙箱，core/webui
            // 进程里的 sync 绝不写操作员的真实 ~/.claude/skills。
            ("HOME", forward_slash(&self.path)),
            // add-workspace-root：env 优先于 config 的 `[workspace] root`——
            // 钉住它，宿主 shell 里 stray 的同名变量就不会把沙箱边界改道
            // （测试需要更窄根时用 spawn 的 extra env 显式覆盖）。
            ("SEBAS_WORKSPACE_ROOT", forward_slash(&self.path)),
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
        set_process_group(&mut cmd);
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

    /// （session-parallel-liveness-and-unread-polish 1.2）给 fake-claude agent
    /// 追加 argv（如 `--delay-init-ms 2500` 拉长握手，让两个 spawn 指令的重叠
    /// 可测量）。在 `spawn_core` 之前调用；按文本把 args 写进既有的
    /// `[acp.agents.claude]` 段（该段由 `new` 写出且当前不带 args 键）。
    pub fn append_acp_args(&self, extra: &[&str]) {
        let config = std::fs::read_to_string(&self.config_path)
            .unwrap_or_else(|e| panic!("read config {}: {e}", self.config_path.display()));
        let quoted = extra
            .iter()
            .map(|a| format!("\"{a}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let anchor = "driver = \"claude\"\n";
        assert!(
            config.contains(anchor) && !config.contains("args ="),
            "sandbox claude section must exist and carry no args yet"
        );
        let rewritten = config.replacen(anchor, &format!("{anchor}args = [{quoted}]\n"), 1);
        std::fs::write(&self.config_path, rewritten)
            .unwrap_or_else(|e| panic!("write config {}: {e}", self.config_path.display()));
    }

    /// 节点进程的默认工作目录（远端项目的路径；`enable_node_link` 的用例里
    /// 必须真实存在，因为路径可用性由**节点**判定）。
    pub fn node_work_dir(&self) -> PathBuf {
        let dir = self.path.join("node-work");
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("mkdir {}: {e}", dir.display()));
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
            // add-workspace-root 4.1：节点自判项目路径 containment，边界同样
            // 钉在沙箱目录（env > [node] workspace_root > cwd 回退告警）。
            .env("SEBAS_WORKSPACE_ROOT", forward_slash(&self.path))
            .env("NO_COLOR", "1")
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true);
        set_process_group(&mut cmd);
        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("spawn sebas-node {args:?}: {e}"));
        self._dir
            .register_group_leader(child.id().expect("freshly spawned child has a pid"));
        child
    }

    /// 节点链路监听端口（`enable_node_link` 之后才有意义）。
    ///
    /// 按 section 作用域解析：config 里 `[router] listen` 也在（两进程形态给
    /// router 针的 probed 端口），全局找第一个 `listen =` 会错拿 router 的。
    pub fn node_link_port(&self) -> u16 {
        let config = std::fs::read_to_string(&self.config_path).unwrap_or_default();
        let mut in_node_link = false;
        for line in config.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_node_link = trimmed == "[node_link]";
                continue;
            }
            if in_node_link && let Some(rest) = trimmed.strip_prefix("listen = \"127.0.0.1:") {
                return rest
                    .trim_end_matches('"')
                    .parse()
                    .unwrap_or_else(|e| panic!("解析 [node_link] listen 失败: {e}"));
            }
        }
        panic!("配置里没有 [node_link] listen（先调 enable_node_link）")
    }

    /// Core: `sebas core -c <config>`（bare core，无 router——需要 router 就
    /// 另行 spawn 独立子进程 [`Self::spawn_router_debug`]；channel socket 因
    /// SEBAS_CORE_SECRET 已设置而照常出现）。
    pub fn spawn_core(&self) -> tokio::process::Child {
        self.spawn_core_extra(&[])
    }

    /// Same as `spawn_core` with extra env vars (test affordances like
    /// `SEBAS_TEST_SPAWN_SESSION=1`).
    pub fn spawn_core_extra(&self, extra: &[(&str, &str)]) -> tokio::process::Child {
        self.spawn(
            &["core", "-c", &forward_slash(&self.config_path)],
            &self.core_secret,
            extra,
            &self.core_log,
        )
    }

    /// Core with NO `SEBAS_CORE_SECRET`: the auto-arm path generates the
    /// key, writes the secret file and binds the channel (5.1/5.2).
    pub fn spawn_core_no_secret(&self) -> tokio::process::Child {
        self.spawn_no_secret(
            &["core", "-c", &forward_slash(&self.config_path)],
            &[],
            &self.core_log,
        )
    }

    /// Router 子进程（unify-router-process-shape D5 两进程形态）：
    /// `sebas router -c <config> --debug`（内置 test provider、下游免鉴权）。
    /// 地址从 [`wait_router_addr`] 读 router.log 获得；进程保活在返回的
    /// Child（kill_on_drop）里，SandboxDir Drop 再 killpg 兜底。
    pub fn spawn_router_debug(&self) -> tokio::process::Child {
        self.spawn(
            &["router", "-c", &forward_slash(&self.config_path), "--debug"],
            &self.core_secret,
            &[],
            &self.router_log,
        )
    }

    /// Router 子进程，不带 `--debug`（下游 auth 强制生效；无内置 test
    /// provider）。供下游鉴权拒绝类旅程。
    pub fn spawn_router(&self) -> tokio::process::Child {
        self.spawn(
            &["router", "-c", &forward_slash(&self.config_path)],
            &self.core_secret,
            &[],
            &self.router_log,
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

    /// fix-pending-queue-liveness：把 `[dispatch] turn_stall_timeout`（秒）
    /// 写进沙箱 config——停滞看门狗的短阈值供 e2e 快速周转。必须在 spawn
    /// 之前调用。
    pub fn set_turn_stall_timeout(&self, secs: u64) {
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let needle = "[dispatch]\n";
        assert!(
            toml.contains(needle),
            "[dispatch] section not found in config"
        );
        let patched = toml.replace(needle, &format!("{needle}turn_stall_timeout = {secs}\n"));
        assert_ne!(toml, patched, "turn_stall_timeout patch did not apply");
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
        // TOML basic strings treat `\` as an escape — the path must go in with
        // forward slashes (same convention as the base config), or Windows
        // cores/webuis die at startup on a parse error.
        let arg = format!(
            "{needle}args = [\"--journal\", \"{}\"]\n",
            forward_slash(&journal)
        );
        let patched = toml.replace(needle, &arg);
        assert_ne!(toml, patched, "journal patch did not apply");
        std::fs::write(&self.config_path, patched).expect("write config");
        journal
    }

    /// （session-slash-commands 5.2）给 fake-claude 加 `--advertise-commands`
    /// （initialize 控制响应带固定命令表 goal/compact——命令物化与 webui
    /// 载荷的进程级数据源）+ `--journal`（in 帧落盘——断言 slash 提交原样
    /// 到达 stub 的数据源）。必须在 spawn 之前调用；返回 journal 路径。
    pub fn advertising_journal_fake_agent(&self) -> std::path::PathBuf {
        let journal = self.path.join("fake-claude-journal.jsonl");
        let toml = std::fs::read_to_string(&self.config_path).expect("read config");
        let needle = "[acp.agents.claude]\n";
        assert!(
            toml.contains(needle),
            "[acp.agents.claude] section not found in config"
        );
        // TOML basic strings treat `\` as an escape — forward slashes only
        // (same convention as journal_fake_agent).
        let arg = format!(
            "{needle}args = [\"--advertise-commands\", \"--journal\", \"{}\"]\n",
            forward_slash(&journal)
        );
        let patched = toml.replace(needle, &arg);
        assert_ne!(toml, patched, "advertise+journal patch did not apply");
        std::fs::write(&self.config_path, patched).expect("write config");
        journal
    }

    /// Standalone webui: `sebas webui -c <config>`; `secret` is what the
    /// webui presents to the core channel (pass a different one for
    /// wrong-secret cases).
    pub fn spawn_webui(&self, secret: &str) -> tokio::process::Child {
        self.spawn_webui_extra(secret, &[])
    }

    /// Same as [`Self::spawn_webui`] with extra env vars.
    pub fn spawn_webui_extra(&self, secret: &str, extra: &[(&str, &str)]) -> tokio::process::Child {
        self.spawn(
            &["webui", "-c", &forward_slash(&self.config_path)],
            secret,
            extra,
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

    /// fake-provider-upstream 3.2：给 sandbox config 追加 `[provider.fake]` 段
    /// （哑 key + `base_url_anthropic` 指向 fake 上游 + models 列表），让 router
    /// 以**真透传**形态路由到本地假上游（namespace `fake/<model>` 直达）。
    ///
    /// **不改默认配置**（同 [`Self::enable_node_link`] 的取舍）：既有旅程的配置
    /// 逐字不变——尤其 router 的「唯一 provider 隐式默认」语义不被新增 provider
    /// 打断。必须在 spawn router/core **之前**调用；`base_url` 来自
    /// [`Self::spawn_fake_provider`]（端口是 probed 随机值）。
    pub fn enable_fake_provider(&self, base_url: &str) {
        let config = std::fs::read_to_string(&self.config_path)
            .unwrap_or_else(|e| panic!("read config {}: {e}", self.config_path.display()));
        assert!(
            !config.contains("[provider.fake]"),
            "enable_fake_provider called twice"
        );
        let patched = format!(
            "{config}\n# fake-provider-upstream：本地 Anthropic 线协议假上游（可拨，\n\
             # 零外呼）。哑上游 key——journal 会明文记 header，绝不可指向生产。\n\
             [provider.fake]\napi_key = \"sk-fake-upstream-dummy\"\n\
             base_url_anthropic = \"{base_url}\"\nmodels = [\"fake-model\"]\n"
        );
        assert_ne!(patched, config);
        std::fs::write(&self.config_path, patched).expect("write config");
    }

    /// fake-provider-upstream 3.4：钉 `[router.rate_limit]`（token bucket）。
    /// 必须在 spawn router 之前调用。
    pub fn set_router_rate_limit(&self, capacity: u32, refill_per_sec: f64) {
        let mut config = std::fs::read_to_string(&self.config_path)
            .unwrap_or_else(|e| panic!("read config {}: {e}", self.config_path.display()));
        config.push_str(&format!(
            "\n[router.rate_limit]\ncapacity = {capacity}\nrefill_per_sec = {refill_per_sec}\n"
        ));
        std::fs::write(&self.config_path, config).expect("write config");
    }

    /// fake-provider-upstream 3.5：把 `[acp.agents.claude]` 的 `path` 换成
    /// **真 claude-code 二进制**（并丢掉 fake-claude 专属 `args`），让
    /// agent-loop 旅程跑真实 ACP 执行体。必须在 spawn core 之前调用。
    pub fn set_agent_claude_path(&self, path: &str) {
        let toml = std::fs::read_to_string(&self.config_path)
            .unwrap_or_else(|e| panic!("read config {}: {e}", self.config_path.display()));
        let mut lines: Vec<String> = toml.lines().map(str::to_string).collect();
        let start = lines
            .iter()
            .position(|l| l.trim() == "[acp.agents.claude]")
            .unwrap_or_else(|| panic!("[acp.agents.claude] section not found"));
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.trim_start().starts_with('['))
            .map(|i| i + start + 1)
            .unwrap_or(lines.len());
        let mut section: Vec<String> = vec![lines[start].clone()];
        for line in &lines[start + 1..end] {
            let key = line.split('=').next().unwrap_or("").trim();
            if key == "path" || key == "args" {
                continue;
            }
            section.push(line.clone());
        }
        section.push(format!("path = \"{}\"", forward_slash(Path::new(path))));
        let mut rebuilt: Vec<String> = lines[..start].to_vec();
        rebuilt.extend(section);
        rebuilt.extend(lines[end..].to_vec());
        lines = rebuilt;
        std::fs::write(&self.config_path, lines.join("\n") + "\n").expect("write config");
    }

    /// fake-provider-upstream 3.1：spawn 一个**真 `sebas fake-provider` 子进程**
    /// （CARGO_BIN_EXE_sebas + 动词 + 随机端口 + journal 落在 sandbox 内），
    /// 解析 ready 行拿到实际地址；拆卸由 `kill_on_drop` + SandboxDir 的进程组
    /// 收割（[`kill_tree`]）负责。
    pub async fn spawn_fake_provider(&self, scenario: Option<&Path>) -> FakeUpstream {
        let log = self.path.join("fake-provider.log");
        let journal = self.path.join("fake-provider-journal.jsonl");
        let listen = "127.0.0.1:0".to_string();
        let journal_arg = forward_slash(&journal);
        let scenario_arg = scenario.map(forward_slash);
        let mut args: Vec<&str> = vec![
            "fake-provider",
            "--listen",
            &listen,
            "--journal",
            &journal_arg,
        ];
        if let Some(s) = &scenario_arg {
            args.push("--scenario");
            args.push(s);
        }
        let child = self.spawn(&args, &self.core_secret, &[], &log);
        let base_url = wait_fake_provider_addr(&log, &self.path).await;
        let port = base_url
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or_else(|| panic!("unparseable fake-provider base url: {base_url}"));
        FakeUpstream {
            child,
            base_url,
            port,
            journal,
            log,
        }
    }

    /// In-process webui form: `sebas core -c <config> --webui --webui-port
    /// <p>`（无 router 旗标——router 只以独立进程运行，见
    /// [`Self::spawn_router_debug`]）。Returns the child and the dashboard port.
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

/// 一个运行中的 `sebas fake-provider` 子进程（fake-provider-upstream 3.1）。
/// `child` 保活（drop 即 kill_on_drop 拆卸）；`journal` 是离线断言透传行为的
/// 数据源（method/path/headers/body 逐行 NDJSON）。
pub struct FakeUpstream {
    pub child: tokio::process::Child,
    /// `http://127.0.0.1:<port>`（ready 行解析出的实际绑定地址）。
    pub base_url: String,
    pub port: u16,
    pub journal: PathBuf,
    pub log: PathBuf,
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

/// Parse the router bind address from the router child's log
/// (`sebas router listening … addr=127.0.0.1:<port>`；独立进程形态，
/// 地址不再写进 core.log)。
pub async fn wait_router_addr(sb: &Sandbox) -> String {
    let log_path = sb.router_log.clone();
    let hint = sb.path.clone();
    wait_for(
        "router addr in router log",
        Duration::from_secs(15),
        &hint,
        move || {
            let log_path = log_path.clone();
            Box::pin(async move {
                let log = std::fs::read_to_string(&log_path).ok()?;
                for line in log.lines().rev() {
                    let line = strip_ansi(line);
                    if line.contains("router listening")
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

/// Parse the fake upstream's bind address from its stdout log
/// (`fake-provider listening addr=127.0.0.1:<port>`；ready 行契约见
/// `sebas_router::fake_provider::announce`）。返回 `http://…` 基址。
pub async fn wait_fake_provider_addr(log: &Path, hint: &Path) -> String {
    let log_path = log.to_path_buf();
    let hint = hint.to_path_buf();
    wait_for(
        "fake-provider addr in log",
        Duration::from_secs(15),
        &hint,
        move || {
            let log_path = log_path.clone();
            Box::pin(async move {
                let log = std::fs::read_to_string(&log_path).ok()?;
                for line in log.lines().rev() {
                    let line = strip_ansi(line);
                    let marker = "fake-provider listening addr=";
                    let idx = line.find(marker)?;
                    let addr = line[idx + marker.len()..].split_whitespace().next()?;
                    if addr.parse::<std::net::SocketAddr>().is_ok() {
                        return Some(format!("http://{addr}"));
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

// ---- 流式假上游 + /ws 客户端（fix-webui-streaming-liveness 6.1/6.2/6.3）----

/// Spawn a local stub upstream that answers any request with an Anthropic
/// SSE stream carrying the given text deltas, `spacing_ms` apart, then the
/// closing stop/end_turn/stop events. Binds 127.0.0.1 on a probed free port
/// and returns it — no external traffic.
///
/// This is the upstream for the native-kernel streaming e2e: the kernel's
/// `AnthropicMessagesClient` posts `{stream:true}` to `{base}/v1/messages`
/// and turns each `content_block_delta` into a `StreamEvent::TextDelta`, so
/// spacing the deltas in time is what makes the downstream pump land them
/// (and broadcast them) one by one.
pub async fn spawn_sse_stub_upstream(chunks: &[&str], spacing_ms: u64) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind sse stub upstream");
    let port = listener.local_addr().unwrap().port();
    let chunks: Vec<String> = chunks.iter().map(|s| s.to_string()).collect();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                continue;
            };
            let chunks = chunks.clone();
            tokio::spawn(async move {
                // Read the request to its end (headers + content-length body),
                // same discipline as spawn_stub_upstream.
                let mut buf: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let len: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + len {
                            break;
                        }
                    }
                }
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                let frame = |json: String| format!("event: anthropic\ndata: {json}\n\n");
                let _ = sock
                    .write_all(
                        frame(
                            serde_json::json!({"type":"message_start","message":{"id":"msg_sse_stub","type":"message","role":"assistant","model":"stub-model","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}}})
                                .to_string(),
                        )
                        .as_bytes(),
                    )
                    .await;
                let _ = sock
                    .write_all(
                        frame(
                            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})
                                .to_string(),
                        )
                        .as_bytes(),
                    )
                    .await;
                for text in &chunks {
                    tokio::time::sleep(Duration::from_millis(spacing_ms)).await;
                    let _ = sock
                        .write_all(
                            frame(
                                serde_json::json!({
                                    "type":"content_block_delta","index":0,
                                    "delta":{"type":"text_delta","text":text}
                                })
                                .to_string(),
                            )
                            .as_bytes(),
                        )
                        .await;
                }
                let _ = sock
                    .write_all(
                        frame(
                            serde_json::json!({"type":"content_block_stop","index":0}).to_string(),
                        )
                        .as_bytes(),
                    )
                    .await;
                let _ = sock
                    .write_all(
                        frame(
                            serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}})
                                .to_string(),
                        )
                        .as_bytes(),
                    )
                    .await;
                let _ = sock
                    .write_all(frame(serde_json::json!({"type":"message_stop"}).to_string()).as_bytes())
                    .await;
                let _ = sock.shutdown().await;
            });
        }
    });
    port
}

/// A live WebUI `/ws` connection (process-level e2e). The stream auto-answers
/// protocol pings while being read; drop it to disconnect.
pub type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Connect to the WebUI WebSocket endpoint (`http://…` base is rewritten to
/// `ws://` and `/ws` appended).
pub async fn ws_connect(base: &str) -> WsStream {
    let url = format!("{}/ws", base.replacen("http://", "ws://", 1));
    let (stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("ws connect");
    stream
}

/// Next JSON frame from the socket (text payloads only; control frames are
/// skipped). Hard timeout so a bug surfaces as a failure, not a hang.
pub async fn next_ws_frame(
    ws: &mut (impl futures_util::StreamExt<
        Item = Result<
            tokio_tungstenite::tungstenite::Message,
            tokio_tungstenite::tungstenite::Error,
        >,
    > + Unpin),
) -> serde_json::Value {
    let msg = tokio::time::timeout(Duration::from_secs(15), ws.next())
        .await
        .expect("timed out waiting for a ws frame")
        .expect("websocket stream ended")
        .expect("websocket error");
    match msg {
        tokio_tungstenite::tungstenite::Message::Text(text) => serde_json::from_str(&text)
            .expect("ws frame must be envelope JSON"),
        _ => Box::pin(next_ws_frame(ws)).await,
    }
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
    fn enable_fake_provider_and_rate_limit_patch_config() {
        // fake-provider-upstream 3.2：模板追加 `[provider.fake]`（哑 key + 指向
        // fake 的 base_url + models）与 `[router.rate_limit]`；既有段不动。
        let sb = Sandbox::new("self", "fake-provider-patch");
        let before = std::fs::read_to_string(&sb.config_path).expect("base config");
        sb.enable_fake_provider("http://127.0.0.1:12345");
        sb.set_router_rate_limit(2, 0.0001);
        let after = std::fs::read_to_string(&sb.config_path).expect("patched config");

        assert!(after.starts_with(&before), "既有配置只能追加，不能改写");
        assert!(after.contains("[provider.fake]"));
        assert!(after.contains("api_key = \"sk-fake-upstream-dummy\""));
        assert!(after.contains("base_url_anthropic = \"http://127.0.0.1:12345\""));
        assert!(after.contains("models = [\"fake-model\"]"));
        assert!(after.contains("[router.rate_limit]"));
        assert!(after.contains("capacity = 2"));
        assert!(after.contains("refill_per_sec = 0.0001"));
        // TOML 可解析（追加段没有破坏语法）——用 toml crate 通过测试二进制
        // 的依赖树不可用，故只做结构断言行（段头 + key）。
        assert_eq!(
            after.matches("[provider.fake]").count(),
            1,
            "stanza must appear exactly once"
        );
    }

    #[test]
    fn set_agent_claude_path_rewrites_only_that_section() {
        // fake-provider-upstream 3.5：真 claude-code 取代 fake-claude 桩，
        // 同段的 sessions_dir/work_dir 保留，树里不留 fake-claude 路径。
        let sb = Sandbox::new("self", "claude-path");
        sb.set_agent_claude_path("/opt/bin/claude");
        let cfg = std::fs::read_to_string(&sb.config_path).expect("config");
        assert!(cfg.contains("[acp.agents.claude]"));
        assert!(cfg.contains("path = \"/opt/bin/claude\""));
        assert!(
            !cfg.contains("fake-claude"),
            "fake-claude path must be gone: {cfg}"
        );
        assert!(cfg.contains("sessions_dir ="));
        assert!(cfg.contains("work_dir ="));
        assert!(cfg.contains("driver = \"claude\""));
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
