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
//! `ClaudeCodeDriver::resolve_args()`（bead sebas-63f.8）传给 agent。表单里是
//! 手填文本框（preset / custom 一致）——model 列表的权威来源是 provider
//! 官方 `/models` 接口（详情面板的「🔍 探测 model 列表」按钮），静态
//! preset 表里的型号很快就会过时；探测不可用时手填兜底。

use feishu::cards::{
    CardElement, CardText, CollapsiblePanel, CollapsiblePanelHeader, StandardIcon,
};
use feishu::forms::{FormField, FormSpec, SelectOption};
use gateway::config::GatewayConfig;
use router::crud::{CrudForm, FileStore, Item};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
                placeholder: "用逗号分隔，从强到弱；或点下方「🔍 获取模型列表」从官方 API 拉取"
                    .into(),
                secret: false,
                disabled: false,
            },
            // default_model（sebas-63f.4，2026-08-17 改回手填）：静态
            // preset 表的型号列表会过时，单选下拉反而误导；权威来源是官方
            // `/models` 接口（详情面板的「🔍 探测 model 列表」按钮会把
            // 列表写回 `models` 目录并可一键设为默认），探测不可用时手填。
            // `models` 是 provider 提供的完整列表（catalog），
            // `default_model` 是偏好——两者并存。
            FormField::Text {
                name: "default_model".into(),
                label: "默认 model".into(),
                required: false,
                placeholder: "手填 model id；或保存后用「🔍 探测 model 列表」从官方 API 选".into(),
                secret: false,
                disabled: false,
            },
            // 协议选择（spec 2026-08-17 §2.4）：把 Direct 模式的协议优先级
            // 从「隐式 anthropic > openai」挪到 UI。"auto" = 旧行为（默认，
            // 向后兼容）；显式 anthropic/openai 强制走对应协议端点，对应 URL
            // 缺失时由 spawn_env 回退 Off + warn。
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
                placeholder: "用逗号分隔，从强到弱；或点下方「🔍 获取模型列表」从官方 API 拉取"
                    .into(),
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
            // 协议选择（spec 2026-08-17 §2.4）：custom provider 也走同一套
            // 三档（auto / anthropic / openai）；preset 表单已加，custom 保持
            // 字段对齐。
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

/// 把 gateway 配置里的 provider 转成 CRUD item（种子）。
///
/// `default_model` 不在 gateway `ProviderConfig` 上（bead sebas-63f.4）：
/// 用户通过 bot 表单写入的值仅落在 overlay 文件，不向 gateway 同步。
/// 后续 `ClaudeCodeDriver::resolve_args()`（bead sebas-63f.8）会从 overlay 读到
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
/// 变更持久化到 `state.json`（详见 spec 2026-08-17 §2.6 与
/// `router::state_store`）。
///
/// **Self-heal（spec 2026-08-17 §2.3）**：legacy overlay 文件（`providers.json`，
/// `state.json` 不存在时一次性迁移源）破损时不再让 `/provider` 死掉 —
/// 先把损坏的文件备份到 `<path>.broken-<ts>-<pid>.json`，再让
/// `FileStore::load`（委托给 `state_store::load`）从 `state.json` / 迁移
/// 路径取数；这样 `/provider` 仍返回 `Some(forms)`，seed 强制为空（让用户
/// 从 `/provider` 重新建）。备份失败（如只读文件系统）时才回退到 `None`
/// （`/provider` 在 UI 上显示「不可用」）。
///
/// **顺序**：先做 broken-overlay 备份再做 state_store 加载 —— 否则
/// state_store::load 看到 broken overlay 只 warn + 返回 default，
/// 损坏文件就留在原位无人清理（spec §2.3 不允许）。
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
    // Self-heal 探测：legacy overlay 存在但 JSON 解析失败 → 备份走人。
    // 必须放在 FileStore::load（→ state_store::load）之前，否则 state_store
    // 只 warn + 跳过，broken 文件留在原位。
    if let Err(parse_err) = validate_legacy_overlay(&path) {
        tracing::warn!(
            path = %path.display(),
            error = %parse_err,
            "legacy overlay 解析失败，按 spec 2026-08-17 §2.3 备份后从空 seed 恢复"
        );
        if let Err(backup_err) = backup_broken_overlay(&path) {
            tracing::warn!(
                path = %path.display(),
                parse_error = %parse_err,
                backup_error = %backup_err,
                "legacy overlay 备份失败，/provider 不可用"
            );
            return None;
        }
        // 备份成功 → 用空 seed 重 load，让用户从 /provider 重新建。
        match FileStore::load(path, ID_FIELD, Vec::new()) {
            Ok(store) => return Some(Arc::new(make_forms(store))),
            Err(e) => {
                tracing::warn!(error = %e, "备份成功后重新 load 仍失败，/provider 不可用");
                return None;
            }
        }
    }
    match FileStore::load(path, ID_FIELD, seed) {
        Ok(store) => Some(Arc::new(make_forms(store))),
        Err(e) => {
            tracing::warn!(error = %e, "provider 存储加载失败；/provider 不可用");
            None
        }
    }
}

