//! The Feishu adapter (decouple-feishu-channel, task 3): implements the core's
//! neutral [`sebas_channels::ChannelAdapter`] for the `feishu` channel.
//!
//! Ownership boundary:
//! - **Inbound**: the adapter owns the Feishu WebSocket lifecycle (open-lark
//!   long connection + reconnect backoff), event deduplication, chat-type
//!   filtering and group mention gating. Events that pass the gates are
//!   translated into neutral [`sebas_channels::ChannelEvent`]s and forwarded
//!   to the core through the inbound channel given to
//!   [`FeishuAdapter::spawn`]. The core never sees Feishu wire shapes.
//! - **Outbound**: the adapter renders the neutral [`ChannelCard`] — the
//!   router's accumulated presentation model — into Feishu card schema 2.0
//!   JSON and calls the Feishu send/update APIs. The full card chrome
//!   (topic-derived header title, user-prompt quote block, divider, body
//!   element mapping, footer) lives here per the `feishu-cards` presentation
//!   rules; the router stays the streaming accumulator.
//!
//! The router's accumulated body vocabulary maps 1:1 onto the Feishu card
//! element vocabulary (`Hr → hr`, `Markdown → markdown`, `Div → div`,
//! `Button → v2 button` with `behaviors`, `Fields → div.fields`,
//! `CollapsiblePanel → collapsible_panel`, `Form → form`,
//! `SelectStatic → select_static`, `ColumnSet → column_set`) so this file's
//! `element_to_feishu` translation is mechanical ([`crate::cards::CardElement`]).

use crate::cards::{
    Card, CardBehavior, CardElement, CardText, CollapsiblePanel as FsCollapsiblePanel,
    CollapsiblePanelHeader, DivText as FsDivText, StandardIcon,
};
use crate::client::FeishuClient;
use crate::events::{FeishuEnvelope, FeishuIn, SessionKey};
use sebas_channels::adapter::CardRef;
use sebas_channels::card::{
    ChannelCard, ChannelElement, CollapsiblePanel as NeutralPanel, FormField, RichText,
};
use sebas_channels::event::{ChannelAction, ChannelEvent};
use sebas_channels::key::{ChannelKey, ChannelName};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

/// Parameters that configure the adapter's inbound WebSocket loop and its
/// outbound renderer. Mirrors the `[feishu]` config section plus the
/// `[card]` rendering knobs; the core assembles one [`FeishuAdapter`] from
/// these at startup (see `src/run.rs`).
#[derive(Debug, Clone, Default)]
pub struct FeishuAdapterConfig {
    pub app_id: String,
    pub app_secret: String,
    pub owner_id: String,
    /// Allowed inbound chat types (`"p2p"`, `"group"`, ...). Empty = all.
    pub allowed_chat_types: Vec<String>,
    /// Bot name for group @-mention gating. Empty = no gate.
    pub bot_name: String,
    /// Optional directory for raw inbound-payload snapshots (`replay --dir`).
    pub dump_dir: Option<std::path::PathBuf>,
    /// Card rendering knobs (`[card]`), interpreted by the adapter.
    pub card_config: crate::cards::CardConfig,
}

