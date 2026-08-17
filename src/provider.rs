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
//!
//! `default_model`：bot 侧的「spawn 时落到 agent 的默认 model」选择，仅
//! 写入 overlay（不落 gateway `ProviderConfig`），由后续
//! `AgentDriver::resolve_args()`（bead sebas-63f.8）传给 agent。preset 表
//! 单用 `Select` 给常用 model 一个下拉（union 全 preset 的 model 列表）；
//! custom 表单用文本框让用户手填（custom provider 不在静态 preset 表里）。

use feishu::cards::{
    CardElement, CardText, CollapsiblePanel, CollapsiblePanelHeader, StandardIcon,
};
use feishu::forms::{FormField, FormSpec, SelectOption};
use gateway::config::GatewayConfig;
use router::crud::{CrudForm, FileStore, Item};
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// 预设表单：用户从代码里写好的 provider 里选一个，只填名称 + 密钥。
/// base_url 在 normalizer 提交时按 preset 推断并以「只读」回填展示。
pub const FORM_PRESET: &str = "provider-preset";
/// 自定义表单：用户手填所有参数（与原单表单形态一致）。
pub const FORM_CUSTOM: &str = "provider-custom";
pub const ID_FIELD: &str = "name";

/// 预设模式表单：name + preset + api_key + default_model（4 个可见字段）。
///
/// base_url_* 字段原本是「disabled 文本框」放在表单里展示 preset 决定的端点
/// —— 但飞书 `select_static` 在 form 容器内挂 behaviors 不稳定，preset 切换
/// 没法实时回填那俩字段，且 disabled input 让表单看起来很拥挤。改成把这些
/// preset 决定的只读详情放在表单独立的 `CardElement::CollapsiblePanel`（由
/// 调用方在卡片 body 上叠加 `render_preset_details()`），表单只剩真正要
/// 用户填的字段。
///
/// 提交时 `apply_preset_defaults` 按 preset 推断并把 base_url_* 写回存储，
/// 所以存储侧的 shape 不变；编辑已有 provider 时 `item_to_initial()` 只
/// 从 spec 字段读 initial，未列入 spec 的 base_url_* 也不会预填（反正
/// 用户看不到这俩字段）。
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
                options: gateway::config::presets()
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
            FormField::Text {
                name: "models".into(),
                label: "模型列表".into(),
                required: false,
                placeholder: "用逗号分隔，从强到弱：如 deepseek-v4-pro[1m], deepseek-v4-flash".into(),
                secret: false,
                disabled: false,
            },
            // default_model（sebas-63f.4）：表单是静态的，select options
            // 用全 preset 的 models 并集。`models` 字段是 provider 提供的
            // 完整列表（HEAD 引入），`default_model` 是单选默认 model。
            // 两者并存 —— 前者是 catalog，后者是偏好。
            FormField::Select {
                name: "default_model".into(),
                label: "默认 model".into(),
                required: false,
                options: gateway::config::presets()
                    .iter()
                    .flat_map(|p| p.models.iter().copied())
                    .map(|m| SelectOption {
                        value: m.to_string(),
                        label: m.to_string(),
                    })
                    .collect(),
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
    let Some(p) = gateway::config::presets()
        .iter()
        .find(|p| p.name == preset_name)
    else {
        return Vec::new();
    };

    let url_anthropic = p.base_url_anthropic.unwrap_or("—");
    let url_openai = p.base_url_openai.unwrap_or("—");

    let lines = [
        format!("**Base URL(Anthropic)**\n`{url_anthropic}`"),
        format!("**Base URL(OpenAI)**\n`{url_openai}`"),
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
                content: "📋 预设详情".into(),
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

/// 自定义模式表单：所有字段都让用户填（与 gateway `ProviderConfig` 字段对齐）。
/// base_url_anthropic / base_url_openai 各自独立，可只填一个（只支持对应协议）。
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
                name: "base_url_openai".into(),
                label: "Base URL(OpenAI)".into(),
                required: false,
                placeholder: "留空表示不提供 OpenAI 协议".into(),
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
                placeholder: "用逗号分隔，从强到弱：如 deepseek-v4-pro[1m], deepseek-v4-flash".into(),
                secret: false,
                disabled: false,
            },
            // default_model（sebas-63f.4）：custom provider 不在静态 preset
            // 表里，model 名无法预填；让用户手填。
            FormField::Text {
                name: "default_model".into(),
                label: "默认 model".into(),
                required: false,
                placeholder: "如 deepseek-chat 或 gpt-4o".into(),
                secret: false,
                disabled: false,
            },
        ],
    )
}

/// 兼容老引用（测试 / 旧调用方）。
pub fn spec() -> FormSpec {
    spec_custom()
}

