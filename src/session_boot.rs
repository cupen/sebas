//! ACP 会话生命周期：spawn/resume/restore、事件泵、排队 prompt 冲刷。
//!
//! 从 `run.rs` 拆出（原文件 922 行混了编排/分发/会话/WS 四个职责）。
//! 对外的稳定入口经 `crate::run` 的 re-export 暴露，integration tests 路径不变。
//!
//! **与飞书解耦（本模块的边界契约）**：本文件的每个函数只依赖 ACP
//! manager + router——不 import 任何 `sebas_feishu` 类型，不持 HTTP 客户端。
//! 飞书侧的呈现（种子卡发送 / root message_id 记录 / 初始 reaction）是
//! dispatch 层的编排职责（见 `crate::dispatch::seed_and_send_root_card`）：
//! ACP 生命周期 → router 状态，飞书卡片 → Out 指令副作用，两条线在此分离。

use crate::spawn_env::resolve_spawn_overrides;
use sebas_acp::claude::ClaudeCodeDriver;
use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use sebas_channels::ChannelKey;
use sebas_router::config::RouterConfig;
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// 触发 abort 的 env var 名（openspec/specs/provider-management/spec.md）。
///
/// 当 [`crate::spawn_env::compute_provider_resolution`] 检测到配置错误
/// （缺失 provider / `api_key_env` 未设 / router.listen 空等），driver
/// 会把单一 env var `SEBAS_PROVIDER_ERROR=<reason>` 注入 `extra_env`。
/// spawn wrapper 看到这条 var 就立刻 `eprintln!` + `exit(1)`，不真的去
/// fork `claude` 子进程。这是把「silent Off fallback」升级为「in-band
/// error signal」的关键链路：用户能从 sebas stderr 直接看到原因，而不是
/// 看到 claude 启动了但啥都没发生（然后猜是 sebas / claude / 网络哪个环节）。
pub(crate) const SEBAS_PROVIDER_ERROR_ENV: &str = "SEBAS_PROVIDER_ERROR";

/// Pull the openspec/specs/provider-management/spec.md abort reason out of an `extra_env` list,
/// if present. Pure function, easy to unit-test. Returns `None` when the
/// signal is absent (the common case).
pub(crate) fn extract_provider_error(extra_env: &[(String, String)]) -> Option<&str> {
    extra_env
        .iter()
        .find(|(k, _)| k == SEBAS_PROVIDER_ERROR_ENV)
        .map(|(_, v)| v.as_str())
}

/// Check `extra_env` for the openspec/specs/provider-management/spec.md abort signal. If the
/// signal is present, print the reason to stderr and `std::process::exit(1)`.
///
/// `pub(crate)` so the spawn wrapper can call it directly; integration
/// tests bypass this layer (they pass already-empty env) so they don't
/// trigger the abort path by accident.
///
/// Why `std::process::exit` and not returning `Err`: the parent daemon
/// has no way to surface the reason to the Feishu card cleanly mid-spawn
/// (we haven't even recorded the `session_id` yet), and the user-visible
/// failure mode of "claude started but produced nothing" is exactly what
/// we're trying to eliminate. A noisy non-zero exit with a stderr line is
/// strictly better than the silent fallback we're replacing.
pub(crate) fn abort_if_provider_error(extra_env: &[(String, String)]) {
    if let Some(reason) = extract_provider_error(extra_env) {
        eprintln!(
            "sebas: provider configuration error; refusing to launch claude.\n  reason: {reason}"
        );
        // exit code 1: 与 `sebas` 其它「配置错不启动」路径一致；reason 已
        // 打到 stderr，daemon log 同步能看到。
        std::process::exit(1);
    }
}