/// The concrete Feishu implementation of [`sebas_channels::ChannelAdapter`].
///
/// Thread-safety: the `render` side is a plain snapshot of client + config
/// (no interior mutation); the per-chat help-card message-id registry is
/// mutex-guarded. The adapter is cheap to clone exactly because all state it
/// needs for `spawn`/`render` is `Send + Sync`.
#[derive(Clone)]
pub struct FeishuAdapter {
    feishu: FeishuClient,
    config: FeishuAdapterConfig,
    /// Per-chat message-id of the interactive help card (`chat -> msg_id`),
    /// so a help-card tab click can PATCH in place instead of posting a new
    /// card. This is the adapter's own `channel key → feishu message_id`
    /// mapping (task 3.3); permission-card refs stay router-side keyed by
    /// request_id.
    help_card_msgid: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

impl FeishuAdapter {
    /// Build the adapter for direct use by the core's adapter registry.
    pub fn new(feishu: FeishuClient, config: FeishuAdapterConfig) -> Self {
        Self {
            feishu,
            config,
            help_card_msgid: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// The renderer's config (used by the core to seed the router's card
    /// config mirror, keeping `/settings` round-trips consistent).
    pub fn card_config(&self) -> &crate::cards::CardConfig {
        &self.config.card_config
    }

    pub fn feishu_client(&self) -> &FeishuClient {
        &self.feishu
    }

    /// Record/lookup the help card message-id for a chat (in-place tab
    /// switches PATCH that card). Message-ids are Feishu-assigned; an entry
    /// is only written after the first `send_card` of a help card returns.
    pub fn record_help_card_msgid(&self, chat_id: &str, msg_id: String) {
        self.help_card_msgid
            .lock()
            .unwrap()
            .insert(chat_id.to_string(), msg_id);
    }

    pub fn help_card_msg_id(&self, chat_id: &str) -> Option<String> {
        self.help_card_msgid.lock().unwrap().get(chat_id).cloned()
    }

    /// Async outbound render+send/update. The implementation detail the
    /// synchronous [`sebas_channels::ChannelAdapter::render`] delegate to.
    ///
    /// A `ChannelCard` carrying `turn` chrome is a **turn card** — the adapter
    /// frames it (topic-derived header, quote block, usage footer) and sends/
    /// updates per the `feishu-cards` presentation rules. A card without
    /// `turn` is a **standalone UI card** (help / provider / permission /
    /// error/status) rendered verbatim.
    pub async fn async_render(
        &self,
        key: &ChannelKey,
        card_ref: Option<&CardRef>,
        card: &ChannelCard,
    ) -> Result<Option<CardRef>, Box<dyn std::error::Error + Send + Sync>> {
        let session_key = SessionKey::from_channel_key(key);
        let framed = match &card.turn {
            Some(turn) => {
                let usage = turn.usage.as_ref().map(|u| crate::cards::CardFooter {
                    model: u.model.clone(),
                    round_input: 0,
                    round_output: 0,
                    total_input: u.total_input,
                    total_output: u.total_output,
                });
                render_channel_card_frame(
                    &turn.prompt,
                    &turn.session_id,
                    card,
                    usage.as_ref(),
                )
            }
            None => render_standalone_card(card),
        };
        let card_json = serde_json::to_value(framed)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        let http = reqwest::Client::new();
        // FeishuClient::send_card/update_card do not touch the config's
        // token state; rebuild the token manager per call. SEBAS_TEST_FAKE_TOKEN
        // (integration-test affordance) is honoured by the caller wiring.
        let tokens = crate::client::TokenManager::new(
            self.config.app_id.clone(),
            self.config.app_secret.clone(),
        );
        match card_ref {
            Some(r) => {
                self.feishu
                    .update_card(&http, &tokens, &r.0, card_json)
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
                Ok(Some(r.clone()))
            }
            None => {
                let thread_id = key
                    .reference
                    .split_once('\0')
                    .map(|(_, t)| t)
                    .map(str::to_owned);
                let msg_id = self
                    .feishu
                    .send_card(
                        &http,
                        &tokens,
                        &session_key,
                        card_json,
                        None,
                        thread_id.as_deref(),
                    )
                    .await
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
                Ok(Some(CardRef(msg_id)))
            }
        }
    }
}

fn short_model_name(model: &str) -> String {
    let known = [
        ("claude-sonnet-4", "sonnet"),
        ("claude-sonnet-5", "sonnet"),
        ("claude-opus-4", "opus"),
        ("claude-opus-5", "opus"),
        ("claude-haiku-3", "haiku"),
        ("claude-haiku-4", "haiku"),
    ];
    for (prefix, short) in &known {
        if model.starts_with(prefix) {
            return short.to_string();
        }
    }
    if let Some(name) = model
        .strip_prefix("claude-")
        .and_then(|rest| rest.split('-').next())
    {
        return name.to_string();
    }
    model.to_string()
}

fn format_token_count(n: u64) -> String {
    if n >= 1000 {
        let k = n as f64 / 1000.0;
        if k >= 100.0 {
            format!("{:.0}K", k)
        } else {
            format!("{:.1}K", k)
        }
    } else {
        n.to_string()
    }
}

// ── Neutral presentation → Feishu card schema 2.0 JSON ──────────────────────

/// Render a **framed** turn card (the router's accumulated [`ChannelCard`]) as
/// Feishu schema-2.0 JSON: header title = topic derived from the first
/// non-empty prompt line (via [`crate::cards::derive_topic`]), theme from the
/// card body's header colour, quote block with the user prompt, divider, body
/// elements, footer (`msg_id: {session_id}` or usage line).
///
/// `session_id` is the value the footer falls back to when no usage is known
/// (`msg_id: {session_id}`), matching the historical `render_accumulated_card`
/// contract.
pub fn render_channel_card_frame(
    prompt: &str,
    session_id: &str,
    card: &ChannelCard,
    usage: Option<&crate::cards::CardFooter>,
) -> Card {
    let mut out = Card::new(&crate::cards::derive_topic(prompt), &card.theme);
    out.push_text(format!("> {prompt}"));
    out.push_divider();
    for el in &card.elements {
        out.body.elements.push(element_to_feishu(el));
    }
    match usage {
        Some(u) => {
            let model = u
                .model
                .as_deref()
                .map(short_model_name)
                .unwrap_or_else(|| "?".to_string());
            let total_in = format_token_count(u.total_input);
            let total_out = format_token_count(u.total_output);
            let ctx = format_token_count(u.total_input);
            out.push_note(format!(
                "{model}  ·  in: {total_in}  out: {total_out}  ·  ctx: {ctx}"
            ));
        }
        None => {
            out.push_note(format!("msg_id: {session_id}"));
        }
    }
    out
}

/// Render a **standalone** card (fire-and-forget UI card: help, provider,
/// permission, error/status) from the neutral [`ChannelCard`] the router
/// produced. No quote/footer chrome — the card's own elements are the whole
/// body, exactly mirroring the pre-split dedicated renderers.
pub fn render_standalone_card(card: &ChannelCard) -> Card {
    let mut out = Card::new(&card.title, &card.theme);
    for el in &card.elements {
        out.body.elements.push(element_to_feishu(el));
    }
    out
}

/// Render only the neutral body elements (no header/frame) into feishu
/// `CardElement`s — used when the caller composes its own `Card` header
/// (e.g. preset-details panels the core assembles into a form card).
pub fn render_raw_body(card: &ChannelCard) -> Vec<CardElement> {
    card.elements.iter().map(element_to_feishu).collect()
}

/// The mechanical 1:1 vocabulary mapping from the neutral presentation model
/// to Feishu card schema 2.0 elements (kept lossless so the wire shape
/// matches the pre-split rendering exactly).
pub fn element_to_feishu(el: &ChannelElement) -> CardElement {
    match el {
        ChannelElement::Hr => CardElement::Hr,
        ChannelElement::Markdown { content } => CardElement::Markdown {
            content: content.clone(),
        },
        ChannelElement::Div { text } => CardElement::Div {
            text: div_text(text),
        },
        ChannelElement::Button {
            text,
            style,
            behaviors,
        } => CardElement::Button {
            text: rich_text(text),
            r#type: style.clone(),
            behaviors: behaviors
                .iter()
                .map(|b| CardBehavior {
                    r#type: b.r#type.clone(),
                    value: b.value.clone(),
                })
                .collect(),
        },
        ChannelElement::Fields(fields) => CardElement::Fields(
            fields
                .iter()
                .map(|f| crate::cards::CardField {
                    is_short: f.is_short,
                    text: rich_text(&f.text),
                })
                .collect(),
        ),
        ChannelElement::CollapsiblePanel(panel) => {
            CardElement::CollapsiblePanel(collapsible_panel(panel))
        }
        ChannelElement::Form {
            name,
            fields,
            initials,
            submit,
        } => CardElement::Form {
            name: name.clone(),
            elements: form_elements(fields, initials, submit),
        },
        ChannelElement::SelectStatic {
            name,
            placeholder,
            options,
            initial,
            on_change,
        } => CardElement::SelectStatic {
            name: name.clone(),
            placeholder: rich_text(placeholder),
            options: options.clone(),
            initial: initial.clone(),
            on_change: on_change.clone(),
        },
        ChannelElement::ColumnSet {
            flex_mode,
            horizontal_spacing,
            columns,
        } => CardElement::ColumnSet {
            flex_mode: *flex_mode,
            horizontal_spacing: horizontal_spacing.clone(),
            columns: columns
                .iter()
                .map(|c| crate::cards::CardColumn {
                    tag: "column",
                    width: c.width.clone(),
                    elements: c.elements.iter().map(element_to_feishu).collect(),
                    vertical_spacing: c.vertical_spacing.clone(),
                    horizontal_align: c.horizontal_align.clone(),
                })
                .collect(),
        },
    }
}

fn div_text(text: &sebas_channels::card::DivText) -> FsDivText {
    FsDivText {
        tag: text.tag.clone(),
        content: text.content.clone(),
        text_size: text.text_size.clone(),
        text_color: text.text_color.clone(),
    }
}

fn rich_text(text: &RichText) -> CardText {
    CardText {
        tag: text.tag.clone(),
        content: text.content.clone(),
    }
}

fn collapsible_panel(panel: &NeutralPanel) -> FsCollapsiblePanel {
    FsCollapsiblePanel {
        expanded: panel.expanded,
        header: CollapsiblePanelHeader {
            title: rich_text(&panel.header_title),
            icon: StandardIcon {
                tag: "standard_icon".into(),
                token: panel.icon_token.clone(),
                size: "16px 16px".into(),
            },
            icon_position: "right".into(),
            icon_expanded_angle: -180,
        },
        elements: panel.elements.iter().map(element_to_feishu).collect(),
    }
}

/// Neutral `FormField`s → pre-serialized feishu form-container elements,
/// reusing `crate::forms`' established input/select/sumbit element shapes.
/// Public so the core's provider/crud form composition can build the same
/// container elements without re-implementing feishu wire shapes.
pub fn form_elements(
    fields: &[FormField],
    initials: &std::collections::BTreeMap<String, String>,
    submit: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let form_name = submit
        .get("form")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("form");
    let mut elements: Vec<serde_json::Value> = Vec::new();
    for f in fields {
        elements.push(label_element(f));
        match f {
            FormField::Text {
                name,
                required,
                placeholder,
                disabled,
                ..
            } => {
                elements.push(crate::forms::input_element(
                    name,
                    placeholder,
                    *required,
                    initials.get(name).cloned(),
                    *disabled,
                ));
            }
            FormField::Select {
                name,
                required,
                options,
                on_change,
                ..
            } => {
                let options: Vec<crate::forms::SelectOption> = options
                    .iter()
                    .map(|o| crate::forms::SelectOption {
                        value: o.value.clone(),
                        label: o.label.clone(),
                    })
                    .collect();
                elements.push(crate::forms::select_element(
                    name,
                    &options,
                    *required,
                    initials.get(name).map(|v| vec![v.clone()]),
                    on_change.as_ref(),
                ));
            }
        }
    }
    elements.push(crate::forms::submit_button(form_name, "提交", submit));
    elements
}

fn label_element(f: &FormField) -> serde_json::Value {
    let star = match f {
        FormField::Text { required, .. } | FormField::Select { required, .. } => {
            if *required { " *" } else { "" }
        }
    };
    serde_json::json!({ "tag": "markdown", "content": format!("**{}**{}", f.label(), star) })
}

// ── Inbound WebSocket lifecycle ─────────────────────────────────────────────

/// Neutral-key helper: a feishu `SessionKey` (`chat\0thread` composite) maps
/// onto a `ChannelKey` whose reference keeps the composite, per the adapter's
/// key-encoding contract.
fn channel_key(k: &SessionKey) -> ChannelKey {
    ChannelKey::feishu(&k.chat_id, k.thread_id.as_deref())
}

/// Feishu-boundary translation: `FeishuIn` → neutral [`ChannelEvent`]. Kept
/// at the adapter's boundary so the core never sees Feishu wire shapes.
/// (This is the historical `src/ws_loop::feishu_in_to_channel_event` moved
/// into the adapter; session keys, reply targets and message refs are all
/// adapter-owned encoding.)
pub fn feishu_in_to_channel_event(evt: FeishuIn) -> ChannelEvent {
    match evt {
        FeishuIn::Text {
            key,
            text,
            reply_to,
            chat_type: _,
            mentions: _,
        } => ChannelEvent::Text {
            key: channel_key(&key),
            text,
            reply_target: reply_to,
        },
        FeishuIn::Media {
            key,
            files,
            caption,
            reply_to,
            ..
        } => ChannelEvent::Media {
            key: channel_key(&key),
            files,
            caption,
            reply_target: reply_to,
        },
        FeishuIn::ButtonCb { key, action, .. } => ChannelEvent::ButtonCb {
            key: channel_key(&key),
            action: ChannelAction {
                session_id: action.session_id,
                request_id: action.request_id,
                decision: action.decision,
                value: action.value,
            },
        },
        FeishuIn::FormCb {
            key,
            value,
            form_value,
            message_id,
            ..
        } => ChannelEvent::FormCb {
            key: channel_key(&key),
            value,
            form_value,
            card_ref: message_id,
        },
    }
}

/// Gate + translate one parsed inbound event. Returns `None` when a gate
/// rejects it (chat type not allowed, group message not mentioning the bot)
/// or the envelope produced no recognizable event.
fn gate_and_translate(
    allowed_chat_types: &[String],
    bot_name: &str,
    in_ev: FeishuIn,
) -> Option<ChannelEvent> {
    if !is_chat_type_allowed(allowed_chat_types, in_ev.chat_type()) {
        return None;
    }
    if should_filter_by_mention(bot_name, &in_ev) {
        return None;
    }
    Some(feishu_in_to_channel_event(in_ev))
}

/// chat_type 归一化: "private"(本地缺省/存量配置的幻影值)映射到飞书真实
/// 私聊 wire 值 "p2p",其余原样返回。
fn norm_chat_type(t: &str) -> &str {
    if t == "private" { "p2p" } else { t }
}

/// 是否允许该 chat_type 的消息。空列表 = 全部允许;private↔p2p 视为同值。
fn is_chat_type_allowed(allowed: &[String], chat_type: &str) -> bool {
    allowed.is_empty()
        || allowed
            .iter()
            .any(|t| norm_chat_type(t) == norm_chat_type(chat_type))
}

/// 群聊(group/p2p)中非 @bot 消息应过滤;无 bot_name 配置时不过滤;
/// 私聊(chat_type 非 group/p2p)不过滤。
fn should_filter_by_mention(bot_name: &str, evt: &FeishuIn) -> bool {
    let chat_type = evt.chat_type();
    if chat_type != "group" && chat_type != "p2p" {
        return false;
    }
    if bot_name.is_empty() {
        return false;
    }
    let mentioned = evt.mentions().iter().any(|m| {
        m.name
            .to_lowercase()
            .contains(&bot_name.to_lowercase())
            || m.key.to_lowercase().contains(&bot_name.to_lowercase())
    });
    !mentioned
}

impl sebas_channels::ChannelAdapter for FeishuAdapter {
    fn channel_name(&self) -> ChannelName {
        ChannelName::FEISHU.into()
    }

    fn spawn(
        &self,
        inbound: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let adapter = self.clone();
        tokio::spawn(async move {
            adapter.run_ws_loop(inbound).await;
        });
        Ok(())
    }

    fn shutdown(&self) {
        // The open-lark client owns the connection; dropping `self`'s spawned
        // loop is driven by the runtime teardown. There is no process-local
        // socket to remove (unlike the core session channel), so shutdown is
        // a no-op placeholder kept for trait symmetry.
    }

    /// Render one outbound presentation instance for a session.
    ///
    /// Whether this blocks depends on the caller: the core's outbound pump is
    /// an async task, so the dispatcher uses [`FeishuAdapter::async_render`]
    /// directly. This synchronous trait entry point covers callers outside a
    /// runtime (tests, sync bridges) by driving a short-lived current-thread
    /// runtime of its own; if a runtime is already present on this thread, the
    /// call is *rejected* with a clear error rather than panicking with
    /// "Cannot start a runtime from within a runtime".
    fn render(
        &self,
        key: &ChannelKey,
        card_ref: Option<&CardRef>,
        card: &ChannelCard,
    ) -> Result<Option<CardRef>, Box<dyn std::error::Error + Send + Sync>> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err("FeishuAdapter::render must not be called from inside a tokio runtime worker; \
                        use FeishuAdapter::async_render (the core outbound pump path)"
                .into());
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
        rt.block_on(self.async_render(key, card_ref, card))
    }
}

impl FeishuAdapter {
    /// The WebSocket event loop, one connection-attempt cycle at a time with
    /// exponential backoff (moved from the old `src/ws_loop::run_ws_loop`).
    /// Each inbound payload is parsed as a [`FeishuEnvelope`], gated
    /// (dedup / chat-type / mention), translated to a neutral [`ChannelEvent`]
    /// and forwarded to the core through `inbound`.
    async fn run_ws_loop(&self, inbound: tokio::sync::mpsc::Sender<ChannelEvent>) {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(60);

        let app_id = self.config.app_id.clone();
        let app_secret = self.config.app_secret.clone();
        let owner_id = self.config.owner_id.clone();
        let dump_dir = self.config.dump_dir.clone();
        let allowed_chat_types = self.config.allowed_chat_types.clone();
        let bot_name = self.config.bot_name.clone();

        loop {
            // Fresh dispatcher + handler per attempt so retries start with a
            // clean handler clone. `register_raw` (openlark 0.19+) is a keyed
            // map insert, so both inbound names can share one handler; any
            // registration error aborts the loop (fatal config bug).
            let handler = WsEventHandler {
                inbound: inbound.clone(),
                owner_id: owner_id.clone(),
                dump_dir: dump_dir.clone(),
                seen_events: Arc::new(Mutex::new(HashSet::new())),
                allowed_chat_types: allowed_chat_types.clone(),
                bot_name: bot_name.clone(),
            };
            let dispatcher =
                match open_lark::ws_client::EventDispatcherHandler::builder()
                    .register_raw("im.message.receive_v1", handler.clone())
                    .and_then(|b| b.register_raw("card.action.trigger", handler))
                {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "failed to register event handlers; aborting WS loop"
                        );
                        return;
                    }
                };

