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
) -> anyhow::Result<()> {
    let kind = requested_kind.unwrap_or_else(|| cfg.acp.default_kind().to_string());
    let command = cfg.acp.command_for(&kind).unwrap_or_default();
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
            router.fail_spawn(&key).await;
            warn!(?e, "web_spawn: acp_spawn_and_activate failed");
            return Ok(());
        }
    };
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
            router.fail_spawn(&key).await;
            warn!(?e, %old_sid, "resume failed (feishu-less run; no card sent)");
            return Ok(());
        }
    };
    if !resumed {
        info!(%old_sid, %session_id, "old session could not be loaded; continued as fresh session");
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
            },
            RouterAction::Off => RpcControlRequest::ServiceSet {
                service: "router".into(),
                desired: "off".into(),
                persist: false,
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
            }
        );
        assert_eq!(
            router_control_request(RouterAction::Off),
            RpcControlRequest::ServiceSet {
                service: "router".into(),
                desired: "off".into(),
                persist: false,
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
