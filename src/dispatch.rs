//! 出站分发：把 router 产出的 `Out` 指令落实为飞书/ACP 副作用。
//!
//! 从 `run.rs` 拆出；只被出站泵（`crate::run`）调用。

use crate::config::Config;
use crate::session_boot::{
    acp_resume_and_activate, acp_spawn_and_activate, flush_pending_prompts,
    spawn_acp_pump_with_idle,
};
use sebas_acp::claude::manager::SessionManager;
use sebas_channels::ChannelKey;
use sebas_router::config::RouterConfig;
use sebas_dispatch::engine::{Out, DispatchHandle};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

// 话题失效提示文案（Q8→F1 熔断）：发一次提示并终止会话，不重试、不重发。
// 群聊/p2p 通用，不提「开新话题」。

/// `[acp.agents.<kind>] idle_kill_secs` → 事件泵 idle 超时（sebas-9pz ②）。
/// 配置 > 0 时启用（生产默认 172800/48h 照常生效）；0 = 不过期。
fn idle_timeout_from(cfg: &Config, kind: &str) -> Option<Duration> {
    let secs = cfg.acp.idle_kill_for(kind);
    (secs > 0).then(|| Duration::from_secs(secs))
}

// 参数即 outbound 共享上下文（client/http/tokens/cfg/router/mgr/reactions），
// 打包 struct 只会给每个 match arm 增加 `ctx.` 噪音。
// `router_control_request` 在 tests 模块里——它只服务
// `router_actions_normalize_to_control_requests` 这条归一化契约测试。