            let ws_config = Arc::new(
                open_lark::Config::builder()
                    .app_id(app_id.clone())
                    .app_secret(app_secret.clone())
                    .build(),
            );

            tracing::info!("connecting to feishu WS via open-lark");
            let result = open_lark::ws_client::LarkWsClient::open(ws_config, dispatcher).await;

            match result {
                Ok(()) => {
                    tracing::info!("feishu WS session ended cleanly; reconnecting");
                    backoff = Duration::from_secs(1);
                }
                Err(open_lark::ws_client::WsClientError::ConnectionClosed { .. }) => {
                    tracing::warn!("feishu WS closed; reconnecting");
                    backoff = Duration::from_secs(1);
                }
                Err(open_lark::ws_client::WsClientError::RequestError(core_err))
                    if matches!(core_err, open_lark::CoreError::Authentication { .. }) =>
                {
                    tracing::error!(
                        error = %core_err,
                        "feishu WS auth failed; aborting (fatal)"
                    );
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "feishu WS failed; backing off");
                }
            }

            tracing::info!(?backoff, "WS reconnect after backoff");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(max_backoff);
        }
    }
}

/// Raw-bytes WS handler bound to `im.message.receive_v1` /
/// `card.action.trigger`: parse → dedup (event_id) → chat-type/mention gates
/// → neutral event → forward to the core. Owned by the adapter's WS loop so
/// the core's inbound seam never depends on open-lark.
#[derive(Clone)]
struct WsEventHandler {
    inbound: tokio::sync::mpsc::Sender<ChannelEvent>,
    owner_id: String,
    dump_dir: Option<std::path::PathBuf>,
    seen_events: Arc<Mutex<HashSet<String>>>,
    allowed_chat_types: Vec<String>,
    bot_name: String,
}

