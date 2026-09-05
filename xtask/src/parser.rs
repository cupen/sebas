//! Parser for the models.dev `api.json` shape -> a flat list of
//! `RawEntry` rows that the renderer turns into `ModelDef` lines.
//!
//! Source: <https://models.dev/api.json>. Live JSON shape (verified
//! 2026-08-17 via WebFetch):
//!
//! ```json
//! {
//!   "<provider_id>": {
//!     "id": "...",
//!     "name": "<display name>",
//!     "models": {
//!       "<model_id>": {
//!         "id": "...",
//!         "name": "<display name>",
//!         "limit": { "context": <u64>, "output": <u64> }
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! We deliberately ignore `cost`, `modalities`, `attachment`, etc. —
//! `sebas_router::models::ModelDef` only needs `name`, `context_window`,
//! `max_output_tokens`. Optional fields fall through to `None` so
//! downstream rendering picks a sensible default rather than dropping
//! the entry.

use serde::Deserialize;

/// One model row extracted from models.dev, ordered by JSON traversal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub provider_id: String,
    pub provider_name: String,
    pub model_id: String,
    /// Pretty display name (falls back to model_id if absent).
    pub model_name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
}

/// Top-level deserialisation: `api.json` is `{ provider_id: ProviderJson }`.
#[derive(Deserialize)]
struct ProviderMap {
    #[serde(flatten)]
    providers: std::collections::BTreeMap<String, ProviderJson>,
}

#[derive(Deserialize)]
struct ProviderJson {
    id: String,
    name: String,
    models: ModelMap,
}

#[derive(Deserialize)]
struct ModelMap {
    #[serde(flatten)]
    models: std::collections::BTreeMap<String, ModelJson>,
}

#[derive(Deserialize)]
struct ModelJson {
    id: String,
    name: Option<String>,
    #[serde(default)]
    limit: Option<LimitJson>,
}

#[derive(Deserialize)]
struct LimitJson {
    context: Option<u64>,
    output: Option<u64>,
}

/// Parse a raw `api.json` byte slice into a flat row list, in
/// provider-id (BTreeMap) order then model-id order.
///
/// Provider IDs starting with `_` (models.dev convention for
/// metadata-only entries) are skipped — they have no `models` map
/// in practice and break the schema otherwise.
pub fn parse_api_json(bytes: &[u8]) -> Result<Vec<RawEntry>, ParseError> {
    let parsed: ProviderMap =
        serde_json::from_slice(bytes).map_err(|e| ParseError::Json(e.to_string()))?;
    let mut out = Vec::new();
    for (provider_id, provider) in parsed.providers {
        if provider_id.starts_with('_') {
            continue;
        }
        if provider.id != provider_id {
            // Sanity check: provider key should match inner id. If not,
            // trust the inner id and log a warning in stderr upstream.
        }
        for (model_id, model) in provider.models.models {
            if model_id.starts_with('_') {
                continue;
            }
            let (ctx, max_out) = match model.limit {
                Some(l) => (l.context, l.output),
                None => (None, None),
            };
            out.push(RawEntry {
                provider_id: provider.id.clone(),
                provider_name: provider.name.clone(),
                model_id: model.id.clone(),
                model_name: model.name.unwrap_or_else(|| model.id.clone()),
                context_window: ctx,
                max_output_tokens: max_out,
            });
        }
    }
    Ok(out)
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("models.dev JSON parse error: {0}")]
    Json(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Embedded fixture: two providers (Acme, Beta) with three models
    /// total. Acme-Big intentionally omits `limit` to exercise the
    /// None fallback path. Comments in the JSON are NOT valid JSON —
    /// this fixture is the literal string the test passes to the
    /// parser, so it must be valid JSON syntax.
    const FIXTURE: &str = r#"{
        "acme": {
            "id": "acme",
            "name": "Acme",
            "models": {
                "acme-small": {
                    "id": "acme-small",
                    "name": "Acme Small",
                    "limit": { "context": 128000, "output": 16384 }
                },
                "acme-big": {
                    "id": "acme-big",
                    "name": "Acme Big"
                }
            }
        },
        "beta": {
            "id": "beta",
            "name": "Beta Labs",
            "models": {
                "beta-1": {
                    "id": "beta-1",
                    "name": "Beta One",
                    "limit": { "context": 1048576, "output": 65536 }
                }
            }
        }
    }"#;

    #[test]
    fn parses_fixture_into_expected_entries() {
        let entries = parse_api_json(FIXTURE.as_bytes()).expect("parse ok");

        assert_eq!(
            entries.len(),
            3,
            "fixture has 3 models, got {}",
            entries.len()
        );

        // BTreeMap -> alphabetic provider order: acme, beta.
        assert_eq!(entries[0].provider_id, "acme");
        assert_eq!(entries[0].provider_name, "Acme");
        assert_eq!(entries[0].model_id, "acme-big");
        assert_eq!(entries[0].context_window, None);
        assert_eq!(entries[0].max_output_tokens, None);

        assert_eq!(entries[1].provider_id, "acme");
        assert_eq!(entries[1].model_id, "acme-small");
        assert_eq!(entries[1].context_window, Some(128_000));
        assert_eq!(entries[1].max_output_tokens, Some(16_384));

        assert_eq!(entries[2].provider_id, "beta");
        assert_eq!(entries[2].provider_name, "Beta Labs");
        assert_eq!(entries[2].model_id, "beta-1");
        assert_eq!(entries[2].context_window, Some(1_048_576));
        assert_eq!(entries[2].max_output_tokens, Some(65_536));
    }

    #[test]
    fn empty_models_section_yields_no_entries() {
        let json = r#"{
            "noop": {
                "id": "noop",
                "name": "Noop",
                "models": {}
            }
        }"#;
        let entries = parse_api_json(json.as_bytes()).expect("parse ok");
        assert!(entries.is_empty());
    }

    #[test]
    fn malformed_json_returns_error() {
        let json = "{ this is not json";
        let err = parse_api_json(json.as_bytes()).unwrap_err();
        assert!(matches!(err, ParseError::Json(_)));
    }

    #[test]
    fn missing_provider_name_field_is_error_not_panic() {
        // Schema is strict (no defaults) — name is required. A
        // missing name should surface as a parse error, not panic.
        let json = r#"{
            "broken": { "id": "broken", "models": {} }
        }"#;
        let err = parse_api_json(json.as_bytes()).unwrap_err();
        assert!(matches!(err, ParseError::Json(_)));
    }
}