/// 把 gateway 配置里的 provider 转成 CRUD item（种子）。
///
/// `default_model` 不在 gateway `ProviderConfig` 上（bead sebas-63f.4）：
/// 用户通过 bot 表单写入的值仅落在 overlay 文件，不向 gateway 同步。
/// 后续 `AgentDriver::resolve_args()`（bead sebas-63f.8）会从 overlay 读到
/// 这个值传给 agent。这里不写入 `default_model`，让表单编辑时初始为空；
/// overlay 里已有 `default_model` 的项会在 `item_to_initial` 里被预填。
// TODO: gateway config sync is out of scope for this bead.
pub fn item_from_provider(name: &str, p: &gateway::config::ProviderConfig) -> Item {
    let mut m = Map::new();
    m.insert("name".into(), Value::String(name.into()));
    if let Some(u) = &p.base_url_anthropic {
        m.insert("base_url_anthropic".into(), Value::String(u.clone()));
    }
    if let Some(u) = &p.base_url_openai {
        m.insert("base_url_openai".into(), Value::String(u.clone()));
    }
    if let Some(key) = &p.api_key {
        m.insert("api_key".into(), Value::String(key.clone()));
    }
    if let Some(env) = &p.api_key_env {
        m.insert("api_key_env".into(), Value::String(env.clone()));
    }
    // 回写 models（从强到弱），表单用逗号分隔字符串，保证 /provider 保存时
    // 不被 overlay 抹掉，且表单 Text 字段能展示。
    if !p.models.is_empty() {
        m.insert("models".into(), Value::String(p.models.join(",")));
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

/// `/provider` 命令的两张表单（共享同一个 overlay 存储）。
/// 定义在 router 里；sebas root crate 只是装配。
pub use router::crud::ProviderForms;

/// 构造两套 provider CRUD 表单：种子来自 config.toml 的顶层 `[provider.*]`，
/// 变更持久化到 overlay 文件。加载失败时返回 None（`/provider` 退化为帮助）。
pub fn build_form(raw_config: &str) -> Option<Arc<ProviderForms>> {
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
        Ok(store) => Some(Arc::new(ProviderForms {
            preset: Arc::new(
                CrudForm::new(spec_preset(), ID_FIELD, store.clone())
                    .with_normalizer(Arc::new(apply_preset_defaults)),
            ),
            custom: Arc::new(
                CrudForm::new(spec_custom(), ID_FIELD, store)
                    .with_normalizer(Arc::new(noop_normalizer)),
            ),
        })),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "provider overlay 加载失败；/provider 不可用");
            None
        }
    }
}

/// 自定义模式表单的 normalizer：什么都不做（用户已填全所有字段）。
/// 保留签名一致让两套表单可以共享 `with_normalizer` 调用点。
fn noop_normalizer(_item: &mut Item) {}