impl open_lark::ws_client::EventHandler for WsEventHandler {
    fn handle(
        &self,
        payload: &[u8],
    ) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(dir) = &self.dump_dir {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let pid = std::process::id();
            let path = dir.join(format!("{ts}-{pid}.json"));
            if let Err(e) = std::fs::write(&path, payload) {
                tracing::warn!(?e, ?path, "failed to dump inbound payload");
            }
        }
        let text = match std::str::from_utf8(payload) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(?e, "non-UTF8 payload, skipping");
                return Ok(());
            }
        };
        let env = match serde_json::from_str::<FeishuEnvelope>(text) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(?e, "failed to parse FeishuEnvelope, skipping");
                return Ok(());
            }
        };
        // 事件去重：飞书可能重投相同 event_id 的事件。容量上限 4096。
        if let Some(ref eid) = env.header.event_id {
            let mut seen = self.seen_events.lock().unwrap();
            if !seen.insert(eid.clone()) {
                tracing::debug!(event_id = %eid, "duplicate event, skipping");
                return Ok(());
            }
            if seen.len() > 4096 {
                seen.clear();
            }
        }
        let Some(in_ev) = env.into_event(&self.owner_id) else {
            tracing::debug!("envelope produced no FeishuIn (filtered or unrecognized)");
            return Ok(());
        };
        let Some(channel_evt) =
            gate_and_translate(&self.allowed_chat_types, &self.bot_name, in_ev)
        else {
            tracing::debug!("inbound event rejected by chat-type/mention gate");
            return Ok(());
        };
        tracing::debug!(?channel_evt, "forwarding neutral event to core");
        if self.inbound.blocking_send(channel_evt).is_err() {
            tracing::warn!("inbound channel closed; dropping event");
        }
        Ok(())
    }
}

