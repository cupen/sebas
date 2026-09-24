//! `/provider` 命令背后的 provider CRUD：表单 schema、config.toml 种子与
//! 实例构造。
//!
//! 数据流（retire-legacy-state-json 3.1）：`/provider` 列表的种子来自
//! config.toml 的顶层 `[provider.*]`（只读，不改写，支持 preset 惯例默认），
//! 叠加**状态库**里的 provider 表；bot 里新增/修改/删除的变更写进状态库。
//! legacy overlay 文件（`~/.sebas/providers.json`）已退休——既不读也不写；
//! router 侧经 core 通道拿到同一份状态并热生效（不再有 overlay 合并）。
//!
//! 密钥策略：表单直接收 `api_key`（飞书里无法设置环境变量）。密钥落状态库
//! （库文件 owner-only 0600），列表/日志中掩码回显。
//!
//! `default_model`：bot 侧的「spawn 时落到 agent 的默认 model」选择，仅
//! 写入 overlay（不落 router `ProviderConfig`），由后续
//! `ClaudeCodeDriver::resolve_args()`（bead sebas-63f.8）传给 agent。表单里是
//! 手填文本框（preset / custom 一致）——model 列表的权威来源是 provider
//! 官方 `/models` 接口（详情面板的「🔍 探测 model 列表」按钮），静态
//! preset 表里的型号很快就会过时；探测不可用时手填兜底。

use sebas_dispatch::crud::{CrudForm, FileStore, Item};
use sebas_feishu::cards::{
    CardElement, CardText, CollapsiblePanel, CollapsiblePanelHeader, StandardIcon,
};
use sebas_feishu::forms::{FormField, FormSpec, SelectOption};
use sebas_router::config::RouterConfig;
use serde_json::{Map, Value, json};

use std::sync::Arc;

/// 预设表单：用户从代码里写好的 provider 里选一个，只填名称 + 密钥 +
/// 默认 model + 协议。preset 的 base_url / models 跟随代码表，不进表单、
/// 不落盘（提交时 normalizer 会剥掉可能残留的 url/models 字段）。
pub const FORM_PRESET: &str = "provider-preset";
/// 自定义表单：用户手填所有参数（与原单表单形态一致）。
pub const FORM_CUSTOM: &str = "provider-custom";
pub const ID_FIELD: &str = "name";

/// 预设模式表单：name + preset + api_key + default_model + protocol。
///
/// preset 决定的只读详情（三槽位 URL / 默认 env 名）放在表单独立的
/// `CardElement::CollapsiblePanel`（由调用方在卡片 body 上叠加
/// `render_preset_details()`），表单只剩真正要用户填的字段。
///
/// 提交时 `apply_preset_defaults` 只保留用户字段并剥掉 url/models——
/// preset 数据跟随代码，存储侧不再持有副本（resolve/spawn 时从代码表
/// 物化），代码更新 preset 后存量 provider 自动跟随。
pub fn spec_preset() -> FormSpec {
    FormSpec::new(
        FORM_PRESET,
        "Provider（预设）",
        vec![
            FormField::Text {
                name: "name".into(),
                label: "名称".into(),
                required: true,
                placeholder: "如 deepseek".into(),
                secret: false,
                disabled: false,
            },
            FormField::Select {
                name: "preset".into(),
                label: "预设".into(),
                required: true,
                options: sebas_router::config::presets()
                    .iter()
                    .map(|p| SelectOption {
                        value: p.name.to_string(),
                        label: p.name.to_string(),
                    })
                    .collect(),
                on_change: Some(json!({
                    "form": FORM_PRESET,
                    "op": "recompute",
                })),
            },
            FormField::Text {
                name: "api_key".into(),
                label: "API Key".into(),
                required: false,
                placeholder: "粘贴 API Key（保存后不回显）".into(),
                secret: true,
                disabled: false,
            },
            // default_model：preset 的 models 列表跟随代码（表单不再提供
            // catalog 输入），default_model 是用户偏好，权威来源是官方
            // `/models` 探测结果卡。
            FormField::Text {
                name: "default_model".into(),
                label: "默认 model".into(),
                required: false,
                placeholder: "手填 model id；或保存后用「🔍 探测 model 列表」从官方 API 选".into(),
                secret: false,
                disabled: false,
            },
            // 协议选择：Direct 模式的协议优先级。"auto" = 默认（anthropic
            // 优先）；显式 anthropic/openai 强制走对应协议端点，缺失时由
            // spawn_env 显式报错。
            FormField::Select {
                name: "protocol".into(),
                label: "协议".into(),
                required: false,
                options: vec![
                    SelectOption {
                        value: "auto".into(),
                        label: "Auto（Anthropic 优先）".into(),
                    },
                    SelectOption {
                        value: "anthropic".into(),
                        label: "Anthropic".into(),
                    },
                    SelectOption {
                        value: "openai".into(),
                        label: "OpenAI".into(),
                    },
                ],
                on_change: None,
            },
        ],
    )
}

