//! Render a flat `Vec<RawEntry>` -> the body of the `MODELS` const
//! in `gateway/src/models.rs`, and patch the existing file's
//! timestamp comment + const body in place.

use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::Utc;

use crate::parser::RawEntry;

/// Format an unsigned integer with `_` separators every 3 digits,
/// matching the convention used by the hand-curated `MODELS` array
/// (e.g. `1_000_000`, `128_000`, `16_384`). Numbers < 1000 are
/// emitted bare (no underscore).
fn fmt_u64(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, &c) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push('_');
        }
        out.push(c as char);
    }
    out
}

/// Render the body lines that go between `const MODELS: &[ModelDef] = &[`
/// and `];`. Returns one logical line per entry plus per-provider
/// `// ---- Name ----` separators. Trailing newline on the closing
/// `];` line is preserved by the caller.
///
/// Rules:
/// - Entries without `context_window` are dropped (we can't render
///   a meaningful `ModelDef` and silently filling in DEFAULT_CAPS
///   would mask models.dev gaps).
/// - Entries with `max_output_tokens = None` fall back to
///   `context_window / 4` (consistent with `resolve_caps` in
///   `gateway/src/models.rs`).
/// - Provider section separators are emitted once per provider, in
///   the order providers first appear.
pub fn render_models_body(entries: &[RawEntry]) -> String {
    let mut out = String::new();
    let mut current_provider: Option<&str> = None;
    let mut dropped = 0u32;
    for entry in entries {
        let Some(ctx) = entry.context_window else {
            dropped += 1;
            eprintln!(
                "xtask: skipping {}::{} (no context_window in models.dev)",
                entry.provider_id, entry.model_id
            );
            continue;
        };
        let max_out = entry.max_output_tokens.unwrap_or_else(|| ctx / 4);
        if current_provider != Some(entry.provider_name.as_str()) {
            if current_provider.is_some() {
                out.push('\n');
            }
            out.push_str(&format!("    // ---- {} ----\n", entry.provider_name));
            current_provider = Some(entry.provider_name.as_str());
        }
        out.push_str(&format!(
            "    ModelDef {{ name: {:?}, context_window: {}, max_output_tokens: {} }},\n",
            entry.model_id,
            fmt_u64(ctx),
            fmt_u64(max_out),
        ));
    }
    if dropped > 0 {
        eprintln!(
            "xtask: {} total entries dropped (missing context_window)",
            dropped
        );
    }
    out
}

/// Marker that opens the const block we're replacing.
const CONST_OPEN: &str = "const MODELS: &[ModelDef] = &[";
/// Marker line that closes the const block. Matched only when the
/// line starts with this prefix (after trimming leading whitespace).
const CONST_CLOSE_PREFIX: &str = "];";
/// Prefix of the timestamp comment we update on every run.
const TIMESTAMP_PREFIX: &str = "// last synced:";

/// Result of patching `gateway/src/models.rs`.
pub struct PatchReport {
    pub entries_written: usize,
    pub timestamp_line: String,
}

