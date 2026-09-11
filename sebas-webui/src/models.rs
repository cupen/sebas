//! Data models for the WebUI dashboard.

use serde::Serialize;

/// The operator-facing status of a session: one of six words, derived from
/// `(MappingState, phase)` by [`SessionStatus::derive`].
///
/// This exists because `phase` carries *Feishu reaction names* — `Get`,
/// `OnIt`, `CrossMark` — which are an implementation detail of how the
/// router decorates a chat card. Rendering them raw is how `OnIt` ended up
/// on the operator's screen as a status. The projection lives here rather
/// than in minijinja so there is exactly one copy of the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Starting,
    Queued,
    Working,
    Done,
    Failed,
    Dormant,
    /// （add-remote-execution-node 8.4）会话**在等人批**，不是在干活。
    ///
    /// 它由 `remote.parked_approvals > 0` 投影而来，不是 mapping 的原始状态：
    /// 一个进程活着、有悬空审批的远端的会话，其底层 status 仍是 active——
    /// 直接呈现 active 会让操作者以为它在跑。
    Waiting,
}

impl SessionStatus {
    /// Derive from a mapping state discriminant and the raw card phase.
    ///
    /// `state` is `"active"`, `"spawning"` or `"dormant"`; `phase` is a
    /// `sebas_dispatch::card_state::phase` constant, or empty when the router has no
    /// card state for the session yet. An active session with no phase is
    /// Queued, not Working: the child process exists but has not produced
    /// anything, and claiming otherwise would be a lie the operator acts on.
    pub fn derive(state: &str, phase: &str) -> Self {
        match state {
            "spawning" => Self::Starting,
            "dormant" => Self::Dormant,
            // fail-fast-on-startup-errors：spawn 失败的会话诚实呈现为 Failed
            // （而非继续假装 Starting/Queued）。
            "spawn-failed" => Self::Failed,
            // "active", plus any unknown state, falls through to the phase.
            _ => match phase {
                "OnIt" => Self::Working,
                "DONE" => Self::Done,
                "CrossMark" => Self::Failed,
                // "Get" (received) and empty both mean "nothing yet".
                _ => Self::Queued,
            },
        }
    }

    /// The word shown to the operator.
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "Starting",
            Self::Queued => "Queued",
            Self::Working => "Working",
            Self::Done => "Done",
            Self::Failed => "Failed",
            Self::Dormant => "Dormant",
            Self::Waiting => "Waiting",
        }
    }

    /// Lowercase slug, used as the `data-status` attribute the stylesheet and
    /// the endpoint tests both key off.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Queued => "queued",
            Self::Working => "working",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Dormant => "dormant",
            Self::Waiting => "waiting",
        }
    }

    /// A shape, so status survives greyscale and colour-blindness — colour is
    /// never the only channel.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Starting => "◇",
            Self::Queued => "▹",
            Self::Working => "▶",
            Self::Done => "✓",
            Self::Failed => "✕",
            Self::Dormant => "·",
            Self::Waiting => "⏸",
        }
    }

    /// （add-remote-execution-node 8.4）把「有悬空审批」投影进状态词。
    ///
    /// 只把**非终态**改成 Waiting：一个已经 Done/Failed 的会话不该因为历史
    /// 上留了一条没答的审批就看起来还在等人。底层 `status` 不变（active 计数
    /// 照旧），变的是呈现。
    pub fn with_parked_approvals(self, parked: u32) -> Self {
        if parked == 0 {
            return self;
        }
        match self {
            Self::Done | Self::Failed => self,
            _ => Self::Waiting,
        }
    }
}

/// Shorten a long identifier by eliding its middle, keeping both ends.
///
/// Both ends matter for these ids: the prefix identifies the kind and the
/// suffix is what actually distinguishes two sessions, so end-truncation
/// (`text-overflow: ellipsis`) would hide the discriminating part. Callers
/// must still put the full value in a `title` attribute.
///
/// Operates on chars, not bytes, so a multi-byte id cannot be split mid
/// character.
pub fn middle_truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    // One char of the budget goes to the ellipsis itself.
    let keep = max.saturating_sub(1);
    let head = keep / 2 + keep % 2;
    let tail = keep / 2;
    let mut out: String = chars[..head].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - tail..]);
    out
}

