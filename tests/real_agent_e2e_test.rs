//! Real-backend agent e2e (`invoke testsuite-real-agents`): first-message
//! round-trip journeys against REAL agent backends — 项目管理 (project
//! registration), 会话管理 (session spawn/close) and agent 通信 (the first
//! message completes with a real model reply).
//!
//! Two journeys, same shape:
//!
//! - `real_opencode_first_message_roundtrip` — `opencode acp` (opencode's
//!   native Agent Client Protocol server), agent id `opencode`.
//!   Requires the `opencode` CLI on PATH with its own authentication
//!   (`~/.local/share/opencode/auth.json`); otherwise it skips with a
//!   one-line reason (same early-return pattern as the cross_uid `[skip]`).
//! - `real_claude_first_message_roundtrip` — the dedicated Claude driver
//!   spawning the real `claude` CLI (Claude Agent SDK, uses claude's own
//!   login), agent id `claude`. Requires `claude` on PATH AND
//!   `claude auth status` → `loggedIn: true`; otherwise it skips with
//!   "claude CLI not logged in — run `claude` → /login; journey ready".
//!   The moment the user logs in, the journey runs with zero code changes.
//!
//! Opt-in only (`#[ignore]`, run with `--test-threads=1`): every turn costs
//! REAL tokens and takes 10–120 s of wall clock. Run via
//! `cargo test --test real_agent_e2e_test -- --ignored --test-threads=1`
//! or `invoke testsuite-real-agents`.
//!
//! Isolation mirrors the proven `testsuite_e2e_test` sandbox recipe: all
//! sebas state (config, state DB, state file, provider overlay, dispatch /
//! usage files, channel socket) lives inside one throwaway scene dir under
//! `target/tests/` — removed on drop, kept for postmortem on panic. HOME is
//! deliberately NOT scrubbed: the agent CLIs resolve their own credentials
//! under `$HOME` (explicitly authorized for these tests); only sebas's own
//! state is sandboxed. The operator's real instance (port 9797, `~/.sebas`,
//! `~/.config/sebas`) is never touched — the webui binds a probed free high
//! port and every env default that would fall back to `~/.sebas` is
//! overridden. A real sebas crash-panic would leave the scene dir behind for
//! debugging (path printed on drop).
//!
//! Turn completion: both drivers emit `Finished` (the generic ACP driver
//! used to drop the PromptRequest result and leave `acp:<slug>` sessions in
//! the working phase — fixed in sebas-acp/src/acp_driver; see archived
//! cover-core-channel-test-gaps B2.1/B2.2 notes and the agent-driver spec's
//! "Both drivers present the same vocabulary" scenario). Both journeys
//! therefore hard-assert the DONE phase after the reply text lands; the
//! bounded grace window just tolerates the reply→Done race.

use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

/// First prompt sent to the real agent: forces a reply that is trivially
/// attributable (the verbatim sentinel) without asserting model wording.
const PROMPT: &str = "用一句话介绍你自己，然后原样输出:SEBAS_E2E_OK";
const SENTINEL: &str = "SEBAS_E2E_OK";

/// Env-var auth for the claude CLI (no OAuth login needed): sebas's claude
/// driver injects `ANTHROPIC_BASE_URL` + `ANTHROPIC_AUTH_TOKEN` into the
/// subprocess when a provider is resolved — and a provider is only resolved
/// when one has been selected (default provider/model) BEFORE the agent
/// spawns. The journey sets it via `PUT /api/agent-defaults`.
const ANTHROPIC_TOKEN_ENV: &str = "SEBAS_E2E_ANTHROPIC_AUTH_TOKEN";
const ANTHROPIC_BASE_URL_ENV: &str = "SEBAS_E2E_ANTHROPIC_BASE_URL";
const ANTHROPIC_MODEL_ENV: &str = "SEBAS_E2E_ANTHROPIC_MODEL";
/// Defaults target BigModel's documented Anthropic-compatible endpoint
/// (claude-code-over-GLM setup); both are overridable per environment.
const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://open.bigmodel.cn/api/anthropic";
const DEFAULT_ANTHROPIC_MODEL: &str = "glm-4.6";

