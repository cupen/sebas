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
}

impl SessionStatus {
    /// Derive from a mapping state discriminant and the raw card phase.
    ///
    /// `state` is `"active"`, `"spawning"` or `"dormant"`; `phase` is a
    /// `router::card_state::phase` constant, or empty when the router has no
    /// card state for the session yet. An active session with no phase is
    /// Queued, not Working: the child process exists but has not produced
    /// anything, and claiming otherwise would be a lie the operator acts on.
    pub fn derive(state: &str, phase: &str) -> Self {
        match state {
            "spawning" => Self::Starting,
            "dormant" => Self::Dormant,
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
    /// URL-encoded SessionKey (chat_id\0thread_id).
    pub encoded_key: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
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
    /// True if this session is the WebUI's currently focused one. The
    /// template renders an "active" indicator and gates the Switch button.
    pub is_active: bool,
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

/// Gateway info for the settings page.
#[derive(Debug, Clone, Serialize, Default)]
pub struct GatewayInfo {
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
    pub base_url_anthropic: Option<String>,
    pub base_url_openai: Option<String>,
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

/// Card element for rendering in session detail.
#[derive(Debug, Clone, Serialize)]
pub struct CardElementView {
    pub element_type: &'static str,
    pub content: String,
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

    /// Both ends of an id survive truncation, and the result never exceeds
    /// the budget — the point of eliding the middle rather than the tail.
    #[test]
    fn middle_truncate_keeps_both_ends() {
        use super::middle_truncate;
        // Short enough to pass through untouched.
        assert_eq!(middle_truncate("abc", 18), "abc");
        assert_eq!(middle_truncate("012345678901234567", 18), "012345678901234567");

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