/// 检查 legacy overlay 文件是否能解析为 v2 overlay 形状：
/// - 不存在 → `Ok`（全新装机，无需备份）；
/// - 存在 + 解析成功 → `Ok`（state_store 会自动迁移）；
/// - 存在 + 解析失败 → `Err`（broken，调用方走 backup 路径）。
fn validate_legacy_overlay(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let raw =
        std::fs::read_to_string(path).map_err(|e| format!("读取 legacy overlay 失败: {e}"))?;
    #[derive(serde::Deserialize)]
    struct OverlayShape {
        #[serde(default)]
        #[allow(dead_code)]
        providers: serde_json::Map<String, serde_json::Value>,
        #[serde(default)]
        #[allow(dead_code)]
        deleted: Vec<String>,
    }
    serde_json::from_str::<OverlayShape>(&raw)
        .map(|_| ())
        .map_err(|e| format!("解析 legacy overlay 失败: {e}"))
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

/// 把损坏的 overlay 文件移到 `<path>.broken-<ts>-<pid>.json`，返回新路径。
///
/// - 优先 `std::fs::rename`（同 fs 上原子，且不会复制可能很大的文件）。
/// - 跨设备 rename 失败 → `copy + remove` 兜底。
/// - timestamp 用 millis + PID 避免同秒内重复启动导致重名冲突。
/// - 路径已含扩展名 `.json`：直接拼 `<path>.broken-<ts>-<pid>.json`，
///   不在文件名尾部再加一次 `.json`（避免双扩展名）。
fn backup_broken_overlay(path: &Path) -> std::io::Result<PathBuf> {
    let ts_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut backup = path.to_path_buf().into_os_string();
    backup.push(format!(".broken-{ts_ms}-{pid}.json"));
    let backup_path = PathBuf::from(backup);

    match std::fs::rename(path, &backup_path) {
        Ok(()) => Ok(backup_path),
        Err(rename_err) => {
            // rename 在跨设备时会 EXDEV — 试 copy+remove 兜底。
            std::fs::copy(path, &backup_path)?;
            std::fs::remove_file(path)?;
            tracing::debug!(
                from = %path.display(),
                to = %backup_path.display(),
                error = %rename_err,
                "rename 失败（跨设备？），用 copy+remove 兜底完成备份"
            );
            Ok(backup_path)
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
    use router::CrudStore;
    use std::sync::Mutex;

    // 串行化 `SEBAS_GATEWAY_PROVIDER_OVERLAY` env 访问，与
    // `spawn_env::tests` 同惯例（全局 env 跨测试并发跑会撞）。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn item_with(fields: &[(&str, &str)]) -> Item {
        let mut m = Map::new();
        for (k, v) in fields {
            m.insert((*k).into(), Value::String((*v).into()));
        }
        m
    }

    /// 写一个最小 config.toml（含 [provider.deepseek] 种子），让
    /// `build_form` 走真实 GatewayConfig::parse 路径。
    fn minimal_config_with_seed() -> String {
        r#"
[gateway]
listen = "127.0.0.1:0"

[provider.deepseek]
"#
        .to_string()
    }

    /// 把 overlay env 指向指定路径，同时把 state.json 重定向到同目录的
    /// `state.json`（spec 2026-08-17 §2.6：FileStore 现在走 unified
    /// `state.json`，env 不隔离会读到真实 `~/.sebas/state.json` 的污染数据）。
    /// lock 由调用方持。
    fn point_overlay_env(path: &Path) {
        let state_path = path.with_file_name("state.json");
        // SAFETY: ENV_LOCK held by caller.
        unsafe {
            std::env::set_var("SEBAS_GATEWAY_PROVIDER_OVERLAY", path.to_str().unwrap());
            std::env::set_var("SEBAS_STATE_FILE", state_path.to_str().unwrap());
        }
    }

    fn clear_overlay_env() {
        // SAFETY: ENV_LOCK held by caller.
        unsafe {
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
            std::env::remove_var("SEBAS_STATE_FILE");
        }
    }

    /// 枚举指定目录下所有 `.broken-` 后缀的备份文件路径。
    fn broken_backups(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        if name.contains(".broken-") {
                            Some(e.path())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
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

    /// preset 表单 schema 只暴露 name / preset / api_key / default_model
    /// 四个字段 —— base_url_anthropic / base_url_openai 已迁出到独立的
    /// 折叠面板（见 `render_preset_details`）。
    #[test]
    fn preset_spec_has_only_three_editable_text_fields() {
        let spec = spec_preset();
        let names: Vec<&str> = spec.fields.iter().map(|f| f.name()).collect();
        // 顺序：name / preset / api_key / models（HEAD 引入的 catalog）/
        // default_model（63f）/ protocol（spec 2026-08-17 §2.4，详情面板
        // 的协议 radio 也用这个 name）。
        assert_eq!(
            names,
            vec![
                "name",
                "preset",
                "api_key",
                "models",
                "default_model",
                "protocol"
            ],
            "preset spec 字段顺序与 spec 锁定"
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

    /// spec 2026-08-17 §2.4：preset 表单的 protocol 字段是 Select，含
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

    /// spec 2026-08-17 §2.4：custom 表单也带 protocol Select，与 preset
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

    // ---- spec 2026-08-17 §2.3 self-heal tests ----

    /// broken JSON 在 overlay 路径 → `build_form` 返回 `Some(forms)`，
    /// 损坏文件被备份到 `<path>.broken-<ts>-<pid>.json`，原文件不再存在。
    #[tokio::test]
    async fn build_form_self_heals_on_broken_overlay() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        std::fs::write(&overlay, "{not valid json at all").unwrap();
        point_overlay_env(&overlay);

        let forms = build_form(&minimal_config_with_seed());
        // 关键：必须返回 Some，不是 None。
        let _forms = forms.expect("broken overlay must self-heal, not return None");

        // 损坏文件已搬走，原位置不再有 providers.json。
        assert!(
            !overlay.exists(),
            "损坏的 overlay 备份后不应留在原位：{}",
            overlay.display()
        );

        // 备份存在，且名字带 `.broken-` 前缀。
        let backups = broken_backups(dir.path());
        assert_eq!(
            backups.len(),
            1,
            "应恰好一个 .broken- 备份文件，实际：{:?}",
            backups
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        );
        let backup_name = backups[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            backup_name.starts_with("providers.json.broken-"),
            "备份名前缀应为 `providers.json.broken-`，实际：{backup_name}"
        );
        assert!(
            backup_name.ends_with(".json"),
            "备份名应以 .json 结尾：{backup_name}"
        );
        // 备份内容应与原损坏内容一致（rename 搬过去，不是删了）。
        let backed_up = std::fs::read_to_string(&backups[0]).unwrap();
        assert_eq!(backed_up, "{not valid json at all");

        clear_overlay_env();
    }

    /// 备份后用空 seed 重新 load — `/provider` 应渲染空列表，但 form 实例
    /// 仍然可用（不为 None，用户能 add）。直接验证 `forms.preset.store.list()`
    /// 是空 vec、且表单 form_name 没坏。
    #[tokio::test]
    async fn build_form_after_self_heal_uses_empty_seed() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        // 损坏的文件即便看起来像是有 providers 也被忽略 —— 走的是空 seed。
        std::fs::write(
            &overlay,
            r#"{ "providers": { "deepseek": { "name": "deepseek" } } }"#,
        )
        .unwrap();
        // 故意把 JSON 写残。
        std::fs::write(&overlay, "broken{").unwrap();
        point_overlay_env(&overlay);

        let forms = build_form(&minimal_config_with_seed()).expect("self-heal");
        let items = forms.preset.store.list().await;
        assert!(
            items.is_empty(),
            "self-heal 后种子应为空（spec：seed=empty），实际：{:?}",
            items
        );
        // 表单 form_name 保留（用户能 add）。
        assert_eq!(forms.preset.spec.form_name, FORM_PRESET);
        assert_eq!(forms.custom.spec.form_name, FORM_CUSTOM);

        clear_overlay_env();
    }

    /// overlay 文件不存在（首次启动 / 全新装机）→ `build_form` 正常返回
    /// `Some(forms)`，seed 来自 config.toml（不走 self-heal 分支）。这条
    /// 测试锁定「正常路径不受影响」。
    #[tokio::test]
    async fn build_form_missing_overlay_still_works() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        assert!(!overlay.exists());
        point_overlay_env(&overlay);

        let forms = build_form(&minimal_config_with_seed()).expect("missing file is OK");
        let items = forms.preset.store.list().await;
        // config.toml 里 deepseek seed 应该出现在视图中。
        assert_eq!(items.len(), 1, "config.toml 的 deepseek 应作为种子出现");
        assert_eq!(
            items[0].get("name").and_then(Value::as_str),
            Some("deepseek")
        );

        // 没有 backup 文件生成。
        assert!(
            broken_backups(dir.path()).is_empty(),
            "缺失 overlay 不应触发备份"
        );
        clear_overlay_env();
    }

    /// 完整且合法的 overlay 文件 → `build_form` 不触发 self-heal，备份
    /// 目录为空（守住 §2.3 「只在错误路径备份」的承诺）。
    ///
    /// change gateway-admin-api-and-model-aliases：providers.json 拆回
    /// 独立文件成为单一真源（卡片 + gateway admin API 双写者共用），
    /// **不再**迁移到 state.json 后删除。本测试断言新语义：providers.json
    /// 保留在原位、内容不变。
    #[tokio::test]
    async fn build_form_valid_overlay_does_not_backup() {
        let _g = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        std::fs::write(
            &overlay,
            r#"{ "providers": { "openai": { "name": "openai" } }, "deleted": [] }"#,
        )
        .unwrap();
        point_overlay_env(&overlay);

        let forms = build_form(&minimal_config_with_seed()).expect("valid overlay");
        let items = forms.preset.store.list().await;
        // config.toml seed (deepseek) + overlay (openai) = 2 条。
        assert_eq!(items.len(), 2);
        assert!(
            broken_backups(dir.path()).is_empty(),
            "正常加载不应产生备份"
        );
        // providers.json 是单一真源：保留在原位（不迁移删除）。
        assert!(
            overlay.exists(),
            "providers.json 应保留（单一真源）：{}",
            overlay.display()
        );
        clear_overlay_env();
    }

    /// 备份命名唯一性：millis + PID 后缀确保同秒内多次「坏文件出现 +
    /// 备份」不会冲突（虽实际中 self-heal 一次性把文件搬走，再次走
    /// load 就空了，但留作回归保护）。
    #[tokio::test]
    async fn backup_filename_is_unique_and_machine_readable() {
        let dir = tempfile::tempdir().unwrap();
        let overlay = dir.path().join("providers.json");
        std::fs::write(&overlay, "{").unwrap();

        let backup = backup_broken_overlay(&overlay).unwrap();
        let name = backup.file_name().unwrap().to_string_lossy().into_owned();
        // 形如 `providers.json.broken-<millis>-<pid>.json`。
        assert!(name.starts_with("providers.json.broken-"));
        assert!(name.ends_with(".json"));
        let mid = name
            .trim_start_matches("providers.json.broken-")
            .trim_end_matches(".json");
        let parts: Vec<&str> = mid.split('-').collect();
        assert_eq!(
            parts.len(),
            2,
            "备份名中部应为 `<millis>-<pid>`，实际：{mid}"
        );
        assert!(
            parts[0].parse::<u128>().is_ok(),
            "millis 部分应是数字：{}",
            parts[0]
        );
        assert!(
            parts[1].parse::<u32>().is_ok(),
            "pid 部分应是数字：{}",
            parts[1]
        );
    }
}