/// Compute (extra_env, full command) from provider state + router config.
/// Centralizes the spawn-time translation. The provider override only applies
/// to the dedicated Claude driver (it injects `ANTHROPIC_BASE_URL` /
/// `OPENAI_API_KEY`); native-ACP agents manage their own provider and get an
/// untouched command. Pure: same input → same output.
///
/// Before returning, runs [`abort_if_provider_error`] — if `extra_env`
/// carries the in-band error signal, the process exits here and never
/// returns. The spawn call site therefore doesn't need to re-check.
fn spawn_overrides(
    kind: &str,
    command: Vec<String>,
    router_cfg: Option<&RouterConfig>,
) -> (Vec<(String, String)>, Vec<String>) {
    if kind != "claude" {
        return (Vec::new(), command);
    }
    let state = sebas_dispatch::provider_state::load();
    let driver = ClaudeCodeDriver;
    let (extra_env, extra_args) = resolve_spawn_overrides(&driver, &state, router_cfg);
    abort_if_provider_error(&extra_env);
    let mut full = command;
    full.extend(extra_args);
    (extra_env, full)
}

/// Create the ACP session, send the initial prompt, and flip the router's
/// Spawning placeholder to Active (draining queued prompts). No Feishu side
/// effects, no event pump, no pending flush — the caller sequences those so
/// the root card can go out before the pump starts. Returns a clone of the
/// session's event receiver taken BEFORE the initial prompt is sent, so a
/// crash-on-first-prompt terminal event survives the wrapper's eager table
/// removal (D6). `pub` for integration tests (tests/spawn_race_test.rs);
/// not part of the stable API.
///
/// `router_cfg` is the parsed `[router]` config from config.toml (or
/// `None` when the daemon runs without one). Used to resolve
/// `ProviderMode::Router` → URL + auth token; ignored for Off / Direct.
///
/// `model`（add-acp-model-selection D3）：创建时请求的模型 id。`Some` 时在
/// 会话建立后、首 prompt 前经 `SetModel` 应用（失败报非致命错误、会话仍
/// 可对话）；`None` = 用 agent 默认模型。
///
/// 返回 `(session_id, pending, rx, model_info)`：`model_info` 是 spawn
/// outcome 携带的模型选择面（agent 的 configOptions），调用方把它写入
/// session 映射供快照暴露。
#[allow(clippy::too_many_arguments)]
pub async fn acp_spawn_and_activate(
    mgr: &Arc<SessionManager>,
    router: &DispatchHandle,
    key: &ChannelKey,
    prompt: &str,
    kind: &str,
    command: Vec<String>,
    work_dir: Option<String>,
    router_cfg: Option<&RouterConfig>,
    model: Option<String>,
) -> anyhow::Result<(
    String,
    Vec<String>,
    std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<sebas_acp::claude::session::AcpEvent>>>,
    Option<sebas_acp::AcpModelInfo>,
)> {
    let (extra_env, full_command) = spawn_overrides(kind, command, router_cfg);
    let session_id = mgr
        .create_session(
            kind,
            full_command,
            work_dir,
            extra_env,
            prompt.to_string(),
        )
        .await?;
    // Clone the event receiver IMMEDIATELY after create_session returns Ok
    // (entry is in the table, session alive — before any slow I/O or prompt
    // send). This guarantees that if the agent crashes on the first prompt
    // (D6 target), the buffered terminal event survives the wrapper's eager
    // table removal — the dropped entry only releases the manager's Arc
    // clone, not this consumer's.
    let rx = mgr
        .event_rx(&session_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("no event_rx for freshly-created session"))?;
    // create-with-model（D3）：会话建立后、首 prompt 前应用请求的模型。失败
    // 不 abort 会话——记录 warn，模型保持 agent 默认（webui 报非致命错误）。
    if let Some(model_id) = model
        && let Err(e) = mgr.set_model(&session_id, &model_id).await
    {
        tracing::warn!(
            %session_id,
            model_id,
            error = %e,
            "create-with-model failed (non-fatal; session uses its default model)"
        );
    }
    mgr.send(
        &session_id,
        AcpCommand::CreateSession {
            session_id: session_id.clone(),
            prompt: prompt.to_string(),
        },
    )
    .await?;
    // Persist the driver-reported real ACP session id (native-ACP agents) on
    // the mapping so a later resume loads the conversation by the id the
    // agent actually knows (acp-session-mapping 场景 1). `None` (Claude) is
    // a no-op — the routing id is the conversation id there.
    let acp_session_id = mgr.get_acp_session_id(&session_id).await;
    // 模型选择面：spawn outcome 里 driver 上报的 configOptions 解析结果，
    // 一并写入映射供快照暴露（current_model + available_models）。
    let model_info = mgr.get_model_info(&session_id).await;
    let pending = router.activate(key, session_id.clone(), acp_session_id, model_info.clone()).await;
    Ok((session_id, pending, rx, model_info))
}