/// Real LLM latency budget for the first turn (10–120 s typical).
const TURN_BUDGET: Duration = Duration::from_secs(240);
/// Grace window for the DONE phase after the reply text has landed.
const DONE_GRACE: Duration = Duration::from_secs(30);
/// Poll cadence for real-LLM waits (2 s — no rapid-fire retries).
const POLL: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Sandbox scene: one throwaway dir under target/tests/ holding the config and
// every piece of sebas state. Same recipe as tests/support/mod.rs's Sandbox,
// with real agent CLIs instead of the fake-claude stub and a bare core owning
// an in-process webui (`--webui-port`) instead of the detached pair.
// ---------------------------------------------------------------------------

struct SceneDir {
    path: PathBuf,
    keep: AtomicBool,
}

static SCENE_SEQ: AtomicU64 = AtomicU64::new(0);

impl SceneDir {
    fn new(sub: &str) -> Arc<Self> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let seq = SCENE_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = manifest
            .join("target")
            .join("tests")
            .join("sebas")
            .join("real_agent_e2e")
            .join(format!("{stamp:x}-{seq}-{sub}"));
        std::fs::create_dir_all(&path)
            .unwrap_or_else(|e| panic!("create scene dir {}: {e}", path.display()));
        Arc::new(Self {
            path,
            keep: AtomicBool::new(false),
        })
    }
}

