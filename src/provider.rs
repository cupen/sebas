//! `/provider` 命令背后的 provider CRUD：表单 schema、config.toml 种子、
//! overlay 路径与实例构造。
//!
//! 数据流：`/provider` 列表的种子来自 config.toml 的 `[gateway.providers.*]`
//! （只读，不改写）；bot 里新增/修改/删除的变更以 delta 形式持久化到
//! `~/.sebas/providers.json`（overlay）。gateway 启动时把同一份 overlay
//! 合并进自身配置（见 `gateway::config::GatewayConfig::merge_provider_overlay`），
//! 实现「在飞书里改 provider，gateway 重启后生效」。
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

/// provider 表单 schema（与 gateway `ProviderConfig` 字段对齐；
/// 密钥只收 env 变量名，与仓库「密钥不落盘」的安全约定一致）。
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
                required: true,
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

/// 构造 provider CRUD 表单：种子来自 config.toml 的 `[gateway]` providers，
/// 变更持久化到 overlay 文件。加载失败时返回 None（`/provider` 退化为帮助）。
pub fn build_form(raw_config: &str) -> Option<Arc<CrudForm<FileStore>>> {
    let seed = match GatewayConfig::parse(raw_config) {
        Ok(g) => g
            .providers
            .iter()
            .map(|(name, p)| item_from_provider(name, p))
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "provider 种子解析失败（config.toml 缺 [gateway] 段），从空列表开始");
            Vec::new()
        }
    };
    let path = overlay_path();
    match FileStore::load(path.clone(), ID_FIELD, seed) {
        Ok(store) => Some(Arc::new(CrudForm::new(spec(), ID_FIELD, store))),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "provider overlay 加载失败；/provider 不可用");
            None
        }
    }
}
