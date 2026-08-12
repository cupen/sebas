//! Data models for the WebUI dashboard.

use serde::Serialize;

/// A single session row for the dashboard / session list.
#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    /// URL-encoded SessionKey (chat_id\0thread_id).
    pub encoded_key: String,
    pub chat_id: String,
    pub thread_id: Option<String>,
    /// The ACP session_id, if active/dormant.
    pub session_id: Option<String>,
    /// "active", "spawning", or "dormant".
    pub status: &'static str,
    /// "SEED", "WORKING", "DONE", "FAILED".
    pub phase: String,
    /// Human-readable relative time.
    pub last_active: String,
}

/// Dashboard overview data.
#[derive(Debug, Clone, Serialize)]
pub struct DashboardData {
    pub active_count: usize,
    pub dormant_count: usize,
    pub spawning_count: usize,
    pub total_sessions: usize,
    pub uptime_seconds: i64,
    pub recent_sessions: Vec<SessionRow>,
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