/// Resume variant of [`acp_spawn_and_activate`] (openspec/specs/session-lifecycle/spec.md): ask claude to
/// `resume` the persisted conversation id (the manager transparently falls
/// back to a fresh session when the id is rejected — sebas-dk8.4), then push
/// the triggering prompt as a continuation and flip the router's placeholder
/// to Active. The returned bool is `SpawnOutcome.resumed` — false means the
/// old conversation is gone and the id is a fresh one. The event receiver is
/// cloned IMMEDIATELY after the spawn returns, before any slow I/O (D6).
///
/// Provider-mode env/args apply on resume just as on fresh spawn — same
/// `router_cfg` and same runtime state produce the same overrides.
///
/// `model`（add-acp-model-selection）：`Some` 时在 resume（或 fallback-fresh）
/// 建立后、首 prompt 前经 `SetModel` 应用；`None` = 沿用 agent 会话设置的
/// 模型（resume 加载的会话自带其模型状态）。
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub async fn acp_resume_and_activate(
    mgr: &Arc<SessionManager>,
    router: &DispatchHandle,
    key: &ChannelKey,
    old_session_id: &str,
    prompt: &str,
    kind: &str,
    command: Vec<String>,
    work_dir: Option<String>,
    router_cfg: Option<&RouterConfig>,
    model: Option<String>,
) -> anyhow::Result<(
    String,
    Vec<String>,
    std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<sebas_acp::claude::session::AcpEvent>>>,
    bool,
)> {
    let (extra_env, full_command) = spawn_overrides(kind, command, router_cfg);
    // Resume by the agent's real ACP session id when the persisted mapping
    // for `key` has one (native-ACP agents like opencode address a
    // conversation by their OWN id, not sebas's routing uuid); `None` keeps
    // the legacy routing-id-as-load-target behavior for agents/records
    // without a distinct id. On a load rejection the manager's fresh
    // fallback proceeds and the returned outcome's `acp_session_id` carries
    // the NEW session's id under the NEW routing id.
    let acp_session_id = router.map.acp_session_id_for(key).await;
    let outcome = mgr
        .resume_session(
            kind,
            full_command,
            work_dir,
            extra_env,
            old_session_id,
            acp_session_id.clone(),
        )
        .await?;
    let session_id = outcome.session_id.clone();
    // 诚实回退（load 被拒 / 无映射）：把原映射（old routing id ↔ 旧真实 id）
    // 归档为 dormant 记录，保留在存储里供未来 load 寻址（D4：不因一次失败
    // 而抹除）。成功 load 时路由 id 不变、映射原位更新，无需归档。
    let is_fallback = !outcome.resumed;
    if is_fallback {
        router
            .map
            .preserve_closed_mapping(old_session_id, acp_session_id)
            .await;
    }
    let rx = mgr
        .event_rx(&session_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("no event_rx for freshly-resumed session"))?;
    // create-with-model 语义同样适用于 resume（会话建立后、首 prompt 前）。
    if let Some(model_id) = model
        && let Err(e) = mgr.set_model(&session_id, &model_id).await
    {
        tracing::warn!(
            %session_id,
            model_id,
            error = %e,
            "resume-with-model failed (non-fatal; session keeps its current model)"
        );
    }
    // The triggering prompt rides as a continuation: for a resumed session
    // it appends to the loaded conversation; for a fallback-fresh session
    // it is simply the first prompt (run_main drives both through
    // `send_prompt`).
    mgr.send(
        &session_id,
        AcpCommand::ContinueSession {
            session_id: session_id.clone(),
            prompt: prompt.to_string(),
        },
    )
    .await?;
    // Persist the (possibly NEW) real ACP session id: a successful load
    // re-records the loaded id under the same routing id; a fallback-fresh
    // session records the new session's id under the new routing id.
    let new_acp_session_id = mgr.get_acp_session_id(&session_id).await;
    let model_info = mgr.get_model_info(&session_id).await;
    let pending = router
        .activate(key, session_id.clone(), new_acp_session_id, model_info)
        .await;
    Ok((session_id, pending, rx, outcome.resumed))
}

