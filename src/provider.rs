//! `/provider` 命令背后的 provider CRUD：表单 schema、config.toml 种子、
//! overlay 路径与实例构造。
//!
//! 数据流：`/provider` 列表的种子来自 config.toml 的顶层 `[provider.*]`
//! （只读，不改写，支持 preset 惯例默认）；bot 里新增/修改/删除的变更以
//! delta 形式持久化到 `~/.sebas/providers.json`（overlay）。gateway 启动时
//! 把同一份 overlay 合并进自身配置
//! （见 `gateway::config::GatewayConfig::merge_provider_overlay`），实现
//! 「在飞书里改 provider，gateway 重启后生效」。
//!
//! 密钥策略：表单直接收 `api_key`（飞书里无法设置环境变量）。密钥存进
//! overlay 文件（~/.sebas/providers.json），列表/日志中掩码回显；如需
//! 更严格的落盘隔离，可后续把密钥挪到独立 secrets 文件。

use feishu::forms::{FormField, FormSpec, SelectOption};
use gateway::config::GatewayConfig;
use router::crud::{CrudForm, FileStore, Item};
use serde_json::{Map, Value};
use std::sync::Arc;

pub const FORM_NAME: &str = "provider";
pub const ID_FIELD: &str = "name";

/// provider 表单 schema（与 gateway `ProviderConfig` 字段对齐）。
/// `preset` 选填：选中的 preset 会在提交时按协议补全 base_url，并在未显式
/// 指定任何密钥来源时注入 preset 默认 env 名（与 config.toml 的解析行为一致）。
pub fn spec() -> FormSpec {
    FormSpec::new(
        FORM_NAME,
        "Provider",
        vec![
            FormField::Text {
                name: "name".into(),
                label: "名称".into(),
                required: true,
                placeholder: "如 deepseek".into(),
                secret: false,
            },
            FormField::Select {
                name: "preset".into(),
                label: "预设（可选）".into(),
                required: false,
                options: std::iter::once(SelectOption {
                    value: String::new(),
                    label: "无（自定义）".into(),
                })
                .chain(gateway::config::presets().iter().map(|p| SelectOption {
                    value: p.name.to_string(),
                    label: p.name.to_string(),
                }))
                .collect(),
            },
            FormField::Select {
                name: "protocol".into(),
                label: "协议".into(),
                required: true,
                options: vec![
                    SelectOption {
                        value: "anthropic".into(),
                        label: "Anthropic".into(),
                    },
                    SelectOption {
                        value: "openai".into(),
                        label: "OpenAI".into(),
                    },
                ],
            },
            FormField::Text {
                name: "base_url".into(),
                label: "Base URL".into(),
                // 选 preset 时可留空，提交时按协议自动填充。
                required: false,
                placeholder: "https://api.xxx.com".into(),
                secret: false,
            },
            FormField::Text {
                name: "api_key".into(),
                label: "API Key".into(),
                required: false,
                placeholder: "粘贴 API Key（保存后不回显）".into(),
                secret: true,
            },
            FormField::Text {
                name: "api_key_env".into(),
                label: "API Key 环境变量".into(),
                required: false,
                placeholder: "如 DEEPSEEK_API_KEY".into(),
                secret: false,
            },
        ],
    )
}

/// 把 gateway 配置里的 provider 转成 CRUD item（种子）。
pub fn item_from_provider(name: &str, p: &gateway::config::ProviderConfig) -> Item {
    let mut m = Map::new();
    m.insert("name".into(), Value::String(name.into()));
    m.insert("protocol".into(), Value::String(p.protocol.as_str().into()));
    m.insert("base_url".into(), Value::String(p.base_url.clone()));
    if let Some(key) = &p.api_key {
        m.insert("api_key".into(), Value::String(key.clone()));
    }
    if let Some(env) = &p.api_key_env {
        m.insert("api_key_env".into(), Value::String(env.clone()));
    }
    m
}

