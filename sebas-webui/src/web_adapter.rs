//! The `web` channel adapter (decouple-feishu-channel task 5.1, design D7).
//!
//! The WebUI is the core's first non-IM channel: its sessions live under
//! `ChannelKey { channel: "web", ... }` alongside feishu's, and its session
//! surface is the [`crate::session_backend::SessionBackend`] seam (reads via
//! snapshot/events, writes via the drive methods) — that seam IS the web
//! channel's client contract. What this module adds is the adapter face the
//! core's [`AdapterRegistry`] registers, so the registry answers "which
//! channels are active" with `web` always present and future channels plug
//! in as peers.
//!
//! Honest semantics, by design:
//! - `spawn` starts no transport: the web channel's inbound path is the
//!   WebUI's HTTP API (axum routes → `SessionBackend`), not a push socket.
//! - `render` returns `Ok(None)`: web sessions have no persistent channel
//!   card — the transcript (turns) is the presentation.

use sebas_channels::adapter::{CardRef, ChannelAdapter};
use sebas_channels::card::ChannelCard;
use sebas_channels::event::ChannelEvent;
use sebas_channels::key::{ChannelKey, ChannelName};

/// The always-registered `web` channel adapter.
#[derive(Debug, Clone, Copy, Default)]
pub struct WebAdapter;

impl ChannelAdapter for WebAdapter {
    fn channel_name(&self) -> ChannelName {
        ChannelName::new(ChannelName::WEB)
    }

    fn spawn(
        &self,
        _inbound: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // No transport to own: inbound web traffic arrives over the WebUI's
        // HTTP API and reaches the router through SessionBackend.
        Ok(())
    }

    fn shutdown(&self) {}

    fn render(
        &self,
        _key: &ChannelKey,
        _card_ref: Option<&CardRef>,
        _card: &ChannelCard,
    ) -> Result<Option<CardRef>, Box<dyn std::error::Error + Send + Sync>> {
        // Web sessions render through the transcript API, not a channel card.
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_adapter_registers_and_answers_honestly() {
        let mut registry = sebas_channels::AdapterRegistry::new();
        registry.register(Box::new(WebAdapter));
        let web = ChannelName::new(ChannelName::WEB);
        assert!(registry.contains(&web));
        // Render is a documented no-op: web sessions have no channel card.
        let key = ChannelKey::web_new();
        let rendered = registry
            .get(&web)
            .unwrap()
            .render(&key, None, &ChannelCard::new("t", "blue"));
        assert!(matches!(rendered, Ok(None)));
    }
}