/// Restore the session map from the state file (openspec/specs/session-lifecycle/spec.md):
/// - missing or empty file → empty table (first boot; an empty file is a
///   harmless leftover, not corruption);
/// - valid JSON → entries come back `Dormant`, eligible for lazy respawn;
/// - corrupt JSON → quarantine the file to `<path>.corrupt-<unix>` and
///   boot with an empty table instead of refusing to start.
///
/// `capacity` wires `[dispatch] max_concurrent_sessions` into the map.
pub fn restore_session_map(state_file: &str, capacity: usize) -> SessionMap {
    let state_raw = std::fs::read_to_string(state_file).unwrap_or_else(|_| "{}".into());
    match state_raw.trim() {
        "" => SessionMap::with_capacity(capacity),
        raw => match SessionMap::restore_json_with_capacity(raw, capacity) {
            Ok(m) => m,
            Err(e) => {
                let unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let quarantined = format!("{state_file}.corrupt-{unix}");
                warn!(
                    ?e,
                    path = %state_file,
                    "session state file is corrupt; quarantining and starting fresh"
                );
                if let Err(re) = std::fs::rename(state_file, &quarantined) {
                    warn!(?re, path = %quarantined, "failed to quarantine corrupt state file");
                }
                SessionMap::with_capacity(capacity)
            }
        },
    }
}

/// Spawn/resume 之后的 ACP 侧启动序列（与飞书无关）：
/// seed 卡片状态、启动事件泵、冲刷 spawn 窗口内排队的 prompt。
///
/// 这是 [`crate::dispatch`] 中 `SpawnAcp` / `SpawnResume` / `WebSpawn`
/// 共用的**引擎侧半场**（decouple-feishu-channel 后 spawn 指令与通道
/// 无关；发卡等通道呈现由各通道边界负责）。刻意不接收 `FeishuClient` / `TokenManager` /
/// `ReactionTracker` / `Config`：`idle_timeout` 由调用方从
/// `[acp.claude] idle_kill_secs` 解析后传入，保持本函数的输入只描述
/// ACP 会话本身。
///
/// 顺序契约（与拆分前逐条一致，无行为变化）：
/// 1. `seed_card`：记录 user_prompt 供后续 flush 重渲染引用块。幂等。
///    必须在 pump 启动前，否则首个事件 lazy seed 会用 prompt="" 冲掉引用块。
/// 2. 启动事件泵（`rx` 已在 `acp_spawn_and_activate` 里于任何慢 I/O 前
///    克隆，D6 保证首次即崩的终端事件不丢）。
/// 3. `flush_pending_prompts`：排队 prompt 合并为一次 ContinueSession
///    （逐条发送会违反 ACP 的 one-prompt-in-flight 规则）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn boot_session_pump_and_flush(
    router: &DispatchHandle,
    mgr: &Arc<SessionManager>,
    session_id: String,
    prompt: String,
    pending: Vec<String>,
    rx: std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<AcpEvent>>>,
    // sebas-9pz ②: idle_kill_secs 死配置接线 —— 由调用方解析；None =
    // 永不过期（生产默认 48h 只在显式配置非零值时启用）。
    idle_timeout: Option<Duration>,
) -> anyhow::Result<()> {
    router.seed_card(session_id.clone(), prompt).await;
    spawn_acp_pump_with_idle(rx, router.clone(), session_id.clone(), idle_timeout, Some(mgr.clone()));
    if let Err(e) = flush_pending_prompts(mgr, &session_id, pending).await {
        warn!(?e, "failed to flush pending prompts");
    }
    Ok(())
}

/// Flush prompts queued during spawn as ONE ContinueSession (one-by-one
/// would violate ACP's one-prompt-in-flight rule). `pub` for tests.
pub async fn flush_pending_prompts(
    mgr: &Arc<SessionManager>,
    session_id: &str,
    pending: Vec<String>,
) -> anyhow::Result<()> {
    if pending.is_empty() {
        return Ok(());
    }
    mgr.send(
        session_id,
        AcpCommand::ContinueSession {
            session_id: session_id.to_string(),
            prompt: pending.join("\n"),
        },
    )
    .await
}

