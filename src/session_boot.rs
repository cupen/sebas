//! ACP 会话生命周期：spawn/resume/restore、卡片种子、事件泵、排队 prompt 冲刷。
//!
//! 从 `run.rs` 拆出（原文件 922 行混了编排/分发/会话/WS 四个职责）。
//! 对外的稳定入口经 `crate::run` 的 re-export 暴露，integration tests 路径不变。

use crate::config::Config;
use crate::dispatch::{TopicSendOutcome, send_card_topic_aware, topic_reply_target};
use crate::reactions::ReactionTracker;
use crate::spawn_env::resolve_spawn_overrides;
use sebas_acp::claude::ClaudeCodeDriver;
use sebas_acp::claude::manager::SessionManager;
use sebas_acp::claude::session::{AcpCommand, AcpEvent};
use sebas_feishu::cards::render_accumulated_card;
use sebas_feishu::client::FeishuClient;
use sebas_feishu::events::SessionKey;
use sebas_gateway::config::GatewayConfig;
use sebas_router::router::RouterHandle;
use sebas_router::state::SessionMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

/// 触发 abort 的 env var 名（openspec/specs/provider-management/spec.md）。
///
/// 当 [`crate::spawn_env::compute_provider_resolution`] 检测到配置错误
/// （缺失 provider / `api_key_env` 未设 / gateway.listen 空等），driver
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