/// A single session row for the dashboard / session list.
#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    /// URL-encoded session key (`channel\0reference`).
    pub encoded_key: String,
    pub channel: String,
    pub reference: String,
    /// The ACP session_id, if active/dormant.
    pub session_id: Option<String>,
    /// `session_id` with its middle elided, for display. The full value goes
    /// in the cell's `title`.
    pub session_id_short: Option<String>,
    /// "active", "spawning", or "dormant". Retained for the session counts
    /// and the JSON summary; not rendered as a status word.
    pub status: &'static str,
    /// Operator-facing status word, e.g. "Working".
    pub status_label: &'static str,
    /// Lowercase slug for `data-status`, e.g. "working".
    pub status_slug: &'static str,
    /// Shape channel for the status, e.g. "▶".
    pub status_glyph: &'static str,
    /// Human-readable relative time.
    pub last_active: String,
    /// Unix timestamp backing `last_active`. Exposed so clients can sort
    /// without re-parsing the rendered string.
    pub last_active_unix: i64,
    /// True if this session is the WebUI's currently focused one. The
    /// template renders an "active" indicator and gates the Switch button.
    pub is_active: bool,
    /// 绑定项目的稳定 id（`proj-<12hex>`，workbench-agent-wire-fix 2.5）。
    /// `None` = inbox（飞书来源或未绑项目）。原始 path 不是 wire 标识。
    pub project_id: Option<String>,
    /// A short preview of the session's first user message, used by the
    /// agent sidebar as a display label when no project_dir is set.
    pub prompt_preview: Option<String>,
    /// 当前生效的模型 id（add-acp-model-selection）；`None` = agent 无模型面。
    pub current_model: Option<String>,
    /// 该会话可选的模型 id 列表（agent 的 configOptions）。创建会话表单用它
    /// 作为下拉数据源（取最近一个暴露模型列表的会话）。
    pub available_models: Option<Vec<String>>,
    /// 会话创建时绑定的执行后端 kind（add-composer-agent-binding）；
    /// `None` = 配置的默认 kind。
    pub agent_kind: Option<String>,
    /// （wire-webui-sebas-agent-e2e D4）会话所属执行体（"acp"/"native"，
    /// 由复合后端打标）；`None` = 未打标。
    pub backend: Option<String>,
    /// （workbench-turn-queue 7.4）待生效提交条数——Rail 关闭确认对话框
    /// 点名「将丢弃 N 条」的数据源。
    pub pending_count: usize,
    /// （add-remote-execution-node 8.x）远端会话的节点/mode/悬空审批呈现信息。
    /// 直接透传 core 的 [`sebas_dispatch::RemoteSessionView`]：`None` = 主控本机
    /// 会话（节点维度对它不存在），**不伪造**一个 `online`。
    pub remote: Option<sebas_dispatch::RemoteSessionView>,
    /// （add-agent-mode-selection）操作者期望的 mode（控制面词汇）；`None` =
    /// agent 默认行为。远端会话与投影 desired 同值。
    pub desired_mode: Option<String>,
    /// （add-agent-mode-selection）执行体回报的实际生效 mode；`None` = 未声称
    /// 生效（与 desired 的差异如实可见）。
    pub effective_mode: Option<String>,
    /// （rail-declutter-unread D1）服务端累计的可见回复段数——rail 会话行
    /// 未读徽标 = `msg_count − 浏览器读锚`。随 `session.updated` 广播 +
    /// rail 10s 轮询兜底。
    pub msg_count: u64,
}

/// Dashboard overview data.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardData {
    pub active_count: usize,
    pub dormant_count: usize,
    pub spawning_count: usize,
    pub total_sessions: usize,
    /// Human-readable uptime, e.g. "2d 3h 14m". Formerly a raw second count
    /// labelled "Uptime (s)", which made the operator do the arithmetic.
    pub uptime: String,
    pub recent_sessions: Vec<SessionRow>,
    /// Summary of the WebUI's currently focused session, if any.
    pub active_session: Option<serde_json::Value>,
    /// URL-encoded key of the active session (shortcut for the template).
    pub active_session_key: Option<String>,
}

/// Router info for the settings page.
#[derive(Debug, Clone, Serialize, Default)]
pub struct RouterInfo {
    pub listen: Option<String>,
    pub provider_count: usize,
    pub debug: bool,
    pub has_auth: bool,
    /// Provider names and their base URLs.
    pub providers: Vec<ProviderInfo>,
}

/// A single provider's info for display.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub name: String,
    /// 派生 preset 名；`None` = 自定义 provider。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    pub base_url_anthropic: Option<String>,
    pub base_url_openai_chat: Option<String>,
    pub base_url_openai_responses: Option<String>,
}

/// Card config data for the settings page.
#[derive(Debug, Clone, Serialize)]
pub struct CardConfigInfo {
    pub theme_color: String,
    pub fold_long_output: bool,
    pub thinking_display: String,
    pub max_user_text_chars: usize,
    pub max_tool_output_chars: usize,
}

/// One conversation entry on the session payload（workbench-conversation-view
/// 1.1，design D1/D2）：`GET /api/sessions/{key}` 与 summary 聚焦会话的
/// 有序条目序列中的一条。`kind` 是「谁说的」（prompt = 操作员提交，content =
/// agent 侧产出），`element_type` 是渲染类型（markdown/thinking/tool/error）；
/// 两侧同序，客户端无需按时间戳重建顺序。
#[derive(Debug, Clone, Serialize)]
pub struct ConversationEntryView {
    /// 0-based monotonic transcript position.
    pub position: u64,
    /// `"prompt"` | `"content"`.
    pub kind: String,
    /// `"markdown"` | `"thinking"` | `"tool"` | `"error"`.
    pub element_type: String,
    pub content: String,
    /// Unix seconds when the entry was appended. Anchors the client's
    /// seen-boundary seam to a stable identity that doesn't change when an
    /// earlier card refreshes in place.
    pub created_at_unix: u64,
}

