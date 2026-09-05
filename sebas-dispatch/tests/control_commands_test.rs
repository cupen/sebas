//! Phase 3 Task 3.1: Feishu control commands route to watchdog Out events.
//! `/system`, `/router on|off|restart|status`, `/webui status` are parsed
//! into `Command::{System, Router, Webui}` and the router emits the matching
//! watchdog Out event — the core then proxies it over the private control RPC.
//! Control commands do not require an active session mapping (unlike core
//! commands), matching the openspec/specs/router-commands/spec.md
//! control-commands contract.

use sebas_channels::{ChannelEvent, ChannelKey};
use sebas_dispatch::commands::RouterAction;
use sebas_dispatch::engine::{Out, DispatchHandle};
use sebas_dispatch::state::SessionMap;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(2);

fn key() -> ChannelKey {
    ChannelKey::feishu("oc_control", None)
}

async fn next_out(rx: &mut tokio::sync::mpsc::Receiver<Out>) -> Out {
    tokio::time::timeout(WAIT, rx.recv())
        .await
        .expect("out within timeout")
        .expect("channel open")
}

async fn dispatch_text(router: &DispatchHandle, text: &str) {
    router
        .dispatch(ChannelEvent::Text {
            key: key(),
            text: text.into(),
            reply_target: None,
        })
        .await;
}

/// `/system` routes to `Out::WatchdogSystem` without requiring an active
/// session (watchdog status is available even with no mapping).
#[tokio::test]
async fn system_command_routes_to_watchdog_system() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    dispatch_text(&router, "/system").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogSystem { ref key } if key.reference == "oc_control"),
        "expected WatchdogSystem, got {out:?}"
    );
}

#[tokio::test]
async fn router_on_routes_to_watchdog_router() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    dispatch_text(&router, "/router on").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogRouter { ref key, ref action } if key.reference == "oc_control" && matches!(action, RouterAction::On)),
        "expected WatchdogRouter(on), got {out:?}"
    );
}

#[tokio::test]
async fn router_restart_routes_to_watchdog_router() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    dispatch_text(&router, "/router restart").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogRouter { ref action, .. } if matches!(action, RouterAction::Restart)),
        "expected WatchdogRouter(restart), got {out:?}"
    );
}

#[tokio::test]
async fn webui_status_routes_to_watchdog_webui() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    dispatch_text(&router, "/webui status").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogWebui { ref key } if key.reference == "oc_control"),
        "expected WatchdogWebui, got {out:?}"
    );
}

/// `/confirm <token>` routes to `Out::WatchdogConfirm` without requiring an
/// active session (sebas-29s): confirmation redemption is a chat-level control
/// op, the dispatcher re-uses the same Feishu actor for the Confirm RPC.
#[tokio::test]
async fn confirm_command_routes_to_watchdog_confirm() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    dispatch_text(&router, "/confirm tok_abc123").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::WatchdogConfirm { ref key, ref token }
            if key.reference == "oc_control" && token == "tok_abc123"),
        "expected WatchdogConfirm, got {out:?}"
    );
}

/// Bare `/confirm` (no token) must not forward anywhere — it replies with
/// usage text instead of silently dropping or passing through to claude.
#[tokio::test]
async fn bare_confirm_replies_usage_text() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    dispatch_text(&router, "/confirm").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        matches!(out, Out::PlainText { ref content, .. } if content.contains("/confirm <token>")),
        "expected usage PlainText, got {out:?}"
    );
}

/// A non-control slash command with an unmapped key must not route to a
/// watchdog event — it stays a core routing decision (spawn/passthrough),
/// proving the control split keeps core commands on the core path.
#[tokio::test]
async fn core_command_does_not_route_to_watchdog() {
    let (router, mut out_rx) = DispatchHandle::new(SessionMap::new());
    dispatch_text(&router, "/provider").await;
    let out = next_out(&mut out_rx).await;
    assert!(
        !matches!(
            out,
            Out::WatchdogSystem { .. } | Out::WatchdogRouter { .. } | Out::WatchdogWebui { .. }
        ),
        "core command must not emit watchdog Out, got {out:?}"
    );
}