/// Compute (extra_env, claude_args+extra_args) from provider state + gateway
/// config. Centralizes the spawn-time translation so spawn / resume /
/// spawn_test_session all see the same view. Pure: same input → same output.
///
/// Before returning, runs [`abort_if_provider_error`] — if `extra_env`
/// carries the in-band error signal, the process exits here and never
/// returns. The spawn call site therefore doesn't need to re-check.
fn spawn_overrides(
    claude_args: Vec<String>,
    gateway_cfg: Option<&GatewayConfig>,
) -> (Vec<(String, String)>, Vec<String>) {
    let state = sebas_router::provider_state::load();
    let driver = ClaudeCodeDriver;
    let (extra_env, extra_args) = resolve_spawn_overrides(&driver, &state, gateway_cfg);
    abort_if_provider_error(&extra_env);
    let mut full_args = claude_args;
    full_args.extend(extra_args);
    (extra_env, full_args)
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
/// `gateway_cfg` is the parsed `[gateway]` config from config.toml (or
/// `None` when the daemon runs without one). Used to resolve
/// `ProviderMode::Gateway` → URL + auth token; ignored for Off / Direct.
pub async fn acp_spawn_and_activate(
    mgr: &Arc<SessionManager>,
    router: &RouterHandle,
    key: &SessionKey,
    prompt: &str,
    claude_path: &str,
    claude_args: Vec<String>,
    work_dir: Option<String>,
    gateway_cfg: Option<&GatewayConfig>,
) -> anyhow::Result<(
    String,
    Vec<String>,
    std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<sebas_acp::claude::session::AcpEvent>>>,
)> {
    let (extra_env, full_args) = spawn_overrides(claude_args, gateway_cfg);
    let session_id = mgr
        .create_session(
            claude_path,
            full_args,
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
    mgr.send(
        &session_id,
        AcpCommand::CreateSession {
            session_id: session_id.clone(),
            prompt: prompt.to_string(),
        },
    )
    .await?;
    let pending = router.activate(key, session_id.clone()).await;
    Ok((session_id, pending, rx))
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
/// `gateway_cfg` and same runtime state produce the same overrides.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub async fn acp_resume_and_activate(
    mgr: &Arc<SessionManager>,
    router: &RouterHandle,
    key: &SessionKey,
    old_session_id: &str,
    prompt: &str,
    claude_path: &str,
    claude_args: Vec<String>,
    work_dir: Option<String>,
    gateway_cfg: Option<&GatewayConfig>,
) -> anyhow::Result<(
    String,
    Vec<String>,
    std::sync::Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<sebas_acp::claude::session::AcpEvent>>>,
    bool,
)> {
    let (extra_env, full_args) = spawn_overrides(claude_args, gateway_cfg);
    let outcome = mgr
        .resume_session(claude_path, full_args, work_dir, extra_env, old_session_id)
        .await?;
    let session_id = outcome.session_id.clone();
    let rx = mgr
        .event_rx(&session_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("no event_rx for freshly-resumed session"))?;
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
    let pending = router.activate(key, session_id.clone()).await;
    Ok((session_id, pending, rx, outcome.resumed))
}

/// Restore the session map from the state file (openspec/specs/session-lifecycle/spec.md):
/// - missing or empty file → empty table (first boot; an empty file is a
///   harmless leftover, not corruption);
/// - valid JSON → entries come back `Dormant`, eligible for lazy respawn;
/// - corrupt JSON → quarantine the file to `<path>.corrupt-<unix>` and
///   boot with an empty table instead of refusing to start.
///
/// `capacity` wires `[router] max_concurrent_sessions` into the map.
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

/// Shared post-spawn wiring for the SpawnAcp and SpawnResume dispatch arms:
/// seed the card state, send the root card, record its message_id, start the
/// event pump, and flush prompts queued during the spawn.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn wire_session_card_and_pump(
    feishu: &FeishuClient,
    http: &reqwest::Client,
    tokens: &sebas_feishu::client::TokenManager,
    cfg: &Config,
    router: &RouterHandle,
    mgr: &Arc<SessionManager>,
    reactions: &ReactionTracker,
    key: SessionKey,
    session_id: String,
    prompt: String,
    pending: Vec<String>,
    rx: std::sync::Arc<
        tokio::sync::Mutex<tokio::sync::mpsc::Receiver<sebas_acp::claude::session::AcpEvent>>,
    >,
    // Feishu message_id this session's root card should reply to (the user's
    // input message), so the card appears threaded under it for easy
    // tracking. `None` = standalone card (WebUI / no input message).
    input_msg_id: Option<String>,
) -> anyhow::Result<()> {
    // seed_card（openspec/specs/feishu-cards/spec.md）: 记录 user_prompt 供后续 flush 重渲染
    // 引用块。幂等。必须在 pump 启动前，否则首个事件 lazy seed
    // 会用 prompt="" 冲掉引用块。
    router.seed_card(session_id.clone(), prompt.clone()).await;
    // Send the seed card (empty body) and record its message_id keyed by the
    // real session_id (so streaming UpdateCards resolve correctly).
    // render_accumulated_card 用真实 theme，与后续 flush 产出的卡结构一致
    //（避免初始卡蓝、后续卡变色的跳变）。
    let card = render_accumulated_card(&prompt, &session_id, &[], &cfg.card.theme_color, None);
    // 话题会话：初始 root 卡回复到话题根消息（Q5），保证整轮对话聚合在
    // 原话题；主线回退到用户输入消息（main 的 input_msg_id 行为，卡片
    // 以 reply 形式挂在输入消息下，方便沿 thread 跟踪）。话题失效时
    // send_card_topic_aware 会发文本提示并熔断（web_close_session 终止
    // 刚 spawn 的会话，返回 TopicInvalid，不冒泡错误）—— 首次出站就失效
    // 更要终止。
    let reply = topic_reply_target(router, &key, None)
        .await
        .or(input_msg_id);
    let outcome = send_card_topic_aware(
        feishu,
        http,
        tokens,
        router,
        &key,
        serde_json::to_value(&card)?,
        reply,
    )
    .await?;
    if let TopicSendOutcome::Sent(msg_id) = outcome {
        router
            .record_root_msg_id(session_id.clone(), msg_id.clone())
            .await;
        // Stamp the initial reaction on the root card. emoji_type 是 Feishu
        // API 合法值（"Get" = 👌 已收到），不再是 "Typing" / unicode 👀 ——
        // "Typing" 暗示正在输入，"Get" 才契合"已收到"语义。Best-effort:
        // a reaction failure must not abort session creation.
        match feishu
            .react(http, tokens, &msg_id, sebas_router::card_state::phase::SEED)
            .await
        {
            Ok(rid) => {
                reactions
                    .record(&session_id, sebas_router::card_state::phase::SEED.into(), rid)
                    .await
            }
            Err(e) => warn!(%session_id, "initial react failed: {e}"),
        }
    }
    // Pump ACP events from this session back into the router.
    // `rx` was cloned before any slow I/O (the send_card HTTP round trip
    // above) so a crash-on-first-prompt terminal event survives the
    // wrapper's eager table removal (D6).
    //
    // sebas-9pz ②: idle_kill_secs 死配置接线 —— 配置 > 0 时,会话连续无事件
    // 超过该时长会被 kill(子进程) + drop_card。默认 172800(48h)照常生效。
    let idle_timeout = (cfg.acp.claude.idle_kill_secs > 0)
        .then(|| Duration::from_secs(cfg.acp.claude.idle_kill_secs));
    spawn_acp_pump_with_idle(
        rx,
        router.clone(),
        session_id.clone(),
        idle_timeout,
        Some(mgr.clone()),
    );
    // Flush queued prompts as ONE follow-up (sending them one by one would
    // violate ACP's one-prompt-in-flight rule).
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
    router: RouterHandle,
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
    router: RouterHandle,
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