impl Drop for SceneDir {
    fn drop(&mut self) {
        if self.keep.load(Ordering::Relaxed) || std::thread::panicking() {
            eprintln!(
                "[scene] kept for diagnosis (core.log inside): {}",
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

struct Scene {
    path: PathBuf,
    config_path: PathBuf,
    webui_port: u16,
    core_log: PathBuf,
    router_log: PathBuf,
    _dir: Arc<SceneDir>,
}

/// Forward-slash form for paths embedded in TOML / env values.
fn fs_string(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Probed free high port, never colliding with the operator's real instance
/// (9797) or the repo's established sandbox/test ports.
fn free_webui_port() -> u16 {
    const RESERVED: [u16; 4] = [9797, 9877, 9897, 9899];
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("probe free port")
            .local_addr()
            .expect("local addr")
            .port();
        if !RESERVED.contains(&port) {
            return port;
        }
    }
}

impl Scene {
    fn new(sub: &str) -> Self {
        let dir = SceneDir::new(sub);
        let path = dir.path.clone();
        let mkdir = |d: &Path| {
            std::fs::create_dir_all(d).unwrap_or_else(|e| panic!("mkdir {}: {e}", d.display()))
        };
        mkdir(&path.join("work"));
        mkdir(&path.join("claude-sessions"));
        mkdir(&path.join("downloads"));
        // XDG_RUNTIME_DIR pin: control-plane sockets must never land in the
        // host's real runtime dir.
        mkdir(&path.join("xdg-run"));
        // The project dir both journeys register and bind their session to.
        mkdir(&path.join("project"));

        let webui_port = free_webui_port();
        let config_path = path.join("config.toml");
        let core_log = path.join("core.log");
        let router_log = path.join("router.log");
        // Router listen：默认 8787 是固定值，可能撞上操作员实例的托管
        // router——钉一个 probed 空闲端口（unify-router-process-shape）。
        let router_port = free_webui_port();

        let toml = format!(
            r#"[feishu]
enabled = false

# Real claude CLI: the Claude Agent SDK spawns it; auth comes from claude's
# own login state under $HOME (NOT sandboxed, explicitly authorized).
[acp.agents.claude]
driver = "claude"
path = "claude"
sessions_dir = "{claude_sessions}"
work_dir = "{work}"

# opencode's native Agent Client Protocol server mode.
[acp.agents.opencode]
driver = "acp"
command = ["opencode", "acp"]

[dispatch]
state_file = "{state_file}"

[media]
download_dir = "{downloads}"

# RELATIVE path: every sandbox child runs with cwd = scene dir, so the unix
# socket never depends on the checkout depth (sun_path caps paths at 108 B).
[watchdog.core]
channel_path = "core-channel.sock"

# The bare core owns the webui via --webui-port (spawn form below).
[watchdog.webui]
enabled = false

# Router validate requires >=1 provider with a base_url; never dialed here
# (ACP sessions bypass the router entirely). Custom (non-preset) name:
# preset names like `anthropic` forbid explicit base_url slots.
[provider.e2e-dummy]
api_key = "sk-real-agent-e2e-dummy"
base_url_anthropic = "https://api.anthropic.com"
{e2e_claude_provider}
[router]
listen = "127.0.0.1:{router_port}"
provider_overlay = "{overlay}"
usage_file = "{usage}"
"#,
            claude_sessions = fs_string(&path.join("claude-sessions")),
            work = fs_string(&path.join("work")),
            state_file = fs_string(&path.join("sessions.json")),
            downloads = fs_string(&path.join("downloads")),
            overlay = fs_string(&path.join("providers.json")),
            usage = fs_string(&path.join("router-usage.jsonl")),
            // Written ONLY when the token env var is set: router validate
            // rejects an api_key_env pointing at an unset variable, which
            // would kill the core at startup even though this provider is
            // never dialed by the opencode journey.
            e2e_claude_provider = e2e_claude_provider_stanza(),
        );
        std::fs::write(&config_path, &toml)
            .unwrap_or_else(|e| panic!("write config {}: {e}", config_path.display()));

        Self {
            path,
            config_path,
            webui_port,
            core_log,
            router_log,
            _dir: dir,
        }
    }

    /// Env overrides for every default that would otherwise fall back to the
    /// operator's real `~/.sebas` (AGENTS.md sandbox rule 1). HOME is
    /// deliberately inherited: the agent CLIs need their own auth under
    /// `$HOME`; only sebas's own state is sandboxed.
    fn envs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("SEBAS_STATE_DB", fs_string(&self.path.join("sebas.db"))),
            ("SEBAS_STATE_FILE", fs_string(&self.path.join("state.json"))),
            (
                "SEBAS_ROUTER_PROVIDER_OVERLAY",
                fs_string(&self.path.join("providers.json")),
            ),
            ("SEBAS_CORE_SECRET", "real-agent-e2e-secret".to_string()),
            // Router admin-plane auth (webui RouterClient bearer + embedded
            // router check): PUT /api/agent-defaults and sibling mutations
            // 503 without it.
            (
                "SEBAS_CONTROL_SECRET",
                "real-agent-e2e-control-secret".to_string(),
            ),
            ("XDG_RUNTIME_DIR", fs_string(&self.path.join("xdg-run"))),
            ("NO_COLOR", "1".to_string()),
        ]
    }

    /// Extra vars forwarded verbatim when set: `[provider.e2e-claude]`
    /// resolves its auth token from the core's env (`api_key_env`), so the
    /// var must reach the core child for the claude env-token path.
    fn passthrough_envs(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::new();
        if let Ok(tok) = std::env::var(ANTHROPIC_TOKEN_ENV) {
            v.push((ANTHROPIC_TOKEN_ENV, tok));
        }
        v
    }

    /// Bare core owning an in-process webui (AGENTS.md sandbox recipe,
    /// unify-router-process-shape 两进程形态):
    /// `core -c <config> --webui --webui-port <free>`（无 router 旗标——
    /// router 由 [`Scene::spawn_router`] 以独立进程拉起）,
    /// cwd = scene dir so the relative channel socket lands inside it.
    fn spawn_core(&self) -> tokio::process::Child {
        let log = self.open_log(&self.core_log);
        let log_err = log.try_clone().expect("clone core log handle");
        tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"))
            .args([
                "core",
                "-c",
                &fs_string(&self.config_path),
                "--webui",
                "--webui-port",
                &self.webui_port.to_string(),
            ])
            .current_dir(&self.path)
            .envs(self.envs())
            .envs(self.passthrough_envs())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true)
            .spawn()
            .unwrap_or_else(|e| panic!("spawn sebas core: {e}"))
    }

    /// Router 独立子进程：`sebas router -c <config> --debug`（与 core 同一
    /// 沙箱 env；ACP 会话不经过它，拉起只为对齐 AGENTS.md 两进程菜谱）。
    fn spawn_router(&self) -> tokio::process::Child {
        let log = self.open_log(&self.router_log);
        let log_err = log.try_clone().expect("clone router log handle");
        tokio::process::Command::new(env!("CARGO_BIN_EXE_sebas"))
            .args([
                "router",
                "-c",
                &fs_string(&self.config_path),
                "--debug",
            ])
            .current_dir(&self.path)
            .envs(self.envs())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true)
            .spawn()
            .unwrap_or_else(|e| panic!("spawn sebas router: {e}"))
    }

    fn open_log(&self, path: &Path) -> std::fs::File {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|e| panic!("open log {}: {e}", path.display()))
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.webui_port)
    }
}

// ---------------------------------------------------------------------------
// HTTP helpers (same shapes as tests/support/mod.rs, self-contained).
// ---------------------------------------------------------------------------