// `SessionKey` ↔ neutral `ChannelKey` (the reference keeps the `chat\0thread`
// composite). Used by the adapter's render path for thread-aware send_card.
impl SessionKey {
    pub fn from_channel_key(key: &ChannelKey) -> SessionKey {
        match key.reference.split_once('\0') {
            Some((chat_id, thread_id)) => SessionKey {
                chat_id: chat_id.to_string(),
                thread_id: Some(thread_id.to_string()),
            },
            None => SessionKey {
                chat_id: key.reference.clone(),
                thread_id: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::CardAction;
    use sebas_channels::card::{Behavior, DivText};
    use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};

    /// Task 3.3 (decouple-feishu-channel): the adapter owns callback →
    /// neutral-event restoration — every FeishuIn variant maps to a
    /// ChannelEvent addressed by the composite feishu ChannelKey, with the
    /// reply target / callback payload / form values carried through.
    #[test]
    fn every_inbound_variant_restores_to_a_neutral_event() {
        let key = SessionKey {
            chat_id: "oc_x".into(),
            thread_id: Some("t1".into()),
        };

        let text = feishu_in_to_channel_event(FeishuIn::Text {
            key: key.clone(),
            text: "hi".into(),
            reply_to: Some("om_1".into()),
            chat_type: "group".into(),
            mentions: Vec::new(),
        });
        let ChannelEvent::Text {
            key: k,
            text: t,
            reply_target,
        } = text
        else {
            panic!("expected Text");
        };
        assert_eq!(k, ChannelKey::feishu("oc_x", Some("t1")));
        assert_eq!(k.reference, "oc_x\0t1");
        assert_eq!(t, "hi");
        assert_eq!(reply_target.as_deref(), Some("om_1"));

        let media = feishu_in_to_channel_event(FeishuIn::Media {
            key: key.clone(),
            files: vec!["file_v3_1".into()],
            caption: Some("cap".into()),
            reply_to: None,
            chat_type: "p2p".into(),
        });
        let ChannelEvent::Media { key: k, files, .. } = media else {
            panic!("expected Media");
        };
        assert_eq!(k, ChannelKey::feishu("oc_x", Some("t1")));
        assert_eq!(files, vec!["file_v3_1".to_string()]);

        let button = feishu_in_to_channel_event(FeishuIn::ButtonCb {
            key: key.clone(),
            action: CardAction {
                session_id: "s1".into(),
                request_id: Some("req_1".into()),
                decision: Some("allow_once".into()),
                value: serde_json::json!({"session_id": "s1"}),
            },
            chat_type: "p2p".into(),
        });
        let ChannelEvent::ButtonCb { key: k, action } = button else {
            panic!("expected ButtonCb");
        };
        assert_eq!(k, ChannelKey::feishu("oc_x", Some("t1")));
        assert_eq!(action.session_id, "s1");
        assert_eq!(action.decision.as_deref(), Some("allow_once"));

        let form = feishu_in_to_channel_event(FeishuIn::FormCb {
            key,
            value: serde_json::json!({"session_id": "s1"}),
            form_value: [("name".to_string(), serde_json::json!("v"))].into(),
            message_id: Some("om_card".into()),
            chat_type: "p2p".into(),
        });
        let ChannelEvent::FormCb {
            key: k,
            value,
            form_value,
            card_ref,
        } = form
        else {
            panic!("expected FormCb");
        };
        assert_eq!(k, ChannelKey::feishu("oc_x", Some("t1")));
        assert_eq!(card_ref.as_deref(), Some("om_card"));
        assert_eq!(value["session_id"], "s1");
        assert_eq!(form_value["name"], "v");
    }

    fn sample_channel_card() -> ChannelCard {
        let mut card = ChannelCard::new("重构 src/foo.rs", "orange");
        card.elements.push(ChannelElement::Markdown {
            content: "hello".into(),
        });
        card.elements.push(ChannelElement::Hr);
        card.elements.push(ChannelElement::Button {
            text: RichText::plain("允许"),
            style: "primary".into(),
            behaviors: vec![Behavior {
                r#type: "callback".into(),
                value: serde_json::json!({"session_id": "s1"}),
            }],
        });
        card
    }

    #[test]
    fn framed_card_matches_historical_accumulated_chrome() {
        let card = render_channel_card_frame("重构 foo", "msg_9", &sample_channel_card(), None);
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(v["schema"], "2.0");
        assert_eq!(v["header"]["title"]["content"], "重构 foo");
        assert_eq!(v["header"]["template"], "orange");
        let s = serde_json::to_string(&card).unwrap();
        assert!(s.contains("> 重构 foo"), "quote block: {s}");
        assert!(s.contains("hello"), "body text: {s}");
        assert!(s.contains("msg_id: msg_9"), "footer: {s}");
        // Button is a first-class v2 button with behaviors, never a V1 action container.
        assert!(s.contains("\"behaviors\":[{\"type\":\"callback\""));
        assert!(!s.contains("\"tag\":\"action\""), "no V1 action block: {s}");
    }

    #[test]
    fn usage_footer_shows_short_model_and_cumulative_totals() {
        let usage = crate::cards::CardFooter {
            model: Some("claude-sonnet-4-20250514".into()),
            round_input: 1234,
            round_output: 5678,
            total_input: 5000,
            total_output: 3000,
        };
        let card = render_channel_card_frame(
            "hi",
            "msg_1",
            &ChannelCard::new("hi", "blue"),
            Some(&usage),
        );
        let s = serde_json::to_string(&card).unwrap();
        assert!(
            s.contains("sonnet  ·  in: 5.0K  out: 3.0K  ·  ctx: 5.0K"),
            "footer: {s}"
        );
        assert!(
            !s.contains("msg_id:"),
            "usage footer replaces msg_id footer: {s}"
        );
    }

    #[test]
    fn standalone_card_omits_quote_and_footer_chrome() {
        let card = render_standalone_card(&ChannelCard::new("⚠ 权限请求", "orange"));
        let s = serde_json::to_string(&card).unwrap();
        assert!(s.contains("⚠ 权限请求"));
        assert!(!s.contains("> "), "no quote block in standalone card");
        assert!(!s.contains("msg_id:"), "no footer in standalone card");
    }

    #[test]
    fn every_neutral_variant_maps_to_a_feishu_element() {
        use sebas_channels::card::Field;
        let mut card = ChannelCard::new("t", "blue");
        card.elements.push(ChannelElement::Hr);
        card.elements.push(ChannelElement::Div {
            text: DivText {
                tag: "plain_text".into(),
                content: "n".into(),
                text_size: Some("notation".into()),
                text_color: Some("grey".into()),
            },
        });
        card.elements.push(ChannelElement::Fields(vec![Field {
            is_short: false,
            text: RichText::plain("label\nvalue"),
        }]));
        card.elements.push(ChannelElement::CollapsiblePanel(NeutralPanel {
            expanded: false,
            header_title: RichText::plain("完整参数"),
            icon_token: "down-small-ccm_outlined".into(),
            elements: vec![ChannelElement::Markdown {
                content: "```json\n{}\n```".into(),
            }],
        }));
        card.elements.push(ChannelElement::Form {
            name: "provider-preset".into(),
            fields: vec![FormField::Text {
                name: "name".into(),
                label: "名称".into(),
                required: true,
                placeholder: "provider name".into(),
                secret: false,
                disabled: false,
            }],
            initials: Default::default(),
            submit: serde_json::json!({"form": "provider-preset", "op": "submit"}),
        });
        card.elements.push(ChannelElement::SelectStatic {
            name: "mode".into(),
            placeholder: RichText::plain("选择"),
            options: vec![("auto".into(), "Auto".into())],
            initial: Some("auto".into()),
            on_change: serde_json::json!({"form": "x"}),
        });
        card.elements.push(ChannelElement::ColumnSet {
            flex_mode: false,
            horizontal_spacing: None,
            columns: vec![sebas_channels::card::Column {
                width: None,
                elements: vec![ChannelElement::Button {
                    text: RichText::plain("编辑"),
                    style: "default".into(),
                    behaviors: vec![Behavior {
                        r#type: "callback".into(),
                        value: serde_json::json!({"op": "edit"}),
                    }],
                }],
                vertical_spacing: None,
                horizontal_align: None,
            }],
        });

        let out = render_standalone_card(&card);
        let s = serde_json::to_string(&out).unwrap();
        for tag in [
            "\"tag\":\"div\"",
            "\"tag\":\"hr\"",
            "\"tag\":\"markdown\"",
            "\"tag\":\"collapsible_panel\"",
            "\"tag\":\"form\"",
            "\"tag\":\"select_static\"",
            "\"tag\":\"column_set\"",
        ] {
            assert!(s.contains(tag), "missing {tag} in {s}");
        }
        assert!(s.contains("\"fields\":"), "Fields → div.fields");
        // Button uses behaviors[].value, never a V1 action container.
        assert!(!s.contains("\"tag\":\"action\""));
    }

    #[test]
    fn form_fields_reuse_forms_module_element_shapes() {
        let elements = form_elements(
            &[FormField::Text {
                name: "api_key".into(),
                label: "API Key".into(),
                required: false,
                placeholder: "sk-...".into(),
                secret: true,
                disabled: false,
            }],
            &Default::default(),
            &serde_json::json!({"form": "provider-custom", "op": "submit"}),
        );
        assert_eq!(elements.len(), 3, "label input + submit button");
        assert_eq!(elements[0]["tag"], "markdown");
        assert_eq!(elements[0]["content"], "**API Key**");
        assert_eq!(elements[1]["tag"], "input");
        assert_eq!(elements[1]["name"], "api_key");
        assert_eq!(elements[1]["placeholder"]["content"], "sk-...");
        assert_eq!(elements[1]["width"], "fill");
        let submit = &elements[2];
        assert_eq!(submit["tag"], "button");
        assert_eq!(submit["form_action_type"], "submit");
        assert_eq!(submit["behaviors"][0]["value"]["op"], "submit");
        assert_eq!(submit["name"], "provider-custom_submit");
    }
}