pub(crate) async fn dispatch_out_without_feishu(
    cfg: &Config,
    router: &DispatchHandle,
    mgr: &Arc<SessionManager>,
    router_cfg: Option<&RouterConfig>,
    out: Out,
) -> anyhow::Result<()> {
    match out {
        Out::WebSpawn {
            key,
            prompt,
            project_dir,
            kind,
            model,
            mode,
        } => {
            handle_web_spawn(
                cfg,
                router,
                mgr,
                router_cfg,
                key,
                prompt,
                project_dir,
                kind,
                model,
                mode,
            )
            .await
        }
        Out::SpawnResume {
            key,
            session_id: old_sid,
            prompt,
            ..
        } => {
            handle_spawn_resume_without_feishu(cfg, router, mgr, router_cfg, key, old_sid, prompt)
                .await
        }
        Out::SendAcp { session_id, cmd } => Ok(mgr.send(&session_id, cmd).await?),
        other => {
            debug!(
                ?other,
                "feishu-less outbound pump dropped a chat-facing Out"
            );
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_web_spawn(
    cfg: &Config,
    router: &DispatchHandle,
    mgr: &Arc<SessionManager>,
    router_cfg: Option<&RouterConfig>,
    key: ChannelKey,
    prompt: String,
    project_dir: Option<String>,
    requested_kind: Option<String>,
    model: Option<String>,
    mode: Option<String>,
) -> anyhow::Result<()> {
    let kind = requested_kind.unwrap_or_else(|| cfg.acp.default_kind().to_string());
    let command = cfg.acp.command_for(&kind).unwrap_or_default();
    // （add-agent-mode-selection）控制面 mode → 子进程 argv：只有 claude
    // 驱动认识 `--permission-mode`；其它执行体接受请求但不生效（非致命，
    // 同 model 的既有语义）。argv 是模式的单一出处（driver 解析它初始化
    // 探针模式，fake-claude journal 也由此可断言）。
    let mut command = command;
    if kind == "claude"
        && let Some(flag) = mode.as_deref().and_then(mode_to_permission_flag)
    {
        command.push("--permission-mode".into());
        command.push(flag.into());
    }
    let (session_id, pending, rx, _model_info) = match acp_spawn_and_activate(
        mgr,
        router,
        &key,
        &prompt,
        &kind,
        command,
        project_dir.or_else(|| cfg.acp.work_dir_for(&kind)),
        router_cfg,
        model,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            // fail-fast-on-startup-errors 3.2：不再只 warn 吞错——失败原因
            // 立即推进 transcript（错误事件 + spawn-failed 状态 + Updated），
            // 触发者（webui 前端）当期可见，不延后到 Removed。
            let reason = format!("{e}");
            warn!(?e, "web_spawn: acp_spawn_and_activate failed");
            router.fail_spawn(&key, &reason).await;
            return Ok(());
        }
    };
    // （add-agent-mode-selection）spawn 成功且模式真的进了 argv → effective
    // = 请求的控制面词汇；其它执行体不声称任何 mode 生效（effective 保持
    // None，与 desired 的差异如实可见）。
    if kind == "claude"
        && mode.as_deref().map(mode_to_permission_flag).flatten().is_some()
    {
        router.apply_mode_changed(session_id.as_str(), mode.as_deref()).await;
    }
    // Seed card state and wire the pump (no Feishu card operations).
    router.seed_card(session_id.clone(), prompt.clone()).await;
    // sebas-9pz ②: idle_kill_secs 接线(与 Feishu 路径一致)。
    let idle_timeout = idle_timeout_from(cfg, &kind);
    spawn_acp_pump_with_idle(
        rx,
        router.clone(),
        session_id.clone(),
        idle_timeout,
        Some(mgr.clone()),
    );
    // Flush prompts queued during spawn.
    if let Err(e) = flush_pending_prompts(mgr, &session_id, pending).await {
        warn!(?e, "web_spawn: flush_pending_prompts failed");
    }
    Ok(())
}

/// （add-agent-mode-selection）控制面 mode 词汇 → CLI `--permission-mode`
/// 参数值的映射转发——单一出处是
/// [`sebas_acp::claude::control_mode_flag`]（driver 解析 argv 与运行时切换
/// 共用同一套约定）。
fn mode_to_permission_flag(mode: &str) -> Option<&'static str> {
    sebas_acp::claude::control_mode_flag(mode)
}

async fn handle_spawn_resume_without_feishu(
    cfg: &Config,
    router: &DispatchHandle,
    mgr: &Arc<SessionManager>,
    router_cfg: Option<&RouterConfig>,
    key: ChannelKey,
    old_sid: String,
    prompt: String,
) -> anyhow::Result<()> {
    let kind = cfg.acp.default_kind().to_string();
    let command = cfg.acp.command_for(&kind).unwrap_or_default();
    // （add-agent-mode-selection）resume 读取映射里的 desired mode：子进程
    // 是新建的，`--permission-mode` 必须随 argv 重新下发，否则恢复出来的
    // 会话回退到 CLI 默认（映射字段随 state.json 持久化）。
    let resume_mode = router.map.get(&key).await.and_then(|m| m.desired_mode.clone());
    let mut command = command;
    if kind == "claude"
        && let Some(flag) = resume_mode.as_deref().and_then(mode_to_permission_flag)
    {
        command.push("--permission-mode".into());
        command.push(flag.into());
    }
    let (session_id, pending, rx, resumed) = match acp_resume_and_activate(
        mgr,
        router,
        &key,
        &old_sid,
        &prompt,
        &kind,
        command,
        cfg.acp.work_dir_for(&kind),
        router_cfg,
        // webui resume 路径暂无模型参数（创建/中程切换走 POST model）。
        None,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            // 同 web_spawn：resume 失败也即时进 transcript，不再静默拆除。
            let reason = format!("resume failed: {e}");
            warn!(?e, %old_sid, "resume failed (feishu-less run; no card sent)");
            router.fail_spawn(&key, &reason).await;
            return Ok(());
        }
    };
    if !resumed {
        // session-lifecycle spec：rejected resume falls back to fresh AND the
        // user is informed the old conversation is gone。进 transcript（同款
        // fail_spawn/spawn-failed 模式），由 IM/webui 渲染，不只落日志。
        info!(%old_sid, %session_id, "old session could not be loaded; continued as fresh session");
        router.notify_resume_fell_back(&key, &session_id).await;
    }
    // （add-agent-mode-selection）resume 的子进程 argv 带上了 desired mode
    // 映射值 → effective 如实记录（claude 且确有映射时）。
    if kind == "claude"
        && resume_mode
            .as_deref()
            .map(mode_to_permission_flag)
            .flatten()
            .is_some()
    {
        router.apply_mode_changed(session_id.as_str(), resume_mode.as_deref()).await;
    }
    router.seed_card(session_id.clone(), prompt.clone()).await;
    let idle_timeout = idle_timeout_from(cfg, &kind);
    spawn_acp_pump_with_idle(
        rx,
        router.clone(),
        session_id.clone(),
        idle_timeout,
        Some(mgr.clone()),
    );
    if let Err(e) = flush_pending_prompts(mgr, &session_id, pending).await {
        warn!(?e, "resume: flush_pending_prompts failed");
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use sebas_dispatch::commands::RouterAction;

    /// `/router on|off` 归一化为 ServiceSet(router, persist=false)；
    /// `/router status` 归一化为 ServiceStatusFor(router)；
    /// `/router restart` 归一化为 ServiceRestart(router)。这是 WebUI/Feishu
    /// 共享的归一化契约（openspec/specs/watchdog/spec.md；跨 adapter 一致性
    /// 背景见 docs/design-history.md ADR-6）。
    fn router_control_request(
        action: RouterAction,
    ) -> crate::watchdog::control_rpc::RpcControlRequest {
        use crate::watchdog::control_rpc::RpcControlRequest;
        match action {
            RouterAction::On => RpcControlRequest::ServiceSet {
                service: "router".into(),
                desired: "on".into(),
                persist: false,
                force: false,
            },
            RouterAction::Off => RpcControlRequest::ServiceSet {
                service: "router".into(),
                desired: "off".into(),
                persist: false,
                force: false,
            },
            RouterAction::Status => RpcControlRequest::ServiceStatusFor {
                service: "router".into(),
            },
            RouterAction::Restart => RpcControlRequest::ServiceRestart {
                service: "router".into(),
            },
        }
    }

    #[test]
    fn router_actions_normalize_to_control_requests() {
        use crate::watchdog::control_rpc::RpcControlRequest;

        assert_eq!(
            router_control_request(RouterAction::On),
            RpcControlRequest::ServiceSet {
                service: "router".into(),
                desired: "on".into(),
                persist: false,
                force: false,
            }
        );
        assert_eq!(
            router_control_request(RouterAction::Off),
            RpcControlRequest::ServiceSet {
                service: "router".into(),
                desired: "off".into(),
                persist: false,
                force: false,
            }
        );
        assert_eq!(
            router_control_request(RouterAction::Status),
            RpcControlRequest::ServiceStatusFor {
                service: "router".into(),
            }
        );
        assert_eq!(
            router_control_request(RouterAction::Restart),
            RpcControlRequest::ServiceRestart {
                service: "router".into(),
            }
        );
    }
}
