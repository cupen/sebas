//! Phase 3 Task 3.1: Feishu control commands route to watchdog Out events.
//! `/system`, `/gateway on|off|restart|status`, `/webui status` are parsed
//! into `Command::{System, Gateway, Webui}` and the router emits the matching
//! watchdog Out event — the core then proxies it over the private control RPC.
//! Control commands do not require an active session mapping (unlike core
//! commands), matching spec §12.

use feishu::events::{FeishuIn, SessionKey};
use router::commands::GatewayAction;
use router::router::{Out, RouterHandle};
use router::state::SessionMap;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(2);

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_control".into(),
        thread_id: None,
    }
}

async fn next_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    tokio::time::timeout(WAIT, rx.recv())
        .await
        .expect("out within timeout")
        .expect("channel open")
}

async fn dispatch_text(router: &RouterHandle, text: &str) {
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: text.into(),
            reply_to: None,
        })
        .await;
}

/// `/system` routes to `Out::WatchdogSystem` without requiring an active
/// session (watchdog status is available even with no mapping).
#[tokio::test]
async fn system_command_routes_to_watchdog_system() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    dispatch_text(&router, "/system").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogSystem { ref key } if key.chat_id == "oc_control"),
        "expected WatchdogSystem, got {out:?}"
    );
}

#[tokio::test]
async fn gateway_on_routes_to_watchdog_gateway() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    dispatch_text(&router, "/gateway on").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogGateway { ref key, ref action } if key.chat_id == "oc_control" && matches!(action, GatewayAction::On)),
        "expected WatchdogGateway(on), got {out:?}"
    );
}

#[tokio::test]
async fn gateway_restart_routes_to_watchdog_gateway() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    dispatch_text(&router, "/gateway restart").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogGateway { ref action, .. } if matches!(action, GatewayAction::Restart)),
        "expected WatchdogGateway(restart), got {out:?}"
    );
}

#[tokio::test]
async fn webui_status_routes_to_watchdog_webui() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    dispatch_text(&router, "/webui status").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogWebui { ref key } if key.chat_id == "oc_control"),
        "expected WatchdogWebui, got {out:?}"
    );
}

/// A non-control slash command with an unmapped key must not route to a
/// watchdog event — it stays a core routing decision (spawn/passthrough),
/// proving the control split keeps core commands on the core path.
#[tokio::test]
async fn core_command_does_not_route_to_watchdog() {
    let (router, mut out_rx) = RouterHandle::new(SessionMap::new());
    dispatch_text(&router, "/provider").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        !matches!(out, Out::WatchdogSystem { .. } | Out::WatchdogGateway { .. } | Out::WatchdogWebui { .. }),
        "core command must not emit watchdog Out, got {out:?}"
    );
}