/// Drain ACP events for one session, accumulating them into CardState and
/// flushing a single UpdateCard at most once per 150 ms (openspec/specs/feishu-cards/spec.md 节流契约)。
///
/// - 流式事件（TextDelta/ThinkingDelta/ToolStart/ToolProgress/ToolEnd/非
///   terminal Error）: `apply_event`（状态）+ 标脏；interval tick 到点若脏
///   则 `flush_card`。FSM 转移出的 reaction 随 flush 一起发（`pending_react`），
///   保持「先出卡、后换 reaction」的顺序。
/// - Finished / terminal Error / PermissionRequest: 即时 `apply_event_to_out`
///   （terminal 额外 remove_by_session + drop_card 后泵退出）。该路径自带
///   即时 React，故丢弃未发的 `pending_react`（避免 ✅/❌ 之后又补一个 🚧）。
/// - 通道关闭（recv → None）: `drop_card` + 退出。
///
/// `rx` 在 `acp_spawn_and_activate` 里于任何慢 I/O 之前克隆，故即便 agent
/// 首次 prompt 即崩（D6）、wrapper 急切移除表项，终端事件仍能经此克隆抵达。
///
/// 机制选择（150ms debounce 节流契约见 openspec/specs/feishu-cards/spec.md，
/// 选型背景见 docs/design-history.md ADR-2）：用
/// `tokio::time::interval(150ms) + dirty bool`，而非 spec 建议的
/// `Option<Sleep> + select + pending()` —— 后者在 select 跨臂借用 `&mut`
/// 会冲突，interval + Copy bool 规避之，契约等价。
///
/// `pub` for integration tests (tests/full_e2e_test.rs)；经 `crate::run` re-export。
pub fn spawn_acp_pump(
    rx: std::sync::Arc<
        tokio::sync::Mutex<tokio::sync::mpsc::Receiver<sebas_acp::claude::session::AcpEvent>>,
    >,
    router: DispatchHandle,
    session_id: String,
) {
    spawn_acp_pump_with_idle(rx, router, session_id, None, None);
}

