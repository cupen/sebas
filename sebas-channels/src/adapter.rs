//! The adapter seam (design D5): [`ChannelAdapter`] is the one trait the core
//! depends on to talk to any concrete channel; [`AdapterRegistry`] maps channel
//! names to their adapters and answers health queries. The core never reaches
//! into an adapter's internals — it feeds inbound events through on a channel
//! and receives outbound presentation through the render side.

use crate::card::ChannelCard;
use crate::event::ChannelEvent;
use crate::key::{ChannelKey, ChannelName};
use std::collections::BTreeMap;

/// An opaque, channel-specific handle to one outbound presentation instance
/// (feishu: the card's `message_id`; web: a presentation instance id).
#[derive(Debug, Clone)]
pub struct CardRef(pub String);

/// A channel's adapter: owns the channel's transport lifecycle and the
/// translation between the neutral model and the channel's wire shapes.
///
/// Implementations are channel-specific by construction; the core only uses
/// the trait's surface.
pub trait ChannelAdapter: Send + Sync {
    /// The channel this adapter serves (`"feishu"`, `"web"`, ...).
    fn channel_name(&self) -> ChannelName;

    /// Start the adapter's transport (feishu: the WebSocket loop). Inbound
    /// events are delivered to the core through `inbound`.
    fn spawn(
        &self,
        inbound: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Stop the adapter's transport.
    fn shutdown(&self);

    /// Render one outbound presentation instance for a session. `card_ref`
    /// selects an existing instance to update in place when `Some`; `None`
    /// starts a fresh one. Returns the resulting card reference.
    fn render(
        &self,
        key: &ChannelKey,
        card_ref: Option<&CardRef>,
        card: &ChannelCard,
    ) -> Result<Option<CardRef>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Name → adapter mapping. Registered once at core startup from config;
/// remains fixed for the process lifetime.
#[derive(Default)]
pub struct AdapterRegistry {
    adapters: BTreeMap<ChannelName, Box<dyn ChannelAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an adapter. A second registration under the same channel name
    /// replaces the previous one.
    pub fn register(&mut self, adapter: Box<dyn ChannelAdapter>) {
        let name = adapter.channel_name();
        self.adapters.insert(name, adapter);
    }

    pub fn get(&self, name: &ChannelName) -> Option<&dyn ChannelAdapter> {
        self.adapters.get(name).map(|a| a.as_ref())
    }

    pub fn contains(&self, name: &ChannelName) -> bool {
        self.adapters.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &ChannelName> {
        self.adapters.keys()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeAdapter {
        name: ChannelName,
        spawns: AtomicUsize,
    }

    impl ChannelAdapter for FakeAdapter {
        fn channel_name(&self) -> ChannelName {
            self.name.clone()
        }
        fn spawn(
            &self,
            _inbound: tokio::sync::mpsc::Sender<ChannelEvent>,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn shutdown(&self) {}
        fn render(
            &self,
            _key: &ChannelKey,
            _card_ref: Option<&CardRef>,
            _card: &ChannelCard,
        ) -> Result<Option<CardRef>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Some(CardRef("fake-1".into())))
        }
    }

    #[test]
    fn registry_maps_names_to_adapters() {
        let mut reg = AdapterRegistry::new();
        let feishu = ChannelName::new("feishu");
        let web = ChannelName::new("web");
        reg.register(Box::new(FakeAdapter {
            name: feishu.clone(),
            spawns: AtomicUsize::new(0),
        }));
        assert!(reg.contains(&feishu));
        assert!(!reg.contains(&web));
        assert!(reg.get(&feishu).is_some());
        assert!(reg.get(&web).is_none());
        assert_eq!(reg.names().collect::<Vec<_>>(), vec![&feishu]);
    }
}