/// 把当前 preset 决定的只读细节渲染成一个折叠面板，给承载 preset 表单的
/// 卡片叠加在表单容器下方（**不放在 FormSpec 里** —— 折叠面板是
/// `CardElement`，不是 `FormField`，spec 里塞不进去）。preset 切换会触发
/// 表单 recompute，调用方重渲卡片时再调一次本函数即可刷新面板内容。
///
/// 找不到对应 preset（例如用户选了不存在的 custom preset）时返回空 vec，
/// 调用方应「不叠加面板」而不是报错。
pub fn render_preset_details(preset_name: &str) -> Vec<CardElement> {
    let Some(p) = sebas_router::config::presets()
        .iter()
        .find(|p| p.name == preset_name)
    else {
        return Vec::new();
    };

    let url_anthropic = p.base_url_anthropic.unwrap_or("—");
    let url_openai_chat = p.base_url_openai_chat.unwrap_or("—");
    let url_openai_responses = p.base_url_openai_responses.unwrap_or("—");

    let lines = [
        format!("**Base URL(Anthropic)**\n`{url_anthropic}`"),
        format!("**Base URL(OpenAI Chat)**\n`{url_openai_chat}`"),
        format!("**Base URL(OpenAI Responses)**\n`{url_openai_responses}`"),
        format!("**默认 env**\n`{}`", p.api_key_env),
    ];
    let elements: Vec<CardElement> = lines
        .into_iter()
        .map(|content| CardElement::Markdown { content })
        .collect();

    vec![CardElement::CollapsiblePanel(CollapsiblePanel {
        expanded: false,
        header: CollapsiblePanelHeader {
            title: CardText {
                tag: "plain_text".into(),
                content: "📋 预设详情（跟随代码）".into(),
            },
            icon: StandardIcon {
                tag: "standard_icon".into(),
                token: "down-small-ccm_outlined".into(),
                size: "16px 16px".into(),
            },
            icon_position: "right".into(),
            icon_expanded_angle: -180,
        },
        elements,
    })]
}

/// 自定义模式表单：所有字段都让用户填（与 router `ProviderConfig` 字段对齐）。
/// 三个 base_url 槽位各自独立，可只填一个（只支持对应协议）。
pub fn spec_custom() -> FormSpec {
    FormSpec::new(
        FORM_CUSTOM,
        "Provider（自定义）",
        vec![
            FormField::Text {
                name: "name".into(),
                label: "名称".into(),
                required: true,
                placeholder: "如 my-openai".into(),
                secret: false,
                disabled: false,
            },
            FormField::Text {
                name: "base_url_anthropic".into(),
                label: "Base URL(Anthropic)".into(),
                required: false,
                placeholder: "留空表示不提供 Anthropic 协议".into(),
                secret: false,
                disabled: false,
            },
            FormField::Text {
                name: "base_url_openai_chat".into(),
                label: "Base URL(OpenAI Chat)".into(),
                required: false,
                placeholder: "chat completions 端点；留空表示不提供".into(),
                secret: false,
                disabled: false,
            },
            FormField::Text {
                name: "base_url_openai_responses".into(),
                label: "Base URL(OpenAI Responses)".into(),
                required: false,
                placeholder: "Responses API 端点；留空表示不提供".into(),
                secret: false,
                disabled: false,
            },
            FormField::Text {
                name: "api_key".into(),
                label: "API Key".into(),
                required: false,
                placeholder: "粘贴 API Key（保存后不回显）".into(),
                secret: true,
                disabled: false,
            },
            FormField::Text {
                name: "api_key_env".into(),
                label: "API Key 环境变量".into(),
                required: false,
                placeholder: "如 MY_OPENAI_API_KEY".into(),
                secret: false,
                disabled: false,
            },
            FormField::Text {
                name: "models".into(),
                label: "模型列表".into(),
                required: false,
                placeholder: "用逗号分隔，从强到弱；或点下方「🔍 获取模型列表」从官方 API 拉取"
                    .into(),
                secret: false,
                disabled: false,
            },
            // default_model：custom provider 不在静态 preset 表里，model 名
            // 无法预填；让用户手填。
            FormField::Text {
                name: "default_model".into(),
                label: "默认 model".into(),
                required: false,
                placeholder: "如 deepseek-chat 或 gpt-4o".into(),
                secret: false,
                disabled: false,
            },
            // 协议选择：custom provider 与 preset 表单字段对齐（三档）。
            FormField::Select {
                name: "protocol".into(),
                label: "协议".into(),
                required: false,
                options: vec![
                    SelectOption {
                        value: "auto".into(),
                        label: "Auto（Anthropic 优先）".into(),
                    },
                    SelectOption {
                        value: "anthropic".into(),
                        label: "Anthropic".into(),
                    },
                    SelectOption {
                        value: "openai".into(),
                        label: "OpenAI".into(),
                    },
                ],
                on_change: None,
            },
        ],
    )
}

/// 兼容老引用（测试 / 旧调用方）。
pub fn spec() -> FormSpec {
    spec_custom()
}

/// 把 router 配置里的 provider 转成 CRUD item（种子）。
///
/// `default_model` 不在 router `ProviderConfig` 上（bead sebas-63f.4）：
/// 用户通过 bot 表单写入的值仅落在 overlay 文件，不向 router 同步。
/// 后续 `ClaudeCodeDriver::resolve_args()`（bead sebas-63f.8）会从 overlay 读到
/// 这个值传给 agent。这里不写入 `default_model`，让表单编辑时初始为空；
/// overlay 里已有 `default_model` 的项会在 `item_to_initial` 里被预填。
pub fn item_from_provider(name: &str, p: &sebas_router::config::ProviderConfig) -> Item {
    let mut m = Map::new();
    m.insert("name".into(), Value::String(name.into()));
    if let Some(preset) = &p.preset {
        // preset 派生条目：连接数据跟随代码，只带 preset 名 + 用户字段
        // （url/models 一概不落盘，代码更新 preset 后存量条目自动跟随）。
        m.insert("preset".into(), Value::String(preset.clone()));
    } else {
        // 自定义 provider：url/models 是用户数据，落盘。
        if let Some(u) = &p.base_url_anthropic {
            m.insert("base_url_anthropic".into(), Value::String(u.clone()));
        }
        if let Some(u) = &p.base_url_openai_chat {
            m.insert("base_url_openai_chat".into(), Value::String(u.clone()));
        }
        if let Some(u) = &p.base_url_openai_responses {
            m.insert("base_url_openai_responses".into(), Value::String(u.clone()));
        }
        if !p.models.is_empty() {
            // 卡片表单的 models 字段是逗号分隔文本（条目 id 视图）；真正
            // 落库经 store 归一化为条目对象（redesign-provider-models-settings
            // 1.1）。
            m.insert("models".into(), Value::String(p.model_ids().join(",")));
        }
    }
    if let Some(key) = &p.api_key {
        m.insert("api_key".into(), Value::String(key.clone()));
    }
    if let Some(env) = &p.api_key_env {
        m.insert("api_key_env".into(), Value::String(env.clone()));
    }
    m
}

/// `/provider` 命令的两张表单（共享同一个 overlay 存储）。
/// 定义在 router 里；sebas root crate 只是装配。
pub use sebas_dispatch::crud::ProviderForms;

/// 构造两套 provider CRUD 表单：种子来自 config.toml 的顶层 `[provider.*]`，
/// 变更持久化到状态库（详见 openspec/specs/provider-management/spec.md 与
/// `sebas_dispatch::state_store`）。
///
/// retire-legacy-state-json 3.1：legacy overlay 文件（`providers.json`）已退休
/// ——不再有「先校验 / 备份损坏 overlay，再从它迁移」这一步。provider 数据的
/// 唯一权威是状态库；`FileStore::load` 委托 `state_store::load`，库不可用时按
/// 默认呈现（并由状态库侧点名成因），**不**读任何文件。
///
/// 返回 `None` 只在「构造表单本身失败」时（今天不会发生——`FileStore::load`
/// 不再有 IO 失败面），保留该形状让 `/provider` 的「不可用」呈现路径不消失。
pub fn build_form(raw_config: &str) -> Option<Arc<ProviderForms>> {
    let seed = match RouterConfig::parse(raw_config) {
        Ok(g) => g
            .providers
            .iter()
            .map(|(name, p)| item_from_provider(name, p))
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "failed to parse provider seed from config.toml ([router] / [provider.*] are optional), starting empty");
            Vec::new()
        }
    };
    match FileStore::load("(state store)", ID_FIELD, seed) {
        Ok(store) => Some(Arc::new(make_forms(store))),
        Err(e) => {
            tracing::warn!(error = %e, "failed to load provider store, /provider unavailable");
            None
        }
    }
}

/// `build_form` 的成功路径 — 抽出来让「构造 ProviderForms」只有一处。
fn make_forms(store: FileStore) -> ProviderForms {
    ProviderForms {
        preset: Arc::new(
            CrudForm::new(spec_preset(), ID_FIELD, store.clone())
                .with_normalizer(Arc::new(apply_preset_defaults)),
        ),
        custom: Arc::new(
            CrudForm::new(spec_custom(), ID_FIELD, store)
                .with_normalizer(Arc::new(noop_normalizer)),
        ),
    }
}

/// 自定义模式表单的 normalizer：什么都不做（用户已填全所有字段）。
/// 保留签名一致让两套表单可以共享 `with_normalizer` 调用点。
fn noop_normalizer(_item: &mut Item) {}

/// 提交规范化（preset 表单）：preset 数据跟随代码——剥掉条目上可能残留的
/// base_url / models 字段（router 校验会拒绝 preset 派生条目携带它们），
/// 用户字段（api_key / api_key_env / default_model / protocol）原样保留。
/// 默认 env 名不再注入：router resolve 与 Direct spawn 都会在缺 key 来源时
/// 从代码表物化 preset 的 `api_key_env`，落盘副本只会变陈旧。
fn apply_preset_defaults(item: &mut Item) {
    if item.get("preset").and_then(Value::as_str).is_none() {
        return;
    }
    for field in [
        "base_url_anthropic",
        "base_url_openai_chat",
        "base_url_openai_responses",
        "models",
    ] {
        item.remove(field);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sebas_dispatch::CrudStore;

    fn item_with(fields: &[(&str, &str)]) -> Item {
        let mut m = Map::new();
        for (k, v) in fields {
            m.insert((*k).into(), Value::String((*v).into()));
        }
        m
    }

    /// 写一个最小 config.toml（含 `[provider.deepseek]` 种子），让
    /// `build_form` 走真实 RouterConfig::parse 路径。
    fn minimal_config_with_seed() -> String {
        r#"
[router]
listen = "127.0.0.1:0"

[provider.deepseek]
"#
        .to_string()
    }

    #[test]
    fn preset_item_keeps_only_user_fields() {
        // preset 数据跟随代码：normalizer 剥掉 url/models，用户字段保留。
        let mut item = item_with(&[("name", "deepseek"), ("preset", "deepseek")]);
        apply_preset_defaults(&mut item);
        assert!(item.get("base_url_anthropic").is_none());
        assert!(item.get("base_url_openai_chat").is_none());
        assert!(item.get("models").is_none());
        assert_eq!(item.get("preset").and_then(Value::as_str), Some("deepseek"));
    }

    #[test]
    fn preset_item_strips_stale_url_fields_and_keeps_key() {
        // 编辑旧条目（或表单夹带 url 字段）时残留的 url/models 被剥掉，
        // api_key 等用户数据不动。
        let mut item = item_with(&[
            ("name", "deepseek"),
            ("preset", "deepseek"),
            ("base_url_anthropic", "https://stale.example/anthropic"),
            ("base_url_openai_chat", "https://stale.example/v1"),
            ("models", "stale-model"),
            ("api_key", "sk-ds"),
            ("default_model", "deepseek-chat"),
        ]);
        apply_preset_defaults(&mut item);
        assert!(item.get("base_url_anthropic").is_none());
        assert!(item.get("base_url_openai_chat").is_none());
        assert!(item.get("models").is_none());
        assert_eq!(item.get("api_key").and_then(Value::as_str), Some("sk-ds"));
        assert_eq!(
            item.get("default_model").and_then(Value::as_str),
            Some("deepseek-chat")
        );
        // 默认 env 名不再落盘（resolve/spawn 期从代码表物化）。
        assert!(
            item.get("api_key_env").is_none(),
            "preset env 名跟随代码，不写入条目"
        );
    }

    #[test]
    fn single_protocol_preset_also_strips_urls() {
        let mut item = item_with(&[
            ("name", "anthropic"),
            ("preset", "anthropic"),
            ("api_key", "sk-anthropic"),
        ]);
        apply_preset_defaults(&mut item);
        assert!(item.get("base_url_anthropic").is_none());
        assert!(item.get("base_url_openai_chat").is_none());
    }

    #[test]
    fn custom_provider_urls_pass_through_preset_normalizer_untouched() {
        // 无 preset 字段的条目（custom 表单也能落到这个 normalizer 的防御
        // 路径）不做任何处理。
        let mut item = item_with(&[
            ("name", "my-custom"),
            ("base_url_anthropic", "http://localhost:9999/anth"),
        ]);
        apply_preset_defaults(&mut item);
        assert_eq!(
            item.get("base_url_anthropic").and_then(Value::as_str),
            Some("http://localhost:9999/anth")
        );
    }

    /// 用户在 preset 表单里挑了一个 model，apply_preset_defaults 不动它
    /// （不强制属于 preset.models，留空时也不自动填）。
    #[test]
    fn default_model_survives_preset_normalizer_unchanged() {
        let mut item = item_with(&[
            ("name", "deepseek"),
            ("preset", "deepseek"),
            ("default_model", "deepseek-reasoner"),
        ]);
        apply_preset_defaults(&mut item);
        assert_eq!(
            item.get("default_model").and_then(Value::as_str),
            Some("deepseek-reasoner"),
            "apply_preset_defaults 不得修改用户选的 default_model"
        );
    }

    /// preset 用户留空 default_model：保留为空，不自动填（避免替用户
    /// 决定走哪个 model）。
    #[test]
    fn empty_default_model_is_not_autofilled() {
        let mut item = item_with(&[("name", "deepseek"), ("preset", "deepseek")]);
        apply_preset_defaults(&mut item);
        assert!(
            item.get("default_model").is_none(),
            "留空时 apply_preset_defaults 不注入默认 model"
        );
    }

    /// 自定义表单 schema 暴露 default_model 文本字段，供 custom provider
    /// 用户手填 model id。
    #[test]
    fn custom_spec_has_default_model_text_field() {
        let spec = spec_custom();
        let field = spec
            .fields
            .iter()
            .find(|f| f.name() == "default_model")
            .expect("custom spec must include default_model");
        assert!(!field.required(), "default_model 在 custom 表单里选填");
        match field {
            FormField::Text {
                placeholder,
                secret,
                disabled,
                ..
            } => {
                assert!(!secret, "default_model 不是敏感字段");
                assert!(!disabled, "default_model 应可编辑");
                assert!(placeholder.contains("deepseek-chat") || placeholder.contains("gpt-4o"));
            }
            FormField::Select { .. } => {
                panic!("custom spec 的 default_model 应是 Text，不是 Select")
            }
        }
    }

    /// preset 表单 schema 把 default_model 暴露成手填 Text（与 custom 表单
    /// 一致）——静态 preset 型号表会过时，权威来源是官方 `/models` 探测。
    #[test]
    fn preset_spec_default_model_is_text_field() {
        let spec = spec_preset();
        let field = spec
            .fields
            .iter()
            .find(|f| f.name() == "default_model")
            .expect("preset spec must include default_model");
        match field {
            FormField::Text {
                placeholder,
                secret,
                disabled,
                ..
            } => {
                assert!(!secret, "default_model 不是敏感字段");
                assert!(!disabled, "default_model 应可编辑");
                assert!(
                    placeholder.contains("探测"),
                    "placeholder 应指向探测按钮：{placeholder}"
                );
            }
            FormField::Select { .. } => {
                panic!("preset spec 的 default_model 应是 Text（手填），不是 Select")
            }
        }
    }

    /// preset 表单 schema 只暴露 name / preset / api_key / default_model /
    /// protocol 五个字段 —— base_url 与 models 跟随代码，已迁出到独立的
    /// 折叠面板（见 `render_preset_details`）。
    #[test]
    fn preset_spec_has_only_user_owned_fields() {
        let spec = spec_preset();
        let names: Vec<&str> = spec.fields.iter().map(|f| f.name()).collect();
        assert_eq!(
            names,
            vec!["name", "preset", "api_key", "default_model", "protocol"],
            "preset spec 字段顺序与 spec 锁定"
        );

        // Text 字段（name / api_key / default_model）必须可编辑。
        for name in ["name", "api_key", "default_model"] {
            let f = spec
                .fields
                .iter()
                .find(|f| f.name() == name)
                .unwrap_or_else(|| panic!("missing field {name}"));
            match f {
                FormField::Text { disabled, .. } => {
                    assert!(!disabled, "{name} 字段不应 disabled")
                }
                _ => panic!("{name} 应是 Text 字段"),
            }
        }

        // 不应有任何 base_url_* 或 models 字段（跟随代码，不进表单）。
        for banned in [
            "base_url_anthropic",
            "base_url_openai_chat",
            "base_url_openai_responses",
            "models",
        ] {
            assert!(
                spec.fields.iter().all(|f| f.name() != banned),
                "preset spec 不应含 {banned}"
            );
        }
    }

    /// openspec/specs/provider-management/spec.md：preset 表单的 protocol 字段是 Select，含
    /// auto/anthropic/openai 三档，default = "auto"。
    #[test]
    fn preset_spec_has_protocol_select_with_three_options() {
        let spec = spec_preset();
        let field = spec
            .fields
            .iter()
            .find(|f| f.name() == "protocol")
            .expect("preset spec must include protocol");
        match field {
            FormField::Select {
                options,
                required,
                on_change,
                ..
            } => {
                assert!(!required, "protocol 在 preset 表单里选填");
                let values: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
                assert_eq!(values, vec!["auto", "anthropic", "openai"]);
                assert!(
                    on_change.is_none(),
                    "protocol 在表单内是静默字段（on_change=None）"
                );
            }
            _ => panic!("preset spec 的 protocol 应是 Select"),
        }
    }

    /// openspec/specs/provider-management/spec.md：custom 表单也带 protocol Select，与 preset
    /// 字段对齐。
    #[test]
    fn custom_spec_has_protocol_select_with_three_options() {
        let spec = spec_custom();
        let field = spec
            .fields
            .iter()
            .find(|f| f.name() == "protocol")
            .expect("custom spec must include protocol");
        match field {
            FormField::Select { options, .. } => {
                let values: Vec<&str> = options.iter().map(|o| o.value.as_str()).collect();
                assert_eq!(values, vec!["auto", "anthropic", "openai"]);
            }
            _ => panic!("custom spec 的 protocol 应是 Select"),
        }
    }

    /// anthropic preset 只有一个 anthropic 端点，折叠面板里应展示该 URL、
    /// 「跟随代码」标注与「—」占位。
    #[test]
    fn render_preset_details_for_anthropic() {
        let elements = render_preset_details("anthropic");
        assert_eq!(elements.len(), 1, "应返回一个 CollapsiblePanel");
        let CardElement::CollapsiblePanel(panel) = &elements[0] else {
            panic!("expected CollapsiblePanel, got {:?}", elements[0]);
        };
        // 面板标题带「跟随代码」标注。
        assert_eq!(panel.header.title.content, "📋 预设详情（跟随代码）");
        // 内容四行：三个 URL + 一个 env 名。Anthropic 没有 OpenAI 端点。
        let rendered = panel
            .elements
            .iter()
            .map(|e| match e {
                CardElement::Markdown { content } => content.clone(),
                other => panic!("unexpected child element: {other:?}"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("https://api.anthropic.com"),
            "anthropic URL 应出现在面板内容：{rendered}"
        );
        assert!(
            rendered.contains("ANTHROPIC_API_KEY"),
            "默认 env 名应出现在面板内容：{rendered}"
        );
        // OpenAI 端点缺失时显示「—」占位（避免误以为有端点）。
        assert!(
            rendered.contains('—'),
            "openai 端点缺失应显示占位：{rendered}"
        );
    }

    /// 找不到 preset 时返回空 vec —— 调用方不叠加任何元素（不报错）。
    #[test]
    fn render_preset_details_for_unknown_preset_returns_empty() {
        let elements = render_preset_details("does-not-exist");
        assert!(
            elements.is_empty(),
            "未知 preset 应返回空 vec，实际拿到 {} 个元素",
            elements.len()
        );
    }

    /// deepseek preset 有 anthropic 与 chat 两个端点（无 responses 端点），
    /// 面板里 anthropic/chat URL 应出现、responses 位显示「—」。
    #[test]
    fn render_preset_details_for_deepseek_shows_both_urls() {
        let elements = render_preset_details("deepseek");
        assert_eq!(elements.len(), 1);
        let CardElement::CollapsiblePanel(panel) = &elements[0] else {
            panic!("expected CollapsiblePanel");
        };
        let rendered = panel
            .elements
            .iter()
            .map(|e| match e {
                CardElement::Markdown { content } => content.clone(),
                _ => panic!("unexpected child element"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("https://api.deepseek.com/anthropic"),
            "anthropic URL 应出现在面板：{rendered}"
        );
        assert!(
            rendered.contains("https://api.deepseek.com"),
            "openai chat URL 应出现在面板：{rendered}"
        );
        assert!(
            rendered.contains("DEEPSEEK_API_KEY"),
            "默认 env 应出现在面板：{rendered}"
        );
        // deepseek 无公开 responses 端点 → 占位。
        assert!(
            rendered.contains("**Base URL(OpenAI Responses)**\n`—`"),
            "responses 端点缺失应显示占位：{rendered}"
        );
    }

    /// openai preset 双 OpenAI 槽位（chat + responses）同端点。
    #[test]
    fn render_preset_details_for_openai_shows_both_slots() {
        let elements = render_preset_details("openai");
        assert_eq!(elements.len(), 1);
        let CardElement::CollapsiblePanel(panel) = &elements[0] else {
            panic!("expected CollapsiblePanel");
        };
        let rendered = panel
            .elements
            .iter()
            .map(|e| match e {
                CardElement::Markdown { content } => content.clone(),
                _ => panic!("unexpected child element"),
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            rendered.matches("https://api.openai.com/v1").count(),
            2,
            "chat 与 responses 槽位都指向同一端点：{rendered}"
        );
    }

    // ---- provider 数据的唯一权威是状态库（retire-legacy-state-json 3.1）----

    /// 全新装机（库里没有任何 provider）→ `build_form` 正常返回 `Some`，
    /// 种子来自 config.toml 的 `[provider.*]`。锁定「引导路径不受退休影响」。
    #[tokio::test(flavor = "multi_thread")]
    async fn build_form_with_empty_store_uses_config_seed() {
        let _engine = sebas_dispatch::test_engine::install_fresh();
        let forms = build_form(&minimal_config_with_seed()).expect("empty store is OK");
        let items = forms.preset.store.list().await;
        assert_eq!(items.len(), 1, "config.toml 的 deepseek 应作为种子出现");
        assert_eq!(
            items[0].get("name").and_then(Value::as_str),
            Some("deepseek")
        );
    }

    /// 库里已有 provider → 它与 config.toml 种子合并（库是权威，种子是引导）。
    /// 覆盖旧「合法 overlay 文件」用例的语义，只是来源换成状态库。
    #[tokio::test(flavor = "multi_thread")]
    async fn build_form_merges_store_values_over_the_config_seed() {
        let _engine = sebas_dispatch::test_engine::install_fresh();
        // 先经表单写一条：库里就有 openai。
        {
            let forms = build_form(&minimal_config_with_seed()).expect("seed bootstrap");
            let mut new_item = Map::new();
            new_item.insert("name".into(), Value::String("openai".into()));
            forms.preset.store.insert(new_item).await.unwrap();
        }
        // 重建表单：config.toml seed (deepseek) + 库 (openai) = 2 条。
        let forms = build_form(&minimal_config_with_seed()).expect("store plus seed");
        let items = forms.preset.store.list().await;
        assert_eq!(items.len(), 2, "库里 1 条 + 种子 1 条：{items:?}");
    }

    /// 盘上残留的 `providers.json`（哪怕内容完全合法、哪怕它「看起来像
    /// providers」）**不参与** provider 数据：build_form 既不读它、也不搬它，
    /// 逐字节未变。
    #[tokio::test(flavor = "multi_thread")]
    async fn legacy_providers_json_is_neither_read_nor_moved() {
        let _engine = sebas_dispatch::test_engine::install_fresh();
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        let body = r#"{ "providers": { "openai": { "name": "openai" } }, "deleted": [] }"#;
        std::fs::write(&overlay, body).unwrap();

        let forms = build_form(&minimal_config_with_seed()).expect("store-only load");
        let items = forms.preset.store.list().await;
        assert_eq!(
            items.len(),
            1,
            "遗留文件里的 openai 不得出现在视图里：{items:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&overlay).unwrap(),
            body,
            "遗留 providers.json 必须逐字节未变（不读、不改、不搬、不备份）"
        );
        // 也没有 `.broken-` 备份残留（自我修复机制已随文件一起退休）。
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".broken-"))
            .collect();
        assert!(backups.is_empty(), "不应再产生备份文件");
    }

    /// 状态库不可用 → provider 域按空表呈现（种子仍在），且**绝不**去读任何
    /// 遗留文件。写操作以 Err 拒绝而不是静默成功。
    #[tokio::test(flavor = "multi_thread")]
    async fn unavailable_store_presents_empty_seed_and_rejects_writes() {
        let _engine = sebas_dispatch::test_engine::install_none();
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        std::fs::write(
            &overlay,
            r#"{ "providers": { "ghost": { "name": "ghost" } }, "deleted": [] }"#,
        )
        .unwrap();

        let forms = build_form(&minimal_config_with_seed()).expect("still constructible");
        let items = forms.preset.store.list().await;
        assert_eq!(items.len(), 1, "只剩 config.toml 种子：{items:?}");
        assert_eq!(
            items[0].get("name").and_then(Value::as_str),
            Some("deepseek")
        );

        let mut new_item = Map::new();
        new_item.insert("name".into(), Value::String("nope".into()));
        assert!(
            forms.preset.store.insert(new_item).await.is_err(),
            "库不可用时写必须以 Err 拒绝"
        );
        assert!(
            overlay.exists(),
            "库不可用不得让实现转头去写/动遗留文件：{}",
            overlay.display()
        );
    }
}