#[cfg(test)]
mod tests {
    use super::SessionStatus;

    /// Every input the router can produce, including the two that used to
    /// leak a Feishu reaction name onto the screen (`Get`, `OnIt`) and the
    /// `Active` + empty case that must read Queued rather than Working.
    #[test]
    fn derives_every_status_row() {
        let cases = [
            ("spawning", "", SessionStatus::Starting),
            ("active", "Get", SessionStatus::Queued),
            ("active", "OnIt", SessionStatus::Working),
            ("active", "DONE", SessionStatus::Done),
            ("active", "CrossMark", SessionStatus::Failed),
            ("active", "", SessionStatus::Queued),
            ("dormant", "", SessionStatus::Dormant),
        ];
        for (state, phase, want) in cases {
            assert_eq!(
                SessionStatus::derive(state, phase),
                want,
                "state={state:?} phase={phase:?}"
            );
        }
    }

    /// A dormant session keeps a stale phase in card state; the mapping state
    /// must win, or a closed session would still read "Working".
    #[test]
    fn mapping_state_outranks_a_stale_phase() {
        assert_eq!(
            SessionStatus::derive("dormant", "OnIt"),
            SessionStatus::Dormant
        );
        assert_eq!(
            SessionStatus::derive("spawning", "OnIt"),
            SessionStatus::Starting
        );
    }

    /// add-remote-execution-node 8.4：悬空审批把非终态投影为 Waiting（在等人，
    /// 不是在跑）；终态不因一条历史悬空审批而被改写；0 条不改变任何东西。
    #[test]
    fn parked_approvals_project_a_waiting_status() {
        assert_eq!(
            SessionStatus::Working.with_parked_approvals(2),
            SessionStatus::Waiting
        );
        assert_eq!(
            SessionStatus::Queued.with_parked_approvals(1),
            SessionStatus::Waiting
        );
        assert_eq!(
            SessionStatus::Starting.with_parked_approvals(1),
            SessionStatus::Waiting
        );
        // 终态不被改写：会话已经结束了。
        assert_eq!(
            SessionStatus::Done.with_parked_approvals(1),
            SessionStatus::Done
        );
        assert_eq!(
            SessionStatus::Failed.with_parked_approvals(1),
            SessionStatus::Failed
        );
        // 没有悬空审批 = 原状态。
        assert_eq!(
            SessionStatus::Working.with_parked_approvals(0),
            SessionStatus::Working
        );
    }

    /// Both ends of an id survive truncation, and the result never exceeds
    /// the budget — the point of eliding the middle rather than the tail.
    #[test]
    fn middle_truncate_keeps_both_ends() {
        use super::middle_truncate;
        // Short enough to pass through untouched.
        assert_eq!(middle_truncate("abc", 18), "abc");
        assert_eq!(
            middle_truncate("012345678901234567", 18),
            "012345678901234567"
        );

        let long = "sess_01H2XABCDEFGHJKMNPQRSTVWXYZ";
        let out = middle_truncate(long, 18);
        assert_eq!(out.chars().count(), 18, "must fit the budget exactly");
        assert!(out.starts_with("sess_"), "prefix lost: {out}");
        assert!(out.ends_with("VWXYZ"), "discriminating suffix lost: {out}");
        assert!(out.contains('\u{2026}'));

        // Multi-byte input must not be split mid character.
        let cjk = "\u{4f1a}\u{8bdd}\u{6807}\u{8bc6}\u{7b26}\u{4f1a}\u{8bdd}\u{6807}\u{8bc6}\u{7b26}\u{4f1a}\u{8bdd}\u{6807}\u{8bc6}\u{7b26}";
        assert_eq!(middle_truncate(cjk, 7).chars().count(), 7);
    }

    /// The slug is the `data-status` contract shared with the stylesheet and
    /// the endpoint tests, and the glyph is the non-colour channel. Both must
    /// be distinct per status or the board becomes ambiguous in greyscale.
    #[test]
    fn slugs_and_glyphs_are_distinct() {
        let all = [
            SessionStatus::Starting,
            SessionStatus::Queued,
            SessionStatus::Working,
            SessionStatus::Done,
            SessionStatus::Failed,
            SessionStatus::Dormant,
            SessionStatus::Waiting,
        ];
        let mut slugs: Vec<_> = all.iter().map(|s| s.slug()).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), all.len(), "slugs collide");

        let mut glyphs: Vec<_> = all.iter().map(|s| s.glyph()).collect();
        glyphs.sort_unstable();
        glyphs.dedup();
        assert_eq!(glyphs.len(), all.len(), "glyphs collide");

        for s in all {
            assert_eq!(s.slug(), s.label().to_lowercase(), "slug must match label");
        }
    }
}