/// 提交规范化：选中 preset 时补全默认值（与 config.toml preset 解析一致）。
/// - `base_url_anthropic` / `base_url_openai` 各自按 preset 填缺项；
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

    let anth_empty = item
        .get("base_url_anthropic")
        .and_then(Value::as_str)
        .is_none_or(|s| s.is_empty());
    if anth_empty && let Some(u) = p.base_url_anthropic {
        item.insert("base_url_anthropic".into(), Value::String(u.to_string()));
    }
    let oai_empty = item
        .get("base_url_openai")
        .and_then(Value::as_str)
        .is_none_or(|s| s.is_empty());
    if oai_empty && let Some(u) = p.base_url_openai {
        item.insert("base_url_openai".into(), Value::String(u.to_string()));
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
    fn deepseek_preset_fills_both_urls_and_default_env() {
        let mut item = item_with(&[("name", "deepseek"), ("preset", "deepseek")]);
        apply_preset_defaults(&mut item);
        assert_eq!(
            item.get("base_url_anthropic").and_then(Value::as_str),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(
            item.get("base_url_openai").and_then(Value::as_str),
            Some("https://api.deepseek.com")
        );
        assert_eq!(
            item.get("api_key_env").and_then(Value::as_str),
            Some("DEEPSEEK_API_KEY")
        );
    }

    #[test]
    fn preset_with_only_api_key_fills_both_urls() {
        // 用户视角：选 deepseek 预设 + 粘密钥，两个 URL 全部补全。
        let mut item = item_with(&[
            ("name", "deepseek"),
            ("preset", "deepseek"),
            ("api_key", "sk-ds"),
        ]);
        apply_preset_defaults(&mut item);
        assert_eq!(
            item.get("base_url_anthropic").and_then(Value::as_str),
            Some("https://api.deepseek.com/anthropic")
        );
        assert_eq!(
            item.get("base_url_openai").and_then(Value::as_str),
            Some("https://api.deepseek.com")
        );
        assert!(
            item.get("api_key_env").is_none(),
            "有 api_key 时不注入默认 env"
        );
    }

    #[test]
    fn single_protocol_preset_fills_only_its_url() {
        let mut item = item_with(&[
            ("name", "anthropic"),
            ("preset", "anthropic"),
            ("api_key", "sk-anthropic"),
        ]);
        apply_preset_defaults(&mut item);
        assert_eq!(
            item.get("base_url_anthropic").and_then(Value::as_str),
            Some("https://api.anthropic.com")
        );
        assert!(
            item.get("base_url_openai").is_none(),
            "anthropic preset 不提供 openai 端点"
        );
        assert!(item.get("api_key_env").is_none());
    }

    #[test]
    fn explicit_base_urls_and_key_override_preset() {
        let mut item = item_with(&[
            ("name", "deepseek"),
            ("preset", "deepseek"),
            ("base_url_anthropic", "http://localhost:9999/anth"),
            ("base_url_openai", "http://localhost:9999/oai"),
            ("api_key", "sk-test"),
        ]);
        apply_preset_defaults(&mut item);
        // 显式字段不被 preset 覆盖。
        assert_eq!(
            item.get("base_url_anthropic").and_then(Value::as_str),
            Some("http://localhost:9999/anth")
        );
        assert_eq!(
            item.get("base_url_openai").and_then(Value::as_str),
            Some("http://localhost:9999/oai")
        );
        // 显式 api_key 时不注入默认 env（否则 resolve_api_keys 会读错 env）。
        assert!(item.get("api_key_env").is_none());
    }

    #[test]
    fn no_preset_leaves_item_untouched() {
        let mut item = item_with(&[("name", "my-custom")]);
        apply_preset_defaults(&mut item);
        assert!(item.get("base_url_anthropic").is_none());
        assert!(item.get("base_url_openai").is_none());
        assert!(item.get("api_key_env").is_none());
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

    /// preset 表单 schema 把 default_model 暴露成 Select，选项是全 preset
    /// models 的并集。
    #[test]
    fn preset_spec_default_model_is_select_with_union_options() {
        let spec = spec_preset();
        let field = spec
            .fields
            .iter()
            .find(|f| f.name() == "default_model")
            .expect("preset spec must include default_model");
        match field {
            FormField::Select { options, .. } => {
                // 全 preset 的 models 并集 —— union 至少含 deepseek-chat。
                let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
                assert!(
                    labels.contains(&"deepseek-chat"),
                    "preset 表单应包含 deepseek-chat：{labels:?}"
                );
                // 至少与某 preset 单个 models 列表等长（说明 union 没漏）。
                let preset = gateway::config::presets();
                let max_single = preset.iter().map(|p| p.models.len()).max().unwrap_or(0);
                assert!(
                    options.len() >= max_single,
                    "union options 不应比单个 preset 的 models 列表短：{} < {max_single}",
                    options.len()
                );
            }
            FormField::Text { .. } => {
                panic!("preset spec 的 default_model 应是 Select，不是 Text")
            }
        }
    }

    /// preset 表单 schema 只暴露 name / preset / api_key / default_model
    /// 四个字段 —— base_url_anthropic / base_url_openai 已迁出到独立的
    /// 折叠面板（见 `render_preset_details`）。
    #[test]
    fn preset_spec_has_only_three_editable_text_fields() {
        let spec = spec_preset();
        let names: Vec<&str> = spec.fields.iter().map(|f| f.name()).collect();
        // 顺序：name / preset / api_key / models（HEAD 引入的 catalog）/ default_model（63f）
        assert_eq!(
            names,
            vec!["name", "preset", "api_key", "models", "default_model"],
            "preset spec 不应再含 base_url_anthropic / base_url_openai"
        );

        // Text 字段（name / api_key / models）必须是可编辑
        // （非 disabled），用户能正常输入。
        for name in ["name", "api_key", "models"] {
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

        // 不应再有 base_url_* 字段。
        assert!(
            spec.fields.iter().all(|f| f.name() != "base_url_anthropic"),
            "preset spec 不应再含 base_url_anthropic"
        );
        assert!(
            spec.fields.iter().all(|f| f.name() != "base_url_openai"),
            "preset spec 不应再含 base_url_openai"
        );
    }

    /// anthropic preset 只有一个 anthropic 端点，折叠面板里应展示该 URL 与
    /// 「—」占位（OpenAI 端点不存在）。
    #[test]
    fn render_preset_details_for_anthropic() {
        let elements = render_preset_details("anthropic");
        assert_eq!(elements.len(), 1, "应返回一个 CollapsiblePanel");
        let CardElement::CollapsiblePanel(panel) = &elements[0] else {
            panic!("expected CollapsiblePanel, got {:?}", elements[0]);
        };
        // 面板标题是「📋 预设详情」。
        assert_eq!(panel.header.title.content, "📋 预设详情");
        // 内容三行：两个 URL + 一个 env 名。Anthropic 没有 OpenAI 端点。
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
        assert!(rendered.contains('—'), "openai 端点缺失应显示占位：{rendered}");
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

    /// deepseek preset 同时有 anthropic 和 openai 两个端点，面板里两个 URL
    /// 都应出现。
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
            "openai URL 应出现在面板：{rendered}"
        );
        assert!(
            rendered.contains("DEEPSEEK_API_KEY"),
            "默认 env 应出现在面板：{rendered}"
        );
    }
}