/// Base URL for the e2e-claude provider (env override > BigModel default).
fn anthropic_base_url() -> String {
    std::env::var(ANTHROPIC_BASE_URL_ENV).unwrap_or_else(|_| DEFAULT_ANTHROPIC_BASE_URL.into())
}

/// Model id for the e2e-claude provider (env override > BigModel default).
fn anthropic_model() -> String {
    std::env::var(ANTHROPIC_MODEL_ENV).unwrap_or_else(|_| DEFAULT_ANTHROPIC_MODEL.into())
}

/// The `[provider.e2e-claude]` stanza, present only when the token env var is
/// set: router validate rejects an `api_key_env` pointing at an unset
/// variable, and an unconditional stanza would kill the core at startup even
/// though this provider is never dialed by the opencode journey. No secret
/// lands in the file — `api_key_env` names the variable, the core reads its
/// value from its own environment.
fn e2e_claude_provider_stanza() -> String {
    match std::env::var(ANTHROPIC_TOKEN_ENV) {
        Ok(tok) if !tok.is_empty() => format!(
            "\n# Env-token auth for the claude journey: api_key_env is read from the \
             CORE's environment at spawn-resolution time. `models` feeds the \
             agent-defaults catalog check.\n[provider.e2e-claude]\napi_key_env = \
             \"SEBAS_E2E_ANTHROPIC_AUTH_TOKEN\"\nbase_url_anthropic = \"{}\"\nmodels = \
             [\"{}\"]\n",
            anthropic_base_url(),
            anthropic_model()
        ),
        _ => String::new(),
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http client")
}

async fn post_json(
    cli: &reqwest::Client,
    url: &str,
    body: serde_json::Value,
) -> (u16, serde_json::Value) {
    let resp = cli
        .post(url)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    let status = resp.status().as_u16();
    let json = resp
        .json()
        .await
        .unwrap_or_else(|e| panic!("POST {url} body: {e}"));
    (status, json)
}

async fn get_detail(cli: &reqwest::Client, base: &str, key: &str) -> Option<serde_json::Value> {
    cli.get(format!("{base}/api/sessions/{key}"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()
}

/// Concatenated transcript text (the markdown/thinking/error blocks).
fn transcript_of(detail: &serde_json::Value) -> String {
    detail["body"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b["content"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn status_slug(detail: &serde_json::Value) -> &str {
    detail["status_slug"].as_str().unwrap_or_default()
}

async fn wait_webui_healthy(cli: &reqwest::Client, scene: &Scene) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let healthy = match cli.get(format!("{}/health", scene.url())).send().await {
            Ok(r) => r.text().await.ok().map(|b| b.trim() == "ok"),
            Err(_) => None,
        };
        if healthy == Some(true) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "webui /health never reported ok; logs at {}",
            scene.core_log.display()
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Poll the session transcript (2 s cadence) until the assistant reply
/// carries the sentinel; aborts early when the turn honestly failed.
/// Returns the concatenated transcript text.
async fn wait_first_reply(cli: &reqwest::Client, scene: &Scene, key: &str) -> String {
    let deadline = Instant::now() + TURN_BUDGET;
    loop {
        if let Some(detail) = get_detail(cli, &scene.url(), key).await {
            let transcript = transcript_of(&detail);
            if transcript.contains(SENTINEL) {
                return transcript;
            }
            if status_slug(&detail) == "failed" {
                panic!(
                    "real-agent turn FAILED; transcript so far: {transcript:?} (logs at {})",
                    scene.core_log.display()
                );
            }
        }
        assert!(
            Instant::now() < deadline,
            "timeout waiting for the real model reply ({SENTINEL}); logs at {}",
            scene.core_log.display()
        );
        tokio::time::sleep(POLL).await;
    }
}

/// Wait (2 s cadence, bounded) for the turn to reach the DONE phase.
async fn wait_done(cli: &reqwest::Client, scene: &Scene, key: &str, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(detail) = get_detail(cli, &scene.url(), key).await
            && status_slug(&detail) == "done"
        {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(POLL).await;
    }
}

// ---------------------------------------------------------------------------
// CLI preconditions (skip-with-reason, cross_uid `[skip]` early-return style).
// ---------------------------------------------------------------------------

/// True when `bin --version` runs — the CLI is installed and on PATH.
fn cli_on_path(bin: &str) -> bool {
    std::process::Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// `claude auth status` → `loggedIn`. The JSON on stdout is authoritative —
/// claude exits 1 when logged out — so the exit code is ignored. `None` when
/// the CLI is missing or the JSON shape changed (cannot determine → honest
/// skip).
fn claude_logged_in() -> Option<bool> {
    let out = std::process::Command::new("claude")
        .args(["auth", "status"])
        .output()
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(v["loggedIn"].as_bool() == Some(true))
}

// ---------------------------------------------------------------------------
// The journey.
// ---------------------------------------------------------------------------

/// 项目管理 → 会话管理 + agent 通信 → 会话管理 (close), against one real
/// backend. `agent` is the wire agent id（`[acp.agents.*]` 配置键名）：
/// `"claude"` 走专用 claude 驱动，`"opencode"` 走通用 ACP 驱动。
///
async fn first_message_roundtrip(agent: &str, sub: &str) {
    provider_selected_first_message_roundtrip(agent, sub, None).await;
}

/// `select_defaults` = Some((provider, model)) runs the "model first, agent
/// second" step: PUT /api/agent-defaults so the spawn resolves
/// `ProviderResolution::Direct` and the claude subprocess is armed with
/// `ANTHROPIC_*` env (no OAuth needed). None = spawn with the driver's own
/// default auth (Off resolution).
async fn provider_selected_first_message_roundtrip(
    agent: &str,
    sub: &str,
    select_defaults: Option<(&'static str, String)>,
) {
    let scene = Scene::new(sub);
    let cli = http_client();
    let mut core = scene.spawn_core();
    let _router = scene.spawn_router();
    wait_webui_healthy(&cli, &scene).await;

    // 项目管理: register a sandbox subdir as a project; assert it lists back.
    let project_dir = scene.path.join("project");
    // The API registers the canonical path; canonicalize on the test side so
    // the list-back assertion compares like with like.
    let canonical = std::fs::canonicalize(&project_dir).expect("canonicalize project dir");
    let (add_status, add_resp) = post_json(
        &cli,
        &format!("{}/api/projects", scene.url()),
        serde_json::json!({ "path": fs_string(&canonical) }),
    )
    .await;
    assert_eq!(add_status, 201, "register project: {add_resp}");
    let projects = cli
        .get(format!("{}/api/projects", scene.url()))
        .send()
        .await
        .expect("list projects")
        .json::<serde_json::Value>()
        .await
        .expect("projects json");
    assert!(
        projects.to_string().contains(&fs_string(&canonical)),
        "registered project must list back: {projects}"
    );

    // 会话管理 + agent 通信: first prompt → real agent → real model reply.
    // wire 是稳定项目 id（不是路径），注册响应即新条目（带回填的 id）。
    let project_id = add_resp["id"]
        .as_str()
        .expect("registered project entry carries an id");
    let (status, resp) = post_json(
        &cli,
        &format!("{}/api/sessions", scene.url()),
        serde_json::json!({
            "prompt": PROMPT,
            "agent": agent,
            "project_id": project_id,
        }),
    )
    .await;
    // "Model first, agent second": env-token auth only arms the claude
    // subprocess when a provider/model has been selected before the spawn.
    if let Some((provider, model)) = &select_defaults {
        let resp = cli
            .put(format!("{}/api/agent-defaults", scene.url()))
            .json(&serde_json::json!({ "provider": provider, "model": model }))
            .send()
            .await
            .unwrap_or_else(|e| panic!("PUT /api/agent-defaults: {e}"));
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        assert_eq!(status, 200, "set agent defaults: {body}");
        eprintln!("[real-agent] {agent}: defaults selected → {provider}/{model}");
    }

    assert_eq!(status, 201, "create session: {resp}");
    let key = resp["key"]
        .as_str()
        .expect("session key in create response")
        .to_string();
    assert!(!key.is_empty());

    let started = Instant::now();
    let transcript = wait_first_reply(&cli, &scene, &key).await;
    let latency = started.elapsed();
    assert!(
        transcript.contains(SENTINEL),
        "real model reply must carry the sentinel, got: {transcript:?}"
    );
    eprintln!(
        "[real-agent] {agent}: first model reply landed after {latency:?} (transcript {} chars)",
        transcript.len()
    );

    // Turn completion: Done is part of the same-vocabulary contract.
    if !wait_done(&cli, &scene, &key, DONE_GRACE).await {
        panic!(
            "{agent}: turn never reached Done within {DONE_GRACE:?} after the reply; \
             transcript: {transcript:?}"
        );
    }
    eprintln!("[real-agent] {agent}: turn reached Done");

    // 会话管理: close; assert accepted and the session leaves the active list.
    let close = cli
        .post(format!("{}/api/sessions/{key}/close", scene.url()))
        .send()
        .await
        .expect("close session");
    assert_eq!(close.status().as_u16(), 200, "close session");
    let close_body: serde_json::Value = close.json().await.expect("close body");
    assert_eq!(close_body["status"], "closed", "close body: {close_body}");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let gone = match cli
            .get(format!("{}/api/sessions/{key}", scene.url()))
            .send()
            .await
        {
            Ok(r) => r.status().as_u16() == 404,
            Err(_) => false,
        };
        let list = cli
            .get(format!("{}/api/sessions", scene.url()))
            .send()
            .await
            .expect("list sessions")
            .json::<serde_json::Value>()
            .await
            .expect("sessions json");
        if gone && !list.to_string().contains(&key) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "closed session must leave the active list (detail 404 + absent from /api/sessions)"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    core.kill().await.expect("kill core");
    // Dropping the scene removes the whole throwaway dir; any assertion
    // failure above keeps it (with core.log) via the Drop panicking() check.
}

/// opencode journey (#[ignore], opt-in): real `opencode acp` child, real
/// model reply. Skips honestly when the CLI is not installed.
#[cfg(unix)]
#[tokio::test]
#[ignore = "real-agent e2e: costs real LLM tokens (10-120s/turn); run with -- --ignored --test-threads=1 or invoke testsuite-real-agents"]
async fn real_opencode_first_message_roundtrip() {
    if !cli_on_path("opencode") {
        eprintln!(
            "[skip] real_opencode_first_message_roundtrip needs the opencode CLI on PATH \
             (authenticated via `opencode auth login`); journey ready"
        );
        return;
    }
    first_message_roundtrip("opencode", "opencode").await;
}

/// claude journey (#[ignore], opt-in): real `claude` CLI via the dedicated
/// Claude driver. Two auth paths, checked in order:
/// 1. env-token (`SEBAS_E2E_ANTHROPIC_AUTH_TOKEN` set): the journey selects
///    the `e2e-claude` provider as default (model first, agent second) and
///    sebas arms the claude subprocess with `ANTHROPIC_BASE_URL` +
///    `ANTHROPIC_AUTH_TOKEN` — no claude login needed.
/// 2. OAuth: claude's own login state under `$HOME`.
/// Neither available → skip with the exact remedy.
#[cfg(unix)]
#[tokio::test]
#[ignore = "real-agent e2e: costs real LLM tokens (10-120s/turn); run with -- --ignored --test-threads=1 or invoke testsuite-real-agents"]
async fn real_claude_first_message_roundtrip() {
    if !cli_on_path("claude") {
        eprintln!(
            "[skip] real_claude_first_message_roundtrip: claude CLI not on PATH — \
             install Claude Code; journey ready"
        );
        return;
    }
    let env_token = std::env::var(ANTHROPIC_TOKEN_ENV)
        .ok()
        .filter(|s| !s.is_empty());
    match (env_token.is_some(), claude_logged_in()) {
        (true, _) => {
            eprintln!(
                "[real-agent] claude: env-token auth via {ANTHROPIC_TOKEN_ENV} — \
                 selecting provider/model before spawn"
            );
            provider_selected_first_message_roundtrip(
                "claude",
                "claude",
                Some(("e2e-claude", anthropic_model())),
            )
            .await
        }
        (false, Some(true)) => first_message_roundtrip("claude", "claude").await,
        (false, Some(false)) => {
            eprintln!(
                "[skip] real_claude_first_message_roundtrip: claude CLI not logged in — \
                 either set {ANTHROPIC_TOKEN_ENV} (any Anthropic-compatible token; \
                 optional {ANTHROPIC_BASE_URL_ENV}, default {DEFAULT_ANTHROPIC_BASE_URL}; \
                 optional {ANTHROPIC_MODEL_ENV}, default {DEFAULT_ANTHROPIC_MODEL}) \
                 or run `claude` → /login; journey ready (zero code changes needed)"
            );
        }
        (false, None) => {
            eprintln!(
                "[skip] real_claude_first_message_roundtrip: cannot determine claude login \
                 state (`claude auth status` failed or unexpected JSON); journey ready"
            );
        }
    }
}
