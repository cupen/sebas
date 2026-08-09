//! Persistent settings file: `~/.sebas/settings.json`.
//!
//! Full-snapshot semantics: each write serializes the entire `CardConfig`.
//! On startup, the in-memory config is the file content (which itself was
//! the TOML defaults at the time of first write). Strict parse: malformed
//! JSON or wrong-typed fields cause `load_settings` to return an error so
//! `run::run` can refuse to start with a clear message.
//!
//! `load_settings` returns `Ok(None)` (not `Err` and not `Some(default())`)
//! when the file is absent: TOML `[card]` is the bootstrap and the caller
//! is responsible for falling back to it. This avoids silently overwriting
//! a TOML-tuned `theme_color` / `max_user_text_chars` / `thinking` with the
//! default on every restart where the user hasn't yet run `/settings`.

use feishu::cards::CardConfig;
use std::path::{Path, PathBuf};

/// `~/.sebas/settings.json`, expanded at call time so the env is honoured.
/// Falls back to `./.sebas/settings.json` when home can't be resolved
/// (Windows without HOME, or sandboxed test envs).
pub fn settings_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".sebas").join("settings.json")
}

/// Read + parse settings.json.
/// - `Ok(Some(cfg))` — file present and parses; this wins wholesale over TOML.
/// - `Ok(None)` — file absent; caller should use its TOML bootstrap.
/// - `Err(msg)` — present but malformed / wrong-typed fields / IO error.
///   The caller decides whether to refuse to start.
pub fn load_settings(path: &Path) -> Result<Option<CardConfig>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s)
            .map(Some)
            .map_err(|e| format!("settings.json 解析失败 ({}): {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读取 settings.json 失败: {e}")),
    }
}

/// Pretty-print the full CardConfig to the file. Creates parent dirs.
/// Uses write-to-temp + rename for atomicity so a crash mid-write can't
/// leave a half-written settings.json on disk.
pub fn save_settings(path: &Path, cfg: &CardConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 settings 父目录失败: {e}"))?;
    }
    let s = serde_json::to_string_pretty(cfg)
        .map_err(|e| format!("序列化 settings 失败: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, s).map_err(|e| format!("写 settings 临时文件失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename settings 失败: {e}"))?;
    Ok(())
}