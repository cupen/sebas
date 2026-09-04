//! sebas-channels: the core's neutral channel abstraction
//! (openspec/changes/decouple-feishu-channel, capability `channels`).
//!
//! The core depends only on the types and the [`adapter::ChannelAdapter`]
//! trait defined here; concrete channels (feishu, web, future IM/agent
//! clients) implement the trait and register into an
//! [`adapter::AdapterRegistry`]. Channel-specific shapes — Feishu chat and
//! thread ids, message ids, card JSON, reactions — stay inside each adapter.
//!
//! Terminology: see `openspec/glossary.md`.

pub mod adapter;
pub mod card;
pub mod event;
pub mod key;

pub use adapter::{AdapterRegistry, CardRef, ChannelAdapter};
pub use card::{
    AppUsage, Behavior, ButtonSpec, ChannelCard, ChannelElement, CollapsiblePanel, DivText, Field,
    FormField, FormSpec, RichText, SelectOption, TurnChrome,
};
pub use event::{ChannelAction, ChannelEvent};
pub use key::{ChannelKey, ChannelName};