/// `spawn_acp_pump` + `[acp.claude] idle_kill_secs`（sebas-9pz ②）：在会话
/// **完全没有事件**超过 `idle_timeout` 时杀死子进程并撤卡。任意事件
/// （TextDelta/ToolStart/…/Finished/PermissionRequest）都会重置计时器。
///
/// - `idle_timeout == None` → 原行为（永不过期，生产默认 48h 死配置的
///   workaround：只在显式配置了非 48h 默认值时启用）。
/// - 超时时：先 `mgr.kill(session_id)`（SIGKILL 子进程 + 撤表项），再
///   `router.drop_card`（清 CardState 防无界增长），并附一条日志。
///
/// `mgr` 用于杀进程；`None` 时只撤卡不杀（供测试与旧调用点）。
pub fn spawn_acp_pump_with_idle(
    rx: std::sync::Arc<
        tokio::sync::Mutex<tokio::sync::mpsc::Receiver<sebas_acp::claude::session::AcpEvent>>,
    >,
    router: DispatchHandle,
    session_id: String,
    idle_timeout: Option<Duration>,
    mgr: Option<std::sync::Arc<SessionManager>>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(150));
        // 第一个 tick 立即触发（tokio interval 语义）；此时 dirty=false，是 no-op。
        let mut dirty = false;
        let mut pending_react: Option<&'static str> = None;
        let mut rx = rx.lock().await;
        // idle-kill 计时（sebas-9pz ②）：距最近一次事件的时间。初始为
        // 本次 pump 启动时刻，避免 spawn 后立刻被误杀。
        let mut last_activity = tokio::time::Instant::now();
        loop {
            let idle_deadline = idle_timeout.map(|t| last_activity + t);
            tokio::select! {
                maybe_evt = rx.recv() => {
                    let Some(evt) = maybe_evt else {
                        router.drop_card(&session_id).await;
                        break;
                    };
                    // 任意事件重置 idle 计时。
                    last_activity = tokio::time::Instant::now();
                    let is_terminal = matches!(evt, AcpEvent::Error { terminal: true, .. });
                    let is_immediate = matches!(
                        evt,
                        AcpEvent::Finished { .. }
                            | AcpEvent::Error { terminal: true, .. }
                            | AcpEvent::PermissionRequest { .. }
                    );
                    if is_immediate {
                        // 即时路径：取消待发 debounce，同步出最终态。
                        dirty = false;
                        pending_react = None;
                        router.apply_event_to_out(session_id.clone(), &evt).await;
                        if is_terminal {
                            break;
                        }
                    } else {
                        // 流式：只累积状态，标脏；FSM 转移（👀→🚧）的 reaction
                        // 记入 pending_react，随下次 flush 一起发。
                        if let Some(emoji) = router.apply_event(&session_id, &evt).await {
                            pending_react = Some(emoji);
                        }
                        dirty = true;
                    }
                }
                _ = ticker.tick() => {
                    if dirty {
                        dirty = false;
                        // 检查是否需要换卡（接近 80% 上限时自动发新消息）
                        if router.card_needs_rotation(&session_id).await {
                            router.rotate_card(&session_id).await;
                            // rotate_card 已发射最终 UpdateCard 和新 SendCard，
                            // 跳过本次 flush 避免与新卡 race。
                            // 下个 tick（150ms 后）会正常 flush 新卡。
                        } else {
                            router.flush_card(&session_id).await;
                        }
                    }
                    if let Some(emoji) = pending_react.take() {
                        router.emit_reaction(&session_id, emoji).await;
                    }
                    // idle-kill 检查（sebas-9pz ②）：距最近事件超过阈值。
                    // 一旦触发就 kill + 撤卡 + 退出，不会重复。
                    if let Some(deadline) = idle_deadline
                        && tokio::time::Instant::now() >= deadline
                    {
                        warn!(
                            %session_id,
                            timeout = ?idle_timeout,
                            "session idle beyond idle_kill_secs; killing child"
                        );
                        // 杀子进程（若有 mgr）+ 撤卡。
                        if let Some(mgr) = &mgr {
                            mgr.kill(&session_id).await;
                        }
                        router.drop_card(&session_id).await;
                        break;
                    }
                }
            }
        }
        debug!(%session_id, "acp event stream closed; pump exiting");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure-logic test: `extract_provider_error` must find the
    /// `SEBAS_PROVIDER_ERROR` key and return its value verbatim. No
    /// subprocess, no panic — just the env list parsing. The provider
    /// review's decision (docs/design-history.md ADR-5): "just unit-test
    /// the env-detection logic if spawning
    /// is too integration-heavy" — we go with the lighter approach.
    #[test]
    fn extract_provider_error_returns_reason_when_signal_present() {
        let env = vec![(
            "SEBAS_PROVIDER_ERROR".to_string(),
            "direct provider 'foo' not found".to_string(),
        )];
        assert_eq!(
            extract_provider_error(&env),
            Some("direct provider 'foo' not found"),
        );
    }

    #[test]
    fn extract_provider_error_returns_none_for_empty_env() {
        let env: Vec<(String, String)> = Vec::new();
        assert_eq!(extract_provider_error(&env), None);
    }

    #[test]
    fn extract_provider_error_returns_none_when_key_absent() {
        let env = vec![
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "https://api.example".to_string(),
            ),
            ("ANTHROPIC_AUTH_TOKEN".to_string(), "sk-test".to_string()),
        ];
        assert_eq!(extract_provider_error(&env), None);
    }

    #[test]
    fn extract_provider_error_returns_none_for_non_matching_signal() {
        // Defensive: if some other code path accidentally injects a
        // similar-looking key (`SEBAS_PROVIDER_ERROR_LOG` etc), we must
        // NOT treat it as the abort signal.
        let env = vec![(
            "SEBAS_PROVIDER_ERROR_LOG".to_string(),
            "not the real one".to_string(),
        )];
        assert_eq!(extract_provider_error(&env), None);
    }
}