/// Patch `gateway/src/models.rs` in place: update the
/// `// last synced:` line and replace the body of the `MODELS` const
/// with `new_body`. Atomic on disk (write to `.tmp`, fsync, rename).
///
/// Returns an error if either marker is missing — that means the
/// file shape drifted and a human needs to reconcile, not us
/// silently massaging the file into something unexpected.
pub fn patch_models_rs(path: &Path, new_body: &str) -> Result<PatchReport, PatchError> {
    let original = fs::read_to_string(path)
        .map_err(|e| PatchError::Io(path.display().to_string(), e.to_string()))?;

    // 1. Update timestamp comment line.
    let timestamp_line = format!(
        "// last synced: {} from models.dev (xtask update-models)",
        Utc::now().format("%Y-%m-%d")
    );
    let mut patched = String::with_capacity(original.len() + 64);
    let mut timestamp_replaced = false;
    for line in original.lines() {
        if !timestamp_replaced && line.trim_start().starts_with(TIMESTAMP_PREFIX) {
            patched.push_str(&timestamp_line);
            patched.push('\n');
            timestamp_replaced = true;
        } else {
            patched.push_str(line);
            patched.push('\n');
        }
    }
    if !timestamp_replaced {
        return Err(PatchError::MissingTimestamp(path.display().to_string()));
    }

    // 2. Replace const body. Walk lines, find `const MODELS: &[ModelDef] = &[`
    //    then replace every subsequent line up to (but not including) the
    //    first line that begins with `];`.
    let lines: Vec<&str> = patched.lines().collect();
    let const_open_idx = lines
        .iter()
        .position(|l| l.trim_start().starts_with(CONST_OPEN))
        .ok_or_else(|| PatchError::MissingConstOpen(path.display().to_string()))?;
    let const_close_idx = lines
        .iter()
        .enumerate()
        .skip(const_open_idx + 1)
        .find(|(_, l)| l.trim_start().starts_with(CONST_CLOSE_PREFIX))
        .map(|(i, _)| i)
        .ok_or_else(|| PatchError::MissingConstClose(path.display().to_string()))?;

    // Preserve trailing newline of the closing `];` line by appending '\n'
    // separately (lines() strips them).
    let mut final_text = String::with_capacity(patched.len());
    for (i, line) in lines.iter().enumerate() {
        if i < const_open_idx {
            final_text.push_str(line);
            final_text.push('\n');
        } else if i == const_open_idx {
            final_text.push_str(line);
            final_text.push('\n');
            final_text.push_str(new_body);
        } else if i == const_close_idx {
            final_text.push_str(line);
            final_text.push('\n');
        } else {
            // Skip: this is the old body we're replacing.
        }
    }

    // Count entries written (one per `\n` in new_body matching our format).
    let entries_written = new_body.matches("ModelDef { name:").count();

    // Atomic write.
    let tmp = path.with_extension("rs.tmp");
    {
        let mut f = fs::File::create(&tmp)
            .map_err(|e| PatchError::Io(tmp.display().to_string(), e.to_string()))?;
        f.write_all(final_text.as_bytes())
            .map_err(|e| PatchError::Io(tmp.display().to_string(), e.to_string()))?;
        f.sync_all()
            .map_err(|e| PatchError::Io(tmp.display().to_string(), e.to_string()))?;
    }
    fs::rename(&tmp, path)
        .map_err(|e| PatchError::Io(path.display().to_string(), e.to_string()))?;

    Ok(PatchReport {
        entries_written,
        timestamp_line,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    #[error("I/O error on {0}: {1}")]
    Io(String, String),
    #[error("could not find `// last synced:` line in {0}")]
    MissingTimestamp(String),
    #[error("could not find `const MODELS:` opener in {0}")]
    MissingConstOpen(String),
    #[error("could not find matching `];` closer for MODELS const in {0}")]
    MissingConstClose(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str, model: &str, ctx: u64, out: u64) -> RawEntry {
        RawEntry {
            provider_id: provider.to_string(),
            provider_name: provider.to_string(),
            model_id: model.to_string(),
            model_name: model.to_string(),
            context_window: Some(ctx),
            max_output_tokens: Some(out),
        }
    }

    #[test]
    fn fmt_u64_inserts_underscores_every_three_digits() {
        assert_eq!(fmt_u64(0), "0");
        assert_eq!(fmt_u64(999), "999");
        assert_eq!(fmt_u64(1_000), "1_000");
        assert_eq!(fmt_u64(16_384), "16_384");
        assert_eq!(fmt_u64(128_000), "128_000");
        assert_eq!(fmt_u64(1_048_576), "1_048_576");
        assert_eq!(fmt_u64(1_000_000), "1_000_000");
        assert_eq!(fmt_u64(12_345_678), "12_345_678");
    }

    #[test]
    fn render_emits_provider_headers_and_correct_lines() {
        let entries = vec![
            entry("acme", "acme-small", 128_000, 16_384),
            entry("acme", "acme-big", 1_000_000, 64_000),
            entry("beta", "beta-1", 32_768, 8_192),
        ];
        let body = render_models_body(&entries);
        // Assert structural properties individually so a formatter
        // tweak to e.g. trailing-whitespace handling doesn't break
        // unrelated assertions.
        assert!(body.contains("    // ---- acme ----"));
        assert!(body.contains("    // ---- beta ----"));
        // Provider header appears exactly once each.
        assert_eq!(body.matches("// ---- acme ----").count(), 1);
        assert_eq!(body.matches("// ---- beta ----").count(), 1);
        // acme section comes before beta section.
        let acme_pos = body.find("// ---- acme ----").unwrap();
        let beta_pos = body.find("// ---- beta ----").unwrap();
        assert!(acme_pos < beta_pos);
        // Blank line separates provider sections.
        assert!(body.contains("},\n\n    // ---- beta ----"));
        // Every ModelDef line carries the right numbers.
        assert!(body.contains(r#"ModelDef { name: "acme-small", context_window: 128_000, max_output_tokens: 16_384 },"#));
        assert!(body.contains(r#"ModelDef { name: "acme-big", context_window: 1_000_000, max_output_tokens: 64_000 },"#));
        assert!(body.contains(
            r#"ModelDef { name: "beta-1", context_window: 32_768, max_output_tokens: 8_192 },"#
        ));
        // Each line ends with a trailing newline.
        assert!(body.ends_with('\n'));
    }

    #[test]
    fn render_skips_entries_without_context_window() {
        let entries = vec![
            entry("acme", "good", 128_000, 16_384),
            RawEntry {
                provider_id: "acme".into(),
                provider_name: "acme".into(),
                model_id: "no-limits".into(),
                model_name: "no-limits".into(),
                context_window: None,
                max_output_tokens: None,
            },
        ];
        let body = render_models_body(&entries);
        assert!(body.contains("\"good\""));
        assert!(!body.contains("\"no-limits\""));
    }

    #[test]
    fn render_falls_back_to_context_quarter_when_output_missing() {
        let entries = vec![RawEntry {
            provider_id: "x".into(),
            provider_name: "x".into(),
            model_id: "x-1".into(),
            model_name: "x-1".into(),
            context_window: Some(80_000),
            max_output_tokens: None,
        }];
        let body = render_models_body(&entries);
        assert!(body.contains("context_window: 80_000, max_output_tokens: 20_000"));
    }

    #[test]
    fn render_empty_input_emits_empty_body() {
        let body = render_models_body(&[]);
        assert_eq!(body, "");
    }

    /// End-to-end: parse a small embedded models.dev fixture into
    /// `RawEntry` rows via the real parser, render those rows into
    /// the MODELS body string, and assert the resulting Rust source
    /// matches what `gateway/src/models.rs` expects. This is the
    /// "parser end-to-end with embedded JSON fixture" test the spec
    /// asks for.
    ///
    /// Models within a provider come out in alphabetic order
    /// (BTreeMap) — the parser makes that contract explicit via
    /// `ProviderMap::providers` and `ModelMap::models`.
    #[test]
    fn end_to_end_parse_fixture_render_models_body() {
        let fixture = r#"{
            "deepseek": {
                "id": "deepseek",
                "name": "DeepSeek V4",
                "models": {
                    "deepseek-v4-pro": {
                        "id": "deepseek-v4-pro",
                        "name": "DeepSeek V4 Pro",
                        "limit": { "context": 1000000, "output": 384000 }
                    },
                    "deepseek-v4-flash": {
                        "id": "deepseek-v4-flash",
                        "name": "DeepSeek V4 Flash",
                        "limit": { "context": 1000000, "output": 384000 }
                    }
                }
            }
        }"#;
        let entries = crate::parser::parse_api_json(fixture.as_bytes()).expect("fixture parse");
        assert_eq!(entries.len(), 2);
        let body = render_models_body(&entries);

        // Exact expected Rust source. Alphabetic order within provider
        // because the parser uses BTreeMap for stability. Built with
        // explicit "\n    " separators — Rust's \-line-continuation
        // strips leading whitespace from the next physical line, so
        // we can't rely on indentation after a `\`.
        let expected = concat!(
            "    // ---- DeepSeek V4 ----\n",
            "    ModelDef { name: \"deepseek-v4-flash\", context_window: 1_000_000, max_output_tokens: 384_000 },\n",
            "    ModelDef { name: \"deepseek-v4-pro\", context_window: 1_000_000, max_output_tokens: 384_000 },\n",
        );
        assert_eq!(body, expected, "rendered MODELS body must match exactly");
    }
}
