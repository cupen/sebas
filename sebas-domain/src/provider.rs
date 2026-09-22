//! Provider 域：状态词表中仍是词表的部分 + providers.json overlay 读取器
//! （add-domain-layer 3.3，design D5）。
//!
//! 只搬**词表与 wire 形状**：`PersistedState` / `Item` 这类注定被
//! `extract-sebas-db` 的 ActiveRecord 取代的无类型 JSON 载体**不进**域层
//! （design D5 已裁决）；SQLite 连接配方、schema 注册表、迁移 diff 归
//! `extract-sebas-db`。JSON 文件持久化的归属因此横跨两个 crate——已记录
//! 在案，`extract-sebas-db` 可再迁。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Provider 路由模式（runtime 决策）：
/// - `Off`：直连 sebas 自带的 default 模型，跳过 router。
/// - `Direct { provider }`：把请求路由到名为 `provider` 的 provider，
///   但不经过 router（直连上游）。
/// - `Router`：所有请求走 router（router 自己负责选 provider）。
///
/// 注意：与 router 内部 `RouterConfig.mode`（`off`/`upstream`）语义
/// 不完全相同 —— 这里是 sebas 这一侧对 spawn 路径的开关。
#[derive(Default, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderMode {
    /// Default = Off（无路由配置时诚实降级）。
    #[default]
    Off,
    Direct {
        provider: String,
    },
    Router,
}

/// openspec/specs/provider-management/spec.md：DIRECT 模式默认 (provider, model)。
///
/// `model` 缺省 / 显式 None 时不写 `--model`（agent 用自己默认）。
///
/// **serde 自定义反序列化**：为了把旧 `default_provider_for_direct: "<name>"`
/// 形态的 state.json 平滑迁到新形状，`DefaultSelection::deserialize` 同时
/// 接受：
/// - 对象 `{"provider": "...", "model": "..."}`（新）
/// - 字符串 `"<provider>"`（旧 default_provider_for_direct 别名走这条）
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DefaultSelection {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl DefaultSelection {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: None,
        }
    }

    pub fn with_model(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: Some(model.into()),
        }
    }
}

impl<'de> Deserialize<'de> for DefaultSelection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt::{self, Formatter};

        struct StringOrStruct;

        impl<'de> Visitor<'de> for StringOrStruct {
            type Value = DefaultSelection;

            fn expecting(&self, f: &mut Formatter) -> fmt::Result {
                f.write_str(
                    "string (legacy default_provider_for_direct) or \
                     object {\"provider\": \"...\", \"model\": \"...\"} \
                     for DefaultSelection",
                )
            }

            fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
                Ok(DefaultSelection::new(s))
            }

            fn visit_string<E: de::Error>(self, s: String) -> Result<Self::Value, E> {
                Ok(DefaultSelection::new(s))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut provider: Option<String> = None;
                let mut model: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "provider" => provider = Some(map.next_value()?),
                        "model" => model = Some(map.next_value()?),
                        _ => {
                            let _: de::IgnoredAny = map.next_value()?;
                        }
                    }
                }
                let provider = provider.ok_or_else(|| de::Error::missing_field("provider"))?;
                Ok(DefaultSelection { provider, model })
            }
        }

        deserializer.deserialize_any(StringOrStruct)
    }
}

/// providers.json `model_aliases` 段的单个别名 wire（与 router admin API
/// 同形状；原 sebas-dispatch::state_store 定义处）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelAliasEntry {
    pub provider: String,
    /// 缺省 = 别名即 upstream model（透传）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_model: Option<String>,
}

/// provider overlay 文件的 wire 结构（providers.json / core channel state
/// snapshot 的 `providers`/`deleted`/`model_aliases` 三段）。
///
/// 原为 `sebas-router/src/config.rs` 的私有副本（add-domain-layer 3.3 删除
/// 重复）：`providers` 段保持**无类型 JSON 对象**——条目的字段校验
/// （preset / *_base_url / api_key_env / api_key）由消费方自己的
/// raw→resolved 管线负责（router 的 `validate_provider_entry`），域层只
/// 承载文件形状。未知段（未来扩展）由 serde 默默忽略——与 `save_overlay`
/// 的 Map 级 RMW「保留未知段」语义配套。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProviderOverlay {
    #[serde(default)]
    pub providers: HashMap<String, serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub deleted: Vec<String>,
    /// 模型别名：`alias -> { provider, upstream_model? }`。由 admin API /
    /// 手工编辑写入；引用不存在 provider 的别名由消费方在合并期 drop + warn。
    #[serde(default)]
    pub model_aliases: HashMap<String, ModelAliasEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_mode_serializes_with_kind_tag() {
        // 关键不变量：snake_case tag 让 JSON 是稳定字符串而非 enum index。
        assert_eq!(
            serde_json::to_value(ProviderMode::Off).unwrap(),
            serde_json::json!({"kind": "off"})
        );
        assert_eq!(
            serde_json::to_value(ProviderMode::Direct {
                provider: "x".into()
            })
            .unwrap(),
            serde_json::json!({"kind": "direct", "provider": "x"})
        );
        assert_eq!(
            serde_json::to_value(ProviderMode::Router).unwrap(),
            serde_json::json!({"kind": "router"})
        );
    }

    #[test]
    fn default_selection_accepts_legacy_string_and_object() {
        let legacy: DefaultSelection = serde_json::from_str("\"legacy\"").unwrap();
        assert_eq!(legacy, DefaultSelection::new("legacy"));
        let modern: DefaultSelection =
            serde_json::from_str(r#"{"provider": "p", "model": "m"}"#).unwrap();
        assert_eq!(modern, DefaultSelection::with_model("p", "m"));
        // 序列化只上新形状；model 缺省不上 wire。
        assert_eq!(
            serde_json::to_value(&DefaultSelection::new("solo")).unwrap(),
            serde_json::json!({"provider": "solo"})
        );
    }

    #[test]
    fn overlay_wire_shape_parses_all_three_sections() {
        let raw = r#"{
            "providers": {"anthropic": {"api_key": "sk-x"}},
            "deleted": ["gone"],
            "model_aliases": {"fast": {"provider": "deepseek"}},
            "unknown_section": {"kept": "by RMW"}
        }"#;
        let ov: ProviderOverlay = serde_json::from_str(raw).unwrap();
        assert_eq!(ov.providers.len(), 1);
        assert_eq!(ov.deleted, vec!["gone".to_string()]);
        assert_eq!(ov.model_aliases["fast"].provider, "deepseek");
        assert!(ov.model_aliases["fast"].upstream_model.is_none());
    }

    #[test]
    fn empty_json_is_a_valid_empty_overlay() {
        let ov: ProviderOverlay = serde_json::from_str("{}").unwrap();
        assert!(ov.providers.is_empty() && ov.deleted.is_empty() && ov.model_aliases.is_empty());
    }
}