/// provider overlay 路径：默认 `~/.sebas/providers.json`，
/// 可用 `SEBAS_GATEWAY_PROVIDER_OVERLAY` 覆盖（与 gateway 侧一致）。
pub fn overlay_path() -> std::path::PathBuf {
    let raw = std::env::var("SEBAS_GATEWAY_PROVIDER_OVERLAY")
        .unwrap_or_else(|_| "~/.sebas/providers.json".into());
    std::path::PathBuf::from(crate::config::expand_tilde(&raw))
}

/// 构造 provider CRUD 表单：种子来自 config.toml 的顶层 `[provider.*]`，
/// 变更持久化到 overlay 文件。加载失败时返回 None（`/provider` 退化为帮助）。
pub fn build_form(raw_config: &str) -> Option<Arc<CrudForm<FileStore>>> {
    let seed = match GatewayConfig::parse(raw_config) {
        Ok(g) => g
            .providers
            .iter()
            .map(|(name, p)| item_from_provider(name, p))
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "provider 种子解析失败（config.toml 缺 [gateway] 或 [provider] 段），从空列表开始");
            Vec::new()
        }
    };
    let path = overlay_path();
    match FileStore::load(path.clone(), ID_FIELD, seed) {
        Ok(store) => Some(Arc::new(
            CrudForm::new(spec(), ID_FIELD, store).with_normalizer(Arc::new(apply_preset_defaults)),
        )),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "provider overlay 加载失败；/provider 不可用");
            None
        }
    }
}

/// 提交规范化：选中 preset 时补全默认值（与 config.toml preset 解析一致）。
/// - base_url 留空 → 按已选 protocol 填 preset 端点；protocol 留空且 preset
///   单协议 → 顺带填 protocol；
/// - api_key / api_key_env 都没填 → 注入 preset 默认 env 名。
fn apply_preset_defaults(item: &mut Item) {
    let Some(preset_name) = item.get("preset").and_then(Value::as_str) else {
        return;
    };
    let Some(p) = gateway::config::presets()
        .iter()
        .find(|p| p.name == preset_name)
    else {
        return;
    };

    let protocol = item
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let base_url_empty = item
        .get("base_url")
        .and_then(Value::as_str)
        .is_none_or(|s| s.is_empty());
    if base_url_empty {
        let base = match protocol {
            "anthropic" => p.anthropic_base_url,
            "openai" => p.openai_base_url,
            _ => None,
        };
        if let Some(b) = base {
            item.insert("base_url".into(), Value::String(b.to_string()));
        }
    }

    let has_key = item
        .get("api_key")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    let has_env = item
        .get("api_key_env")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty());
    if !has_key && !has_env {
        item.insert(
            "api_key_env".into(),
            Value::String(p.api_key_env.to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_with(fields: &[(&str, &str)]) -> Item {
        let mut m = Map::new();
        for (k, v) in fields {
            m.insert((*k).into(), Value::String((*v).into()));
        }
        m
    }

    #[test]
    fn deepseek_preset_fills_base_url_and_default_env() {
        let mut item = item_with(&[
            ("name", "deepseek"),
            ("preset", "deepseek"),
            ("protocol", "anthropic"),
        ]);
        apply_preset_defaults(&mut item);
        assert_eq!(
            item.get("base_url").and_then(Value::as_str),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(
            item.get("api_key_env").and_then(Value::as_str),
            Some("DEEPSEEK_API_KEY")
        );
    }

    #[test]
    fn explicit_base_url_and_key_override_preset() {
        let mut item = item_with(&[
            ("name", "deepseek"),
            ("preset", "deepseek"),
            ("protocol", "anthropic"),
            ("base_url", "http://localhost:9999"),
            ("api_key", "sk-test"),
        ]);
        apply_preset_defaults(&mut item);
        // 显式字段不被 preset 覆盖。
        assert_eq!(
            item.get("base_url").and_then(Value::as_str),
            Some("http://localhost:9999")
        );
        // 显式 api_key 时不注入默认 env（否则 resolve_api_keys 会读错 env）。
        assert!(item.get("api_key_env").is_none());
    }

    #[test]
    fn no_preset_leaves_item_untouched() {
        let mut item = item_with(&[("name", "my-custom"), ("protocol", "openai")]);
        apply_preset_defaults(&mut item);
        assert!(item.get("base_url").is_none());
        assert!(item.get("api_key_env").is_none());
    }
}
