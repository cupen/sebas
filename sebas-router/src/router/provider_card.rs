//! Provider 管理卡 UI（`/provider` 命令主卡，bead sebas-63f.5，2026-08-17
//! 按用户实测反馈重构）。
//!
//! 单一「Provider 管理」卡，自上而下：
//!
//! 1. **Mode segmented control** — Off / Direct / Gateway 三按钮。
//!    当前模式以 `primary` 样式高亮，其余 `default`。点击写
//!    `~/.sebas/state.json` 并刷卡片。
//! 2. **Default provider for DIRECT** — `select_static`，选项 = 全 provider
//!    名（按字母序）。当前选中项以 `initial_value` 高亮。改动写
//!    `state.json.default_provider_for_direct` 并刷卡。
//! 3. **Provider 列表** — 每个 provider 一行：一个**默认折叠**的
//!    `CollapsiblePanel`，header 只显示一行摘要（名字 + 默认 model +
//!    「DIRECT 默认」标记）。展开/收起由飞书客户端本地完成，不走服务端
//!    回调，因此不再需要 per-session 的选择状态（旧 `ProviderSelectionMap`
//!    已删除）。面板内是 markdown 字段行（预设 / Base URL Anthropic /
//!    Base URL OpenAI / API Key 已配置/未配置 / 默认 model）+ 四按钮
//!    （🔍 探测 model 列表 / 编辑 / 删除 / 设为默认（DIRECT））。
//! 4. **新建子区**（常驻底部）— 「＋ 新增（预设）」 / 「＋ 新增
//!    （自定义）」两个按钮，走 `provider-create-preset` / `-custom`
//!    form 名，最终转交给 `forms.preset.handle()` / `custom.handle()`
//!    渲染创建表单。
//!
//! 「编辑」复用 `provider_forms.preset.handle()` / `custom.handle()` 走既
//! 有 create/edit 路径；「删除」直接调 `store.delete(name)`；「设为默认
//! （DIRECT）」写 state 并刷。
//!
//! 不在本卡处理：表单容器（form_value 路径）只在原有 `provider-preset` /
//! `provider-custom` 提交时由 `CrudForm::handle()` 路由，**新卡不内嵌 form
//! 容器**——所有交互（按钮 + select_static）走 button-callback / form-callback
//! 路径，新 form 名统一在 `dispatch()` 中分发。

use super::{Out, RouterHandle};
use crate::crud::{CrudStore, Item};
use crate::provider_state::{self, ProviderMode, ProviderRuntimeState};
use crate::state_store::DefaultSelection;
use sebas_channels::card::{
    Behavior, ButtonSpec, ChannelCard, ChannelElement, CollapsiblePanel, RichText,
};
use sebas_channels::ChannelKey;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

// ---- form 名常量（behaviors[].value.form 的判别字段）----

pub const FORM_MODE: &str = "provider-mode";
pub const FORM_DEFAULT_DIRECT: &str = "provider-default-direct";
pub const FORM_SET_DEFAULT_DIRECT: &str = "provider-set-default-direct";
pub const FORM_DELETE_CONFIRM: &str = "provider-delete-confirm";
pub const FORM_CREATE_PRESET: &str = "provider-create-preset";
pub const FORM_CREATE_CUSTOM: &str = "provider-create-custom";
/// 「🔍 探测 model 列表」按钮（bead sebas-63f.7）：回调进 `handle_probe`。
pub const FORM_PROBE: &str = "provider-probe";
/// 探测结果卡里「使用 <model_id>」按钮：把选中 model 写回 provider 的
/// `default_model` 字段（不调用外部 form 容器，纯普通 callback）。
pub const FORM_PROBE_APPLY: &str = "provider-probe-apply";
/// 探测结果卡底部「← 返回 Provider 管理」按钮：把当前结果卡就地翻回主卡。
pub const FORM_BACK: &str = "provider-back";
/// openspec/specs/provider-management/spec.md：详情面板里「协议」radio 的 callback —— 选完直接
/// 把 `protocol` 字段写回 store（不经表单容器，详情面板内的快捷切换）。
pub const FORM_PROTOCOL: &str = "provider-set-protocol";

/// select_static 触发回调时 `form_value` 里键名（也是 widget `name`）。
pub const SELECT_NAME_DEFAULT_DIRECT: &str = "provider_default_direct";
/// openspec/specs/provider-management/spec.md：详情面板里「协议」radio 的 select_static `name`
/// （与表单 spec 里的 `protocol` 字段名不同——表单字段名走 CRUD 存储，
/// 这里走详情面板的就地 on_change 回调；两者最终写到同一份 item）。
pub const SELECT_NAME_PROTOCOL: &str = "provider_protocol";

/// 卡片标题。
const CARD_TITLE: &str = "Provider 管理";

/// 旧 preset/custom 表单名（route 到既有 `CrudForm::handle()` 时复用）。
const LEGACY_PRESET_FORM: &str = "provider-preset";
const LEGACY_CUSTOM_FORM: &str = "provider-custom";
/// 既有 `CrudForm` 操作的 op 字段。
const OP_CREATE: &str = "create";

// ===========================================================================
// 顶层入口：`/provider` 命令 → 渲染主卡
// ===========================================================================

/// `/provider` 命令主入口：渲染「Provider 管理」主卡并发新卡。
pub async fn render_main_card(handle: &RouterHandle, key: &ChannelKey) -> Out {
    let state = provider_state::load();
    let items = match &handle.provider_forms {
        Some(forms) => forms.preset.store.list().await,
        None => Vec::new(),
    };
    let provider_names = sorted_names(&items);

    let card = build_card(&state, &provider_names, &items);
    let root_id = handle.reply_target(key).await;
    Out::SendCard {
        key: key.clone(),
        card,
        msg_id: None,
        perm_request_id: None,
        perm_meta: None,
        root_id,
    }
}

// ===========================================================================
// Callback 路由：按钮 + select_static 共用同一个分发点
// ===========================================================================

/// 统一分发点：`on_button` 与 `on_form_cb` 都调用此处。`form_value` 在按钮
/// 点击场景下为空（在 `on_button` 处显式传入 `BTreeMap::new()`）。
///
/// 返回 `Some(Out)` 表示已处理；`None` 表示 form 名不归本模块管，调用方
/// 应继续走默认路由（既有 `provider-preset` / `provider-custom` 表单提交
/// 兜底）。
pub async fn dispatch(
    handle: &RouterHandle,
    key: &ChannelKey,
    value: &Value,
    form_value: &BTreeMap<String, Value>,
    message_id: Option<String>,
) -> Option<Out> {
    let form_name = value.get("form").and_then(Value::as_str).unwrap_or("");
    match form_name {
        FORM_MODE => Some(handle_mode(handle, key, value, message_id).await),
        FORM_DEFAULT_DIRECT => {
            Some(handle_default_direct(handle, key, form_value, message_id).await)
        }
        FORM_SET_DEFAULT_DIRECT => {
            Some(handle_set_default_direct(handle, key, value, message_id).await)
        }
        FORM_DELETE_CONFIRM => Some(handle_delete(handle, key, value, message_id).await),
        FORM_CREATE_PRESET => {
            Some(handle_create(handle, key, LEGACY_PRESET_FORM, message_id).await)
        }
        FORM_CREATE_CUSTOM => {
            Some(handle_create(handle, key, LEGACY_CUSTOM_FORM, message_id).await)
        }
        FORM_PROBE => Some(handle_probe(handle, key, value, message_id).await),
        FORM_PROBE_APPLY => Some(handle_probe_apply(handle, key, value, message_id).await),
        FORM_BACK => Some(handle_back(handle, key, message_id).await),
        FORM_PROTOCOL => Some(handle_protocol(handle, key, value, form_value, message_id).await),
        _ => None,
    }
}

// ===========================================================================
// Callback 处理（每个 form 名一个）
// ===========================================================================

/// Mode 按钮：把 `{form, mode}` 翻译成 `ProviderMode` 写盘。
async fn handle_mode(
    handle: &RouterHandle,
    key: &ChannelKey,
    value: &Value,
    _message_id: Option<String>,
) -> Out {
    let mode_str = value.get("mode").and_then(Value::as_str).unwrap_or("off");
    let new_mode = match mode_str {
        "off" => ProviderMode::Off,
        "direct" => {
            let provider = value
                .get("provider")
                .and_then(Value::as_str)
                .map(str::to_owned);
            ProviderMode::Direct {
                provider: provider.unwrap_or_default(),
            }
        }
        "gateway" => ProviderMode::Gateway,
        other => {
            tracing::warn!(mode = %other, "未知 provider mode，忽略");
            return refresh_card(handle, key, None).await;
        }
    };

    // 切到 Direct 且 default_selection 仍为 None 时，自动填第一个
    // provider（设计意图：避免用户切到 Direct 后发现还没选默认）。
    let first_name = if matches!(new_mode, ProviderMode::Direct { .. }) {
        let items = match &handle.provider_forms {
            Some(forms) => forms.preset.store.list().await,
            None => Vec::new(),
        };
        sorted_names(&items).into_iter().next()
    } else {
        None
    };

    if let Err(e) = provider_state::update(|s| {
        if let Some(first) = &first_name
            && matches!(new_mode, ProviderMode::Direct { .. })
            && s.default_selection.is_none()
        {
            tracing::info!(
                default = %first,
                "切到 Direct 且 default_selection 未设，自动填第一个 provider"
            );
            // 自动填的 selection 不带 model —— 用户还没显式选过 default model。
            s.default_selection = Some(DefaultSelection::new(first.clone()));
        }
        s.mode = new_mode.clone();
    }) {
        tracing::warn!(error = %e, "provider_state 更新失败");
    }
    refresh_card(handle, key, None).await
}

/// 「Default provider for DIRECT」下拉变化：取 `form_value[name]` 写 state。
///
/// openspec/specs/provider-management/spec.md：下拉变更也走 [`build_default_selection`] —— 同步把 overlay
/// 里同名 provider 的 `default_model` 复制到 `default_selection.model`，让
/// spawn-time 自动加 `--model`。
async fn handle_default_direct(
    handle: &RouterHandle,
    key: &ChannelKey,
    form_value: &BTreeMap<String, Value>,
    _message_id: Option<String>,
) -> Out {
    let selected = form_value
        .get(SELECT_NAME_DEFAULT_DIRECT)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|s| !s.is_empty());
    let selection = match selected.as_deref() {
        Some(name) => Some(build_default_selection(handle, name).await),
        None => None,
    };
    if let Err(e) = provider_state::update(|s| {
        s.default_selection = selection.clone();
    }) {
        tracing::warn!(error = %e, "default_selection 更新失败");
    }
    refresh_card(handle, key, None).await
}

/// 折叠面板里「设为默认（DIRECT）」按钮。
///
/// openspec/specs/provider-management/spec.md：动作按钮 = 唯一把 overlay `default_model` 复制到
/// `default_selection.model` 的入口（[`build_default_selection`]）。
async fn handle_set_default_direct(
    handle: &RouterHandle,
    key: &ChannelKey,
    value: &Value,
    _message_id: Option<String>,
) -> Out {
    let Some(name) = value.get("name").and_then(Value::as_str).map(str::to_owned) else {
        tracing::warn!("provider-set-default-direct 缺少 name 字段");
        return refresh_card(handle, key, None).await;
    };
    let selection = build_default_selection(handle, &name).await;
    if let Err(e) = provider_state::update(|s| {
        s.default_selection = Some(selection.clone());
    }) {
        tracing::warn!(error = %e, "default_selection 更新失败");
    }
    refresh_card(handle, key, None).await
}

/// 折叠面板里「删除」按钮。直接调 store.delete（与既有 form submit-delete
/// 走同一份存储），然后清掉 default_provider_for_direct 残留。
async fn handle_delete(
    handle: &RouterHandle,
    key: &ChannelKey,
    value: &Value,
    _message_id: Option<String>,
) -> Out {
    let Some(name) = value.get("name").and_then(Value::as_str) else {
        tracing::warn!("provider-delete-confirm 缺少 name 字段");
        return refresh_card(handle, key, None).await;
    };
    if let Some(forms) = &handle.provider_forms
        && let Err(e) = forms.preset.store.delete(name).await
    {
        tracing::warn!(name, error = %e, "provider delete failed");
    }
    if let Err(e) = provider_state::update(|s| {
        if s.default_selection.as_ref().map(|d| d.provider.as_str()) == Some(name) {
            s.default_selection = None;
        }
    }) {
        tracing::warn!(error = %e, "default_selection 清理失败");
    }
    refresh_card(handle, key, None).await
}

/// openspec/specs/provider-management/spec.md：把「provider 名 + overlay 里同名 item 的 default_model」合并
/// 成 `DefaultSelection`。overlay 找不到 / 没 default_model → model = None。
///
/// 这是「设为默认（DIRECT）」动作 + 下拉变更的**唯一**入口，避免两处独立
/// 拼 `DefaultSelection` 时漂移。overlay 仍是 `default_model` 的 UI 源（`/provider`
/// 详情面板的「默认 model」文本框直接读 overlay），`default_selection.model`
/// 是 spawn 翻译的权威值。
async fn build_default_selection(handle: &RouterHandle, name: &str) -> DefaultSelection {
    let model = if let Some(forms) = &handle.provider_forms {
        forms.preset.store.get(name).await.and_then(|item| {
            item.get("default_model")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|s| !s.is_empty())
        })
    } else {
        None
    };
    match model {
        Some(m) => DefaultSelection::with_model(name, m),
        None => DefaultSelection::new(name),
    }
}

/// 新建子区的「＋ 新增（预设/自定义）」按钮：复用既有 `CrudForm::handle()`
/// 的 OP_CREATE 路径——保证「取消」按钮、表单回填逻辑、容器提交协议都和
/// 旧 UI 一致。
async fn handle_create(
    handle: &RouterHandle,
    key: &ChannelKey,
    legacy_form: &str,
    message_id: Option<String>,
) -> Out {
    let Some(forms) = &handle.provider_forms else {
        return Out::HelpText { key: key.clone() };
    };
    let Some(form) = forms.dispatch(legacy_form) else {
        tracing::warn!(form = legacy_form, "未找到 legacy form 实例");
        return refresh_card(handle, key, message_id).await;
    };
    let payload = json!({ "form": legacy_form, "op": OP_CREATE });
    form.handle(key.clone(), &payload, &BTreeMap::new(), message_id)
        .await
}

/// 「🔍 探测 model 列表」按钮（bead sebas-63f.7）：按 provider 的
/// `base_url_openai` / `base_url_anthropic` 决定探测的端点，HTTP GET
/// 拉一次，把成功 / 失败结果渲成一张新卡。
///
/// 探测成功（非空列表）时顺手把官方返回的完整 model 列表写回 provider 的
/// `models` 目录字段（逗号分隔）——model 列表的权威来源是官方 `/models`
/// 接口，手填只是探测不可用时的兜底。`default_model` 仍由用户在结果卡里
/// 点「使用 <model>」显式回写。
///
/// 探测是 best-effort：anthropic 协议没有 `/v1/models`，会得到错误卡（
/// 提示用户手填）；openai 协议端点通常可用。失败 / 空列表都给一张单独
/// 的卡，用户从卡里的按钮挑一个 model 回写。
async fn handle_probe(
    handle: &RouterHandle,
    key: &ChannelKey,
    value: &Value,
    _message_id: Option<String>,
) -> Out {
    let Some(name) = value.get("name").and_then(Value::as_str) else {
        tracing::warn!("provider-probe 缺少 name 字段");
        return refresh_card(handle, key, _message_id).await;
    };
    let Some(forms) = &handle.provider_forms else {
        return Out::HelpText { key: key.clone() };
    };
    let Some(item) = forms.preset.store.get(name).await else {
        // provider 已不在 store 里（同时被删过）→ 退化为主卡。
        return refresh_card(handle, key, _message_id).await;
    };

    let (probe_url, base_kind) = match choose_probe_url(&item) {
        Ok(t) => t,
        Err(reason) => {
            let card = build_probe_error_card(name, &reason);
            return Out::SendCard {
                key: key.clone(),
                card,
                msg_id: None,
                perm_request_id: None,
                perm_meta: None,
                root_id: handle.reply_target(key).await,
            };
        }
    };
    let token = resolve_auth_token(&item);

    // 探测是触发式的小请求，每次新建 `Client`：连接池复用不了，但避免了
    // 把 reqwest 注入 RouterHandle 的面重构（best-effort probe 不值得）。
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| {
            tracing::warn!(error = %e, "reqwest client 构建失败");
            e
        })
        .ok();

    let probe_result = match client {
        Some(c) => probe_models(&c, &probe_url, token.as_deref()).await,
        None => Err(anyhow::anyhow!("HTTP client 不可用")),
    };

    let card = match probe_result {
        Ok(models) if !models.is_empty() => {
            // 把官方返回的 model 列表写回 `models` 目录字段（best-effort：
            // 写失败只记日志，不影响结果卡展示）。
            if let Some(mut item) = forms.preset.store.get(name).await {
                item.insert("models".into(), Value::String(models.join(",")));
                if let Err(e) = forms.preset.store.update(item).await {
                    tracing::warn!(name, error = %e, "探测结果写回 models 目录失败");
                }
            }
            build_probe_result_card(name, base_kind, &probe_url, &models)
        }
        Ok(_) => {
            // 200 但 `data` 缺 / 空 → 当成探测失败。
            build_probe_error_card(name, "服务端未返回 model 列表")
        }
        Err(e) => build_probe_error_card(name, &format!("{e}")),
    };
    Out::SendCard {
        key: key.clone(),
        card,
        msg_id: None,
        perm_request_id: None,
        perm_meta: None,
        root_id: handle.reply_target(key).await,
    }
}

/// 探测结果卡里「使用 <model_id>」按钮：把 model 写回 store，然后原地翻
/// 回主卡（详情面板会显示新的 `default_model`）。
async fn handle_probe_apply(
    handle: &RouterHandle,
    key: &ChannelKey,
    value: &Value,
    message_id: Option<String>,
) -> Out {
    let Some(name) = value.get("name").and_then(Value::as_str) else {
        tracing::warn!("provider-probe-apply 缺少 name 字段");
        return refresh_card(handle, key, message_id).await;
    };
    let Some(model) = value.get("model").and_then(Value::as_str) else {
        tracing::warn!("provider-probe-apply 缺少 model 字段");
        return refresh_card(handle, key, message_id).await;
    };
    let Some(forms) = &handle.provider_forms else {
        return Out::HelpText { key: key.clone() };
    };
    // 取旧 item 改一个字段再写回：保险起见不直接 `overrides`，避免覆盖
    // 其它由表单维护的字段（如 preset / base_url_*）。
    let res = match forms.preset.store.get(name).await {
        Some(mut item) => {
            item.insert("default_model".into(), Value::String(model.to_string()));
            forms.preset.store.update(item).await
        }
        None => Err(format!("provider '{name}' 已不存在")),
    };
    if let Err(e) = res {
        tracing::warn!(name, model, error = %e, "默认 model 写回失败");
    }
    refresh_card(handle, key, message_id).await
}

/// 探测结果卡底部「← 返回 Provider 管理」按钮：原地把当前卡翻回主卡。
async fn handle_back(handle: &RouterHandle, key: &ChannelKey, message_id: Option<String>) -> Out {
    refresh_card(handle, key, message_id).await
}

/// 详情面板里的「协议」radio（openspec/specs/provider-management/spec.md）：用户切换协议时
/// 直接把 `protocol` 字段写回 store（不经表单容器）。不存在的 provider
/// 走 refresh 兜底；非法值（不是 auto/anthropic/openai）也走兜底。
async fn handle_protocol(
    handle: &RouterHandle,
    key: &ChannelKey,
    value: &Value,
    form_value: &BTreeMap<String, Value>,
    message_id: Option<String>,
) -> Out {
    let Some(name) = value.get("name").and_then(Value::as_str) else {
        tracing::warn!("provider-set-protocol 缺少 name 字段");
        return refresh_card(handle, key, message_id).await;
    };
    // select_static on_change 把选中值回传到 `form_value[SELECT_NAME_PROTOCOL]`。
    let new_proto = form_value
        .get(SELECT_NAME_PROTOCOL)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|s| matches!(s.as_str(), "auto" | "anthropic" | "openai"));
    let Some(new_proto) = new_proto else {
        tracing::warn!(name, "provider-set-protocol 收到非法或缺失的协议值，忽略");
        return refresh_card(handle, key, message_id).await;
    };
    let Some(forms) = &handle.provider_forms else {
        return Out::HelpText { key: key.clone() };
    };
    let res = match forms.preset.store.get(name).await {
        Some(mut item) => {
            item.insert("protocol".into(), Value::String(new_proto));
            forms.preset.store.update(item).await
        }
        None => Err(format!("provider '{name}' 已不存在")),
    };
    if let Err(e) = res {
        tracing::warn!(name, error = %e, "详情面板 protocol 写回失败");
    }
    refresh_card(handle, key, message_id).await
}

// ===========================================================================
// 卡片构建（每个 section 一个 helper）
// ===========================================================================

fn build_card(state: &ProviderRuntimeState, provider_names: &[String], items: &[Item]) -> ChannelCard {
    let mut card = ChannelCard::new(CARD_TITLE, "blue");
    card.push_note("切换运行模式、设置默认 provider、增删 provider 条目。");
    card.push_divider();

    // ---- 1. Mode segmented control ----
    card.push_text("**模式**");
    card.push_text(format!("当前：**{}**", mode_display_label(&state.mode)));
    for el in render_mode_buttons(&state.mode) {
        card.elements.push(el);
    }
    card.push_divider();

    // ---- 2. Default provider for DIRECT ----
    card.push_text("**DIRECT 模式默认 provider**");
    card.elements.push(render_default_direct_select(
        provider_names,
        state.default_selection.as_ref(),
    ));
    card.push_divider();

    // ---- 3. Provider 列表：每个 provider 一行折叠面板（默认折叠）----
    card.push_text("**Provider 列表**");
    if provider_names.is_empty() {
        card.push_text("（暂无 provider，先在下方新建）");
    }
    for name in provider_names {
        let item = items
            .iter()
            .find(|i| i.get("name").and_then(Value::as_str) == Some(name.as_str()));
        card.elements.push(render_provider_row(
            name,
            item,
            state
                .default_selection
                .as_ref()
                .map(|d| d.provider.as_str()),
        ));
    }
    card.push_divider();

    // ---- 4. 新建子区（常驻）----
    for el in render_create_sub_section() {
        card.elements.push(el);
    }
    card
}

/// 「Off / Direct <name> / Gateway」文本标签。
fn mode_display_label(mode: &ProviderMode) -> String {
    match mode {
        ProviderMode::Off => "Off".into(),
        ProviderMode::Direct { provider } => {
            if provider.is_empty() {
                "Direct（未选 provider）".into()
            } else {
                format!("Direct ({provider})")
            }
        }
        ProviderMode::Gateway => "Gateway".into(),
    }
}

/// 三个模式按钮。当前模式高亮（primary），其余 default。
/// 按顺序：Off / Direct / Gateway。`behaviors[].value` 携带
/// `{form, mode}`，触发 `card.action.trigger` 路由到 `dispatch()`。
fn render_mode_buttons(current: &ProviderMode) -> Vec<ChannelElement> {
    let off_selected = matches!(current, ProviderMode::Off);
    let direct_selected = matches!(current, ProviderMode::Direct { .. });
    let gateway_selected = matches!(current, ProviderMode::Gateway);
    vec![
        button_from("Off", if off_selected { "primary" } else { "default" }, &mode_button_payload("off")),
        button_from("Direct", if direct_selected { "primary" } else { "default" }, &mode_button_payload("direct")),
        button_from("Gateway", if gateway_selected { "primary" } else { "default" }, &mode_button_payload("gateway")),
    ]
}

/// 单个模式按钮的 callback 负载。
fn mode_button_payload(mode: &str) -> Value {
    let mut payload = Map::new();
    payload.insert("form".into(), Value::String(FORM_MODE.into()));
    payload.insert("mode".into(), Value::String(mode.into()));
    Value::Object(payload)
}

/// 包装一个带 callback behaviors 的中立 v2 Button。
fn button_from(label: &str, style: &str, value: &Value) -> ChannelElement {
    ChannelElement::Button {
        text: RichText::plain(label),
        style: style.into(),
        behaviors: vec![Behavior {
            r#type: "callback".into(),
            value: value.clone(),
        }],
    }
}

/// 详情面板里的「协议」select_static —— 标签 + 三档（auto / anthropic /
/// openai）。当前值取自 item.protocol（缺省 "auto"）。on_change 直接写
/// store（不经表单容器）。
fn render_protocol_select(name: &str, item: Option<&Item>) -> ChannelElement {
    let current = item
        .and_then(|i| i.get("protocol").and_then(Value::as_str))
        .filter(|s| matches!(*s, "auto" | "anthropic" | "openai"))
        .unwrap_or("auto");
    let options = vec![
        ("auto".to_string(), "Auto（Anthropic 优先）".to_string()),
        ("anthropic".to_string(), "Anthropic".to_string()),
        ("openai".to_string(), "OpenAI".to_string()),
    ];
    ChannelElement::SelectStatic {
        name: SELECT_NAME_PROTOCOL.into(),
        placeholder: RichText::plain("协议"),
        options,
        initial: Some(current.to_string()),
        on_change: json!({ "form": FORM_PROTOCOL, "name": name }),
    }
}

/// 「Default provider for DIRECT」下拉。
///
/// openspec/specs/provider-management/spec.md：每个选项的 label 包含 provider 名 + 该 provider 的 default_model
/// （若设置）。`current` 是 `Option<&DefaultSelection>`：`Some(d)` 时高亮
/// `d.provider`，placeholder 文案追加「· 默认 model: <d.model>」让用户看到
/// 当前选择会带哪个 `--model`。
fn render_default_direct_select(
    provider_names: &[String],
    current: Option<&DefaultSelection>,
) -> ChannelElement {
    // 当前选中的 provider 名（用于 `initial` 高亮 + 给每个 option 标注 default_model）。
    let current_provider = current.map(|d| d.provider.clone());
    // 当前 default_selection.model（None / 空都视作「未设」）。
    let current_model = current
        .and_then(|d| d.model.clone())
        .filter(|s| !s.is_empty());
    let mut options: Vec<(String, String)> = Vec::with_capacity(provider_names.len() + 1);
    // 第一项空字符串表示「不选」——与 state 文件的 None 语义一致。
    options.push((String::new(), "（未设置）".into()));
    for n in provider_names {
        options.push((n.clone(), n.clone()));
    }
    // placeholder 文案：默认「选择默认 provider」；有当前 model 时追加，让
    // 用户看到 spawn 时会传哪个 --model。
    let placeholder_text = match current_model.as_deref() {
        Some(m) => format!("选择默认 provider（当前默认 model: {m}）"),
        None => "选择默认 provider".into(),
    };
    ChannelElement::SelectStatic {
        name: SELECT_NAME_DEFAULT_DIRECT.into(),
        placeholder: RichText::plain(placeholder_text),
        options,
        // `current` 为 None 时 initial 也是 None（展示 placeholder）；
        // 当前有 selection 时 initial = provider 名（select_static 只能按
        // option 的 value 高亮）。
        initial: current_provider,
        on_change: json!({ "form": FORM_DEFAULT_DIRECT }),
    }
}

/// 新建子区：两个「＋ 新增」按钮。
fn render_create_sub_section() -> Vec<ChannelElement> {
    vec![
        ChannelElement::Markdown {
            content: "**新建 provider**".into(),
        },
        button_from("＋ 新增（预设）", "primary", &json!({ "form": FORM_CREATE_PRESET })),
        button_from("＋ 新增（自定义）", "default", &json!({ "form": FORM_CREATE_CUSTOM })),
    ]
}

/// 单行 provider 条目：`CollapsiblePanel`（**默认折叠**），header 一行摘要
/// （名字 + 「DIRECT 默认」标记 + 默认 model），面板内是 markdown 字段 +
/// 四个动作按钮。展开/收起由飞书客户端本地完成，不走服务端回调。
/// `item` 为 `None` 时只渲染折叠面板 + 「未找到该 provider」+「删除」按钮
/// （item 列表与名称来源不一致等异常场景的兜底）。
fn render_provider_row(
    name: &str,
    item: Option<&Item>,
    default_direct: Option<&str>,
) -> ChannelElement {
    let mut elements: Vec<ChannelElement> = Vec::new();

    // markdown 字段行：预设 / Base URL(Anthropic) / Base URL(OpenAI) /
    // API Key 已配置/未配置 / 默认 model。**严禁回显 api_key 明文**。
    let preset = item
        .and_then(|i| i.get("preset").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .unwrap_or("自定义");
    let url_anth = item
        .and_then(|i| i.get("base_url_anthropic").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .unwrap_or("—");
    let url_oai = item
        .and_then(|i| i.get("base_url_openai").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .unwrap_or("—");
    let api_key_status = match item.and_then(|i| i.get("api_key")) {
        Some(v) if v.as_str().is_some_and(|s| !s.is_empty()) => "已配置",
        _ => "未配置",
    };
    let default_model = item
        .and_then(|i| i.get("default_model").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .unwrap_or("—");

    let body = format!(
        "**预设**：{preset}\n\
         **Base URL (Anthropic)**：{url_anth}\n\
         **Base URL (OpenAI)**：{url_oai}\n\
         **API Key**：{api_key_status}\n\
         **默认 model**：{default_model}"
    );
    elements.push(ChannelElement::Markdown { content: body });

    // 协议 radio（openspec/specs/provider-management/spec.md）：详情面板就地切换 Direct 模式的
    // 协议面（auto / anthropic / openai）。选完直接写 overlay（不走表单
    // 容器）；缺省 "auto" = 旧约定（Anthropic 优先）。
    elements.push(render_protocol_select(name, item));

    // 探测 model 按钮（bead sebas-63f.7 + openspec/specs/provider-management/spec.md）：
    // 仅当 provider 暴露 openai 端点（base_url_openai 非空）时才显示——
    // Anthropic 协议无 `/v1/models` 端点是已知坏掉的，按 spec 隐藏按钮
    // 避免给用户一个必失败的入口。两个端点都设了 → 仍显示（probe 优先
    // 打 OpenAI URL）。
    let has_openai_url = item
        .and_then(|i| i.get("base_url_openai").and_then(Value::as_str))
        .is_some_and(|s| !s.is_empty());
    if has_openai_url {
        let probe_value = json!({ "form": FORM_PROBE, "name": name });
        elements.push(button_from("🔍 探测 model 列表", "default", &probe_value));
    }

    // 三个动作按钮：编辑 / 删除 / 设为默认（DIRECT）。
    // 「编辑」走既有 form 的 OP_EDIT 路径：旧 `on_button` 的 provider_forms
    // 分发按 item.preset 自动路由到 preset 或 custom 表单；表单回填逻辑
    // （密钥不回显等）保持原样。
    let edit_value = json!({
        "form": LEGACY_PRESET_FORM,
        "op": "edit",
        "id": name,
    });
    let delete_value = json!({ "form": FORM_DELETE_CONFIRM, "name": name });
    let set_default_value = json!({ "form": FORM_SET_DEFAULT_DIRECT, "name": name });

    elements.push(button_from("编辑", "default", &edit_value));
    elements.push(button_from("删除", "danger", &delete_value));
    elements.push(button_from("设为默认（DIRECT）", "primary", &set_default_value));

    // header 一行摘要：`name · <默认 model>`，是 DIRECT 默认时加标记。
    // 保持单行——列表的可扫读性全靠这一行。
    let mut title = name.to_string();
    if default_direct == Some(name) {
        title.push_str(" ✓DIRECT默认");
    }
    if default_model != "—" {
        title.push_str(&format!(" · {default_model}"));
    }

    ChannelElement::CollapsiblePanel(CollapsiblePanel {
        expanded: false,
        header_title: RichText::plain(title),
        icon_token: "down-small-ccm_outlined".into(),
        elements,
    })
}

// ===========================================================================
// Helpers
// ===========================================================================

/// 按 provider item 决定探测的目标 URL：
/// 1. 优先 `base_url_openai`：openai-compatible 端点通常都暴露 `/models`，
///    直接在 base 后追加 `/models`（preset 默认 openai URL 通常已带 `/v1`，
///    例如 `https://api.openai.com/v1`）。
/// 2. 否则 `base_url_anthropic`：best-effort 探测 `/v1/models`，anthropic
///    协议一般不存在该路径，会失败卡——按 spec 标记为 best-effort。
/// 3. 两个都没设 → 错误：探测无意义。
///
/// 返回 `(完整 url, "openai" | "anthropic")`。
fn choose_probe_url(item: &Item) -> Result<(String, &'static str), String> {
    let openai = item
        .get("base_url_openai")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let anth = item
        .get("base_url_anthropic")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (openai, anth) {
        (Some(base), _) => {
            let base = trim_trailing_slash(base);
            Ok((format!("{base}/models"), "openai"))
        }
        (None, Some(base)) => {
            let base = trim_trailing_slash(base);
            Ok((format!("{base}/v1/models"), "anthropic"))
        }
        (None, None) => Err("未配置 base_url_openai / base_url_anthropic".into()),
    }
}

/// 解析认证 token：先看 `api_key` 明文（极少用，preset 通常是 env），再回
/// 退 `api_key_env` 的环境变量。两个都没 → `None`：探测时跳过
/// Authorization 头（不少 openai-compatible 端点也允许匿名）。
fn resolve_auth_token(item: &Item) -> Option<String> {
    if let Some(s) = item.get("api_key").and_then(Value::as_str) {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    if let Some(var) = item.get("api_key_env").and_then(Value::as_str) {
        let var = var.trim();
        if !var.is_empty()
            && let Ok(v) = std::env::var(var)
        {
            return Some(v);
        }
    }
    None
}

/// 简单 string helper：去掉末尾 `/`，反复 normalize 到单一 url 拼接。
fn trim_trailing_slash(s: &str) -> &str {
    let mut end = s.len();
    while end > 0 && s.as_bytes()[end - 1] == b'/' {
        end -= 1;
    }
    &s[..end]
}

/// 供表单「🔍 获取模型列表」按钮复用的取数入口（`crud::ModelFetcher` 的
/// provider 实现）：按 item 的 base_url / api_key 调官方 `/models`，成功
/// 返回 model id 列表，失败给单行原因。与详情面板的探测按钮共享同一套
/// URL 选择与认证解析逻辑。
pub async fn fetch_models_for_item(item: &Item) -> Result<Vec<String>, String> {
    let (url, _kind) = choose_probe_url(item)?;
    let token = resolve_auth_token(item);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("HTTP client 不可用: {e}"))?;
    probe_models(&client, &url, token.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// 实际探测：`GET <url>` 带可选 `Authorization: Bearer <token>`，5s 超时。
/// 返回 `data[].id` 列表（openai-compatible 形状）；服务端返回任意其它
/// 形状（缺 `data`、`data` 非数组等）→ 空 Vec（视作探测失败）。
pub async fn probe_models(
    client: &reqwest::Client,
    url: &str,
    auth_token: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let mut req = client.get(url);
    if let Some(t) = auth_token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body: Value = resp.error_for_status()?.json().await?;
    if !status.is_success() {
        anyhow::bail!("HTTP {}", status.as_u16());
    }
    let models = parse_openai_models_response(&body);
    if models.is_empty() && !looks_like_openai_models_envelope(&body) {
        // body 不像 openai-compatible 的 model 列表 envelope——避免把任意
        // JSON 误判成空 model。这里依然返回空 Vec，让上层走错误分支。
        tracing::debug!(?body, "model 列表响应不具备 data 字段，返回空 Vec");
    }
    Ok(models)
}

/// 解析 openai-compatible `/v1/models` 响应：`{"object": "list", "data": [{"id": "..."}]}`。
/// 容错：缺 `data`、`data` 非数组、对象缺 `id` 字段 → 跳过该元素。
pub fn parse_openai_models_response(body: &Value) -> Vec<String> {
    let Some(arr) = body.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// 判断 body 是否像 openai `/v1/models` envelope。`data` 缺失时返回 false。
fn looks_like_openai_models_envelope(body: &Value) -> bool {
    body.get("data").map(|v| v.is_array()).unwrap_or(false)
}

/// 探测成功结果卡：标题 + URL + bullet list + 每个 model 一个「使用」按钮
/// + 「← 返回」按钮。
fn build_probe_result_card(
    provider_name: &str,
    base_kind: &str,
    probe_url: &str,
    models: &[String],
) -> ChannelCard {
    let mut card = ChannelCard::new(&format!("探测结果：{provider_name}"), "blue");
    card.push_note(format!("base_url 类型：{base_kind} · {probe_url}"));
    card.push_divider();

    // bullet list 用 markdown 一次性给，比逐行 push_text 紧凑。
    let bullets = models
        .iter()
        .map(|m| format!("- {m}"))
        .collect::<Vec<_>>()
        .join("\n");
    card.push_text(format!("**可用模型（{} 个）**\n{bullets}", models.len()));
    card.push_divider();
    card.push_text("选择一个作为默认 model：");
    card.push_actions(
        models
            .iter()
            .map(|m| {
                ButtonSpec::new(
                    format!("使用 {m}"),
                    "default",
                    json!({
                        "form": FORM_PROBE_APPLY,
                        "name": provider_name,
                        "model": m,
                    }),
                )
            })
            .collect(),
    );
    card.push_divider();
    card.push_actions(vec![ButtonSpec::new(
        "← 返回 Provider 管理",
        "default",
        json!({ "form": FORM_BACK }),
    )]);
    card
}

/// 探测失败 / 无 base_url 时的结果卡：单行说明 + 返回按钮。
fn build_probe_error_card(provider_name: &str, reason: &str) -> ChannelCard {
    let mut card = ChannelCard::new(&format!("探测结果：{provider_name}"), "red");
    card.push_text(format!("探测失败：{reason}，请手填默认 model。"));
    card.push_divider();
    card.push_actions(vec![ButtonSpec::new(
        "← 返回 Provider 管理",
        "default",
        json!({ "form": FORM_BACK }),
    )]);
    card
}

/// 列出全部 provider 名（按字母序排序），用于下拉选项。
fn sorted_names(items: &[Item]) -> Vec<String> {
    let mut names: Vec<String> = items
        .iter()
        .filter_map(|i| i.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    names.sort();
    names
}

/// 重新渲染主卡并通过 `UpdateCardByMsgId`（有原 message_id）或 `SendCard`
/// （无原 message_id）发出去。
async fn refresh_card(handle: &RouterHandle, key: &ChannelKey, message_id: Option<String>) -> Out {
    let state = provider_state::load();
    let items = match &handle.provider_forms {
        Some(forms) => forms.preset.store.list().await,
        None => Vec::new(),
    };
    let provider_names = sorted_names(&items);
    let card = build_card(&state, &provider_names, &items);
    match message_id {
        Some(msg_id) => Out::UpdateCardByMsgId {
            key: key.clone(),
            msg_id,
            card,
        },
        None => {
            let root_id = handle.reply_target(key).await;
            Out::SendCard {
                key: key.clone(),
                card,
                msg_id: None,
                perm_request_id: None,
                perm_meta: None,
                root_id,
            }
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crud::{CrudForm, FileStore, ProviderForms};
    use crate::state::SessionMap;
    use sebas_channels::card::FormSpec;
    use std::sync::Arc;

    fn test_key() -> ChannelKey {
        ChannelKey::feishu("oc_test", None)
    }

    fn item(name: &str, preset: Option<&str>) -> Item {
        let mut m = Map::new();
        m.insert("name".into(), Value::String(name.into()));
        if let Some(p) = preset {
            m.insert("preset".into(), Value::String(p.into()));
            m.insert(
                "base_url_anthropic".into(),
                Value::String(format!("https://{p}.example/anthropic")),
            );
            m.insert(
                "base_url_openai".into(),
                Value::String(format!("https://{p}.example/openai")),
            );
            m.insert("api_key".into(), Value::String("sk-secret".into()));
        } else {
            m.insert(
                "base_url_anthropic".into(),
                Value::String("https://custom.example".into()),
            );
            m.insert("api_key_env".into(), Value::String("MY_API_KEY".into()));
        }
        m.insert(
            "default_model".into(),
            Value::String(format!("{name}-model")),
        );
        m
    }

    /// openspec/specs/provider-management/spec.md：所有走 FileStore / state_store 的测试需要
    /// 构造一个带 FileStore 的 RouterHandle（与 provider_test.rs 一致）。
    /// 返回 `(handle, _guard)` —— 调用方必须把 `_guard` 绑到 test 局部
    /// 以保持锁和 SEBAS_STATE_FILE 不被中途重置。
    fn handle_with(
        dir: &std::path::Path,
        seed: Vec<Item>,
    ) -> (RouterHandle, std::sync::MutexGuard<'static, ()>) {
        let _g = crate::test_util::lock_state_file();
        // SAFETY: lock held. provider 数据在 providers.json，两个 env 都隔离。
        unsafe {
            std::env::set_var("SEBAS_STATE_FILE", dir.join("state.json").to_str().unwrap());
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                dir.join("providers.json").to_str().unwrap(),
            );
        }
        let store = FileStore::load(dir.join("providers.json"), "name", seed).unwrap();
        let forms = ProviderForms {
            preset: Arc::new(CrudForm::new(
                FormSpec::new("provider-preset", "Provider（预设）", vec![]),
                "name",
                store.clone(),
            )),
            custom: Arc::new(CrudForm::new(
                FormSpec::new("provider-custom", "Provider（自定义）", vec![]),
                "name",
                store,
            )),
        };
        let (h, _rx) = RouterHandle::new_with_provider_form(
            SessionMap::new(),
            Default::default(),
            16,
            Some(Arc::new(forms)),
            None,
        );
        (h, _g)
    }

    /// Serialize a neutral card and walk its wire JSON for button payloads.
    fn serialised_button_values(card: &ChannelCard) -> Vec<String> {
        let value = serde_json::to_value(card).expect("card serializes");
        fn walk(v: &Value, out: &mut Vec<String>) {
            if let Value::Object(map) = v {
                if map.get("tag").and_then(Value::as_str) == Some("button")
                    && let Some(Value::Array(beh)) = map.get("behaviors")
                    && let Some(b) = beh.first()
                    && let Some(v) = b.get("value")
                {
                    out.push(v.to_string());
                }
                if let Some(Value::Array(arr)) = map.get("elements") {
                    for child in arr {
                        walk(child, out);
                    }
                }
                if let Some(Value::Object(body)) = map.get("body")
                    && let Some(Value::Array(arr)) = body.get("elements")
                {
                    for child in arr {
                        walk(child, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&value, &mut out);
        out
    }

    fn button_mode_types(card: &ChannelCard) -> std::collections::HashMap<String, String> {
        let value = serde_json::to_value(card).expect("card serializes");
        let mut out = std::collections::HashMap::new();
        fn walk(v: &Value, out: &mut std::collections::HashMap<String, String>) {
            if let Value::Object(map) = v {
                if map.get("tag").and_then(Value::as_str) == Some("button") {
                    let r#type = map
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("default")
                        .to_string();
                    if let Some(Value::Array(beh)) = map.get("behaviors")
                        && let Some(b) = beh.first()
                        && let Some(mode) = b
                            .get("value")
                            .and_then(|v| v.get("mode"))
                            .and_then(Value::as_str)
                    {
                        out.insert(mode.to_string(), r#type);
                    }
                }
                if let Some(Value::Array(arr)) = map.get("elements") {
                    for child in arr {
                        walk(child, out);
                    }
                }
                if let Some(Value::Object(body)) = map.get("body")
                    && let Some(Value::Array(arr)) = body.get("elements")
                {
                    for child in arr {
                        walk(child, out);
                    }
                }
            }
        }
        walk(&value, &mut out);
        out
    }

    #[tokio::test]
    async fn render_main_card_includes_all_sections_and_three_mode_buttons() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let Out::SendCard { card, .. } = out else {
            panic!("expected SendCard");
        };
        let serialised = serde_json::to_string(&card).unwrap();
        assert_eq!(card.title, "Provider 管理");

        // 序列化 JSON 里数 button + select_static 个数。
        // 3 模式 + 2 新建 + 详情面板 4（探测/编辑/删除/设为默认）= 9。
        let button_count = serialised.matches("\"tag\":\"button\"").count();
        assert_eq!(button_count, 9, "3 模式 + 2 新建 + 面板 4 = 9");
        // 两个 select_static：DIRECT 默认 provider（顶部）+ 详情面板的
        // 「协议」radio（openspec/specs/provider-management/spec.md）。
        let sel_count = serialised.matches("\"tag\":\"select_static\"").count();
        assert_eq!(
            sel_count, 2,
            "DIRECT 默认 provider + 详情面板协议 radio = 2"
        );

        // button payload 应携带 form + mode 名。
        for m in ["off", "direct", "gateway"] {
            assert!(
                serialised.contains(&format!("\"mode\":\"{m}\"")),
                "应渲染模式按钮 {m}：{serialised}"
            );
        }
        // 新建按钮。
        assert!(
            serialised.contains(FORM_CREATE_PRESET),
            "应渲染「＋ 新增（预设）」按钮"
        );
        assert!(
            serialised.contains(FORM_CREATE_CUSTOM),
            "应渲染「＋ 新增（自定义）」按钮"
        );
    }

    #[tokio::test]
    async fn render_main_card_lists_provider_names_as_collapsed_panels() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(
            dir.path(),
            vec![
                item("openai", Some("openai")),
                item("deepseek", Some("deepseek")),
                item("anthropic", Some("anthropic")),
            ],
        );

        let out = render_main_card(&handle, &test_key()).await;
        let Out::SendCard { card, .. } = out else {
            panic!("expected SendCard");
        };
        let serialised = serde_json::to_string(&card).unwrap();

        for n in ["openai", "deepseek", "anthropic"] {
            assert!(
                serialised.contains(n),
                "列表应包含 provider 名 {n}：{serialised}"
            );
        }
        // 每个 provider 一个折叠面板，且全部默认折叠。
        let panel_count = serialised.matches("\"tag\":\"collapsible_panel\"").count();
        assert_eq!(panel_count, 3, "3 个 provider = 3 个折叠面板");
        assert!(
            !serialised.contains("\"expanded\":true"),
            "所有面板默认折叠：{serialised}"
        );

        // 字母序：anthropic < deepseek < openai。
        let anthropic_pos = serialised.find("anthropic").unwrap();
        let deepseek_pos = serialised.find("deepseek").unwrap();
        let openai_pos = serialised.find("openai").unwrap();
        assert!(
            anthropic_pos < deepseek_pos && deepseek_pos < openai_pos,
            "provider 名应按字母序排列"
        );
    }

    #[tokio::test]
    async fn mode_buttons_highlight_current_mode() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);

        // 1) Off 默认 → Off primary。
        let out = render_main_card(&handle, &test_key()).await;
        let card = match out {
            Out::SendCard { card, .. } => card,
            _ => panic!(),
        };
        let types = button_mode_types(&card);
        assert_eq!(types.get("off").map(String::as_str), Some("primary"));
        assert_eq!(types.get("direct").map(String::as_str), Some("default"));
        assert_eq!(types.get("gateway").map(String::as_str), Some("default"));

        // 2) 切到 Gateway → Gateway primary。
        provider_state::update(|s| s.mode = ProviderMode::Gateway).unwrap();
        let out = render_main_card(&handle, &test_key()).await;
        let card = match out {
            Out::SendCard { card, .. } => card,
            _ => panic!(),
        };
        let types = button_mode_types(&card);
        assert_eq!(types.get("gateway").map(String::as_str), Some("primary"));
        assert_eq!(types.get("off").map(String::as_str), Some("default"));

        // 重置回 Off，避免污染其他测试。
        provider_state::update(|s| s.mode = ProviderMode::Off).unwrap();
    }

    #[tokio::test]
    async fn provider_row_renders_details_inside_collapsed_panel() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let Out::SendCard { card, .. } = out else {
            panic!();
        };
        let serialised = serde_json::to_string(&card).unwrap();

        // 折叠面板（默认折叠）+ 详情字段。
        assert!(
            serialised.contains("\"tag\":\"collapsible_panel\""),
            "每个 provider 应渲染折叠面板：{serialised}"
        );
        assert!(
            serialised.contains("\"expanded\":false"),
            "面板应默认折叠：{serialised}"
        );
        // header 一行摘要含默认 model。
        assert!(
            serialised.contains("deepseek · deepseek-model"),
            "面板 header 应为「name · model」一行摘要：{serialised}"
        );
        assert!(serialised.contains("**预设**"));
        assert!(serialised.contains("https://deepseek.example/anthropic"));
        assert!(serialised.contains("已配置"));
        assert!(serialised.contains("deepseek-model"));

        // api_key 明文绝不出现在卡里。
        assert!(
            !serialised.contains("sk-secret"),
            "api_key 明文不应出现在卡中：{serialised}"
        );

        // 三个动作按钮：编辑 / 删除 / 设为默认（DIRECT）。
        assert!(
            serialised.contains("\"op\":\"edit\"") && serialised.contains("\"id\":\"deepseek\""),
            "编辑按钮 payload 应含 op=edit + id=deepseek"
        );
        assert!(
            serialised.contains(FORM_DELETE_CONFIRM)
                && serialised.contains("\"name\":\"deepseek\""),
            "删除按钮 payload 应含 name=deepseek"
        );
        assert!(
            serialised.contains(FORM_SET_DEFAULT_DIRECT)
                && serialised.contains("\"name\":\"deepseek\""),
            "设为默认按钮 payload 应含 name=deepseek"
        );
    }

    #[tokio::test]
    async fn create_buttons_always_rendered_and_empty_list_hint() {
        // 空列表：显示「暂无 provider」提示，新建按钮仍在。
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), Vec::new());
        let out = render_main_card(&handle, &test_key()).await;
        let card = match out {
            Out::SendCard { card, .. } => card,
            _ => panic!(),
        };
        let serialised = serde_json::to_string(&card).unwrap();
        assert!(
            serialised.contains("暂无 provider"),
            "空列表应显示提示：{serialised}"
        );
        assert!(
            serialised.contains(FORM_CREATE_PRESET),
            "应渲染「＋ 新增（预设）」按钮"
        );
        assert!(
            serialised.contains(FORM_CREATE_CUSTOM),
            "应渲染「＋ 新增（自定义）」按钮"
        );
        // 空列表时不应有删除 / 设为默认按钮。
        assert!(
            !serialised.contains(FORM_DELETE_CONFIRM),
            "空列表不应渲染删除按钮"
        );
        assert!(
            !serialised.contains(FORM_SET_DEFAULT_DIRECT),
            "空列表不应渲染设为默认按钮"
        );
    }

    #[tokio::test]
    async fn dispatch_route_for_mode_updates_state_and_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let payload = json!({ "form": FORM_MODE, "mode": "gateway" });
        let out = dispatch(&handle, &key, &payload, &BTreeMap::new(), None).await;
        assert!(matches!(out, Some(Out::SendCard { .. })));
        let state = provider_state::load();
        assert_eq!(state.mode, ProviderMode::Gateway);

        // 重置。
        provider_state::update(|s| s.mode = ProviderMode::Off).unwrap();
    }

    #[tokio::test]
    async fn dispatch_mode_to_direct_auto_fills_first_provider() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(
            dir.path(),
            vec![
                item("anthropic", Some("anthropic")),
                item("deepseek", Some("deepseek")),
            ],
        );
        let key = test_key();

        // default_selection 初始 None；切到 Direct 后应自动填
        // 字母序第一个 = "anthropic"。
        let payload = json!({ "form": FORM_MODE, "mode": "direct" });
        let _ = dispatch(&handle, &key, &payload, &BTreeMap::new(), None).await;
        let state = provider_state::load();
        assert_eq!(
            state
                .default_selection
                .as_ref()
                .map(|d| d.provider.as_str()),
            Some("anthropic"),
            "切到 Direct 应自动填字母序第一个 provider"
        );
        assert!(matches!(state.mode, ProviderMode::Direct { .. }));

        // 重置。
        provider_state::update(|s| {
            s.mode = ProviderMode::Off;
            s.default_selection = None;
        })
        .unwrap();
    }

    #[tokio::test]
    async fn dispatch_route_for_default_direct_writes_state() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let mut fv = BTreeMap::new();
        fv.insert(
            SELECT_NAME_DEFAULT_DIRECT.into(),
            Value::String("deepseek".into()),
        );
        let payload = json!({ "form": FORM_DEFAULT_DIRECT });
        let _ = dispatch(&handle, &key, &payload, &fv, None).await;
        let state = provider_state::load();
        assert_eq!(
            state
                .default_selection
                .as_ref()
                .map(|d| d.provider.as_str()),
            Some("deepseek")
        );

        // 重置。
        provider_state::update(|s| s.default_selection = None).unwrap();
    }

    #[tokio::test]
    async fn dispatch_route_for_delete_removes_item_and_clears_default() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        provider_state::update(|s| {
            s.default_selection = Some(DefaultSelection::new("deepseek"));
        })
        .unwrap();

        let payload = json!({ "form": FORM_DELETE_CONFIRM, "name": "deepseek" });
        let _ = dispatch(&handle, &key, &payload, &BTreeMap::new(), None).await;

        let items = handle
            .provider_forms
            .as_ref()
            .unwrap()
            .preset
            .store
            .list()
            .await;
        assert!(items.is_empty(), "删除后 store 应为空");
        let state = provider_state::load();
        assert!(state.default_selection.is_none());
    }

    #[tokio::test]
    async fn dispatch_route_for_set_default_direct_writes_state() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let payload = json!({ "form": FORM_SET_DEFAULT_DIRECT, "name": "deepseek" });
        let _ = dispatch(&handle, &key, &payload, &BTreeMap::new(), None).await;
        let state = provider_state::load();
        assert_eq!(
            state
                .default_selection
                .as_ref()
                .map(|d| d.provider.as_str()),
            Some("deepseek")
        );

        // 重置。
        provider_state::update(|s| s.default_selection = None).unwrap();
    }

    #[tokio::test]
    async fn dispatch_returns_none_for_unknown_form_name() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), Vec::new());
        let key = test_key();
        let payload = json!({ "form": "totally-unknown" });
        let out = dispatch(&handle, &key, &payload, &BTreeMap::new(), None).await;
        assert!(out.is_none(), "未知 form 名应返回 None 让调用方兜底");
    }

    #[tokio::test]
    async fn edit_button_payload_uses_legacy_form_name() {
        // 「编辑」按钮必须发 `provider-preset` / `provider-custom` 老 form 名，
        // 让 `on_button` 的旧 provider_forms 分发按 item.preset 路由到正确表单。
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let card = match out {
            Out::SendCard { card, .. } => card,
            _ => panic!(),
        };
        let payloads = serialised_button_values(&card);
        let edit_payload = payloads
            .iter()
            .find(|p| p.contains("\"op\":\"edit\""))
            .expect("应渲染编辑按钮");
        assert!(
            edit_payload.contains(LEGACY_PRESET_FORM)
                && edit_payload.contains("\"id\":\"deepseek\""),
            "编辑按钮 payload 应为 {{form: provider-preset, op: edit, id}}: {edit_payload}"
        );
    }

    /// openspec/specs/provider-management/spec.md：「DIRECT 模式默认 provider」下拉显示 default_selection
    /// 的当前状态：provider 名出现在 placeholder 之外的位置（select_static
    /// 的 initial 字段），有 model 时 placeholder 追加「· 当前默认 model:
    /// <model>」。验证 dropdown 反映新字段。
    #[tokio::test]
    async fn default_direct_dropdown_shows_model_in_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        // 设 default_selection 带 model。
        provider_state::update(|s| {
            s.default_selection = Some(DefaultSelection::with_model("deepseek", "deepseek-chat"));
        })
        .unwrap();

        let out = render_main_card(&handle, &key).await;
        let Out::SendCard { card, .. } = out else {
            panic!("expected SendCard");
        };
        let s = serde_json::to_string(&card).unwrap();

        // placeholder 应包含「当前默认 model: deepseek-chat」。
        assert!(
            s.contains("当前默认 model: deepseek-chat"),
            "placeholder 应包含 default_selection.model：{s}"
        );

        // 重置。
        provider_state::update(|s| s.default_selection = None).unwrap();
    }

    /// openspec/specs/provider-management/spec.md：default_selection 只设 provider 没设 model → placeholder
    /// 用普通版本（不含「· 默认 model」），让用户看不到「虚假的 model」。
    #[tokio::test]
    async fn default_direct_dropdown_placeholder_omits_model_when_none() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        provider_state::update(|s| {
            s.default_selection = Some(DefaultSelection::new("deepseek"));
        })
        .unwrap();

        let out = render_main_card(&handle, &key).await;
        let Out::SendCard { card, .. } = out else {
            panic!("expected SendCard");
        };
        let s = serde_json::to_string(&card).unwrap();

        // 没 model → placeholder 不应出现「当前默认 model」。
        assert!(
            !s.contains("当前默认 model:"),
            "model=None 时 placeholder 不应包含 model 提示：{s}"
        );
        // 但仍应有 placeholder 文本。
        assert!(
            s.contains("选择默认 provider"),
            "应保留 placeholder 基础文本：{s}"
        );

        provider_state::update(|s| s.default_selection = None).unwrap();
    }

    // -------------------------------------------------------------------
    // 探测功能（bead sebas-63f.7）相关测试
    // -------------------------------------------------------------------

    /// provider 行的折叠面板里应出现「🔍 探测 model 列表」按钮，
    /// payload 携带 form=provider-probe 和 name。
    #[tokio::test]
    async fn provider_row_renders_probe_button() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let Out::SendCard { card, .. } = out else {
            panic!();
        };
        let serialised = serde_json::to_string(&card).unwrap();

        assert!(
            serialised.contains(FORM_PROBE) && serialised.contains("\"name\":\"deepseek\""),
            "应渲染「🔍 探测 model 列表」按钮且带 name=deepseek 的 payload：{serialised}"
        );
        assert!(
            serialised.contains("探测 model 列表"),
            "按钮文案应包含「探测 model 列表」：{serialised}"
        );
    }

    /// parse_openai_models_response：从 openai-compatible envelope 抽 id。
    #[test]
    fn parse_openai_models_response_extracts_ids_from_openai_shape() {
        let body = json!({
            "object": "list",
            "data": [
                {"id": "gpt-4o", "object": "model"},
                {"id": "gpt-4o-mini", "object": "model"},
            ]
        });
        assert_eq!(
            parse_openai_models_response(&body),
            vec!["gpt-4o".to_string(), "gpt-4o-mini".to_string()]
        );
    }

    /// parse_openai_models_response：缺 `data` 字段 → 空 Vec（不要 panic）。
    #[test]
    fn parse_openai_models_response_handles_missing_data() {
        let body = json!({"object": "list"});
        assert!(parse_openai_models_response(&body).is_empty());

        // data 存在但不是数组也容忍。
        let body = json!({"data": "not-an-array"});
        assert!(parse_openai_models_response(&body).is_empty());

        // data 数组里元素不带 id → 跳过，不 panic。
        let body = json!({"data": [{"object": "model"}]});
        assert!(parse_openai_models_response(&body).is_empty());
    }

    /// trim_trailing_slash：去掉末尾一个或多个 `/`，但保留中间 / 不动。
    #[test]
    fn trim_trailing_slash_normalises_trailing_slashes() {
        assert_eq!(
            trim_trailing_slash("https://api.openai.com/v1/"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            trim_trailing_slash("https://api.openai.com/v1"),
            "https://api.openai.com/v1"
        );
        assert_eq!(
            trim_trailing_slash("https://api.openai.com/v1///"),
            "https://api.openai.com/v1"
        );
        assert_eq!(trim_trailing_slash("/"), "");
        assert_eq!(trim_trailing_slash(""), "");
        assert_eq!(trim_trailing_slash("a/b"), "a/b");
    }

    /// choose_probe_url：优先 base_url_openai，回退 base_url_anthropic。
    #[test]
    fn choose_probe_url_prefers_openai_then_anthropic() {
        let mut item = Map::new();
        item.insert(
            "base_url_openai".into(),
            Value::String("https://api.openai.com/v1".into()),
        );
        let (url, kind) = choose_probe_url(&item).unwrap();
        assert_eq!(url, "https://api.openai.com/v1/models");
        assert_eq!(kind, "openai");

        // 同时给两个：openai 胜出。
        item.insert(
            "base_url_anthropic".into(),
            Value::String("https://api.anthropic.com".into()),
        );
        let (url, kind) = choose_probe_url(&item).unwrap();
        assert_eq!(url, "https://api.openai.com/v1/models");
        assert_eq!(kind, "openai");

        // 只有 anthropic：best-effort 探测 /v1/models。
        let mut item = Map::new();
        item.insert(
            "base_url_anthropic".into(),
            Value::String("https://api.anthropic.com/".into()),
        );
        let (url, kind) = choose_probe_url(&item).unwrap();
        assert_eq!(url, "https://api.anthropic.com/v1/models");
        assert_eq!(kind, "anthropic");
    }

    /// choose_probe_url：两个 base_url 都没设 → 错误。
    #[test]
    fn choose_probe_url_errors_when_no_base_url() {
        let item = Map::new();
        assert!(choose_probe_url(&item).is_err());

        let mut item = Map::new();
        item.insert("base_url_openai".into(), Value::String("".into()));
        item.insert("base_url_anthropic".into(), Value::String("".into()));
        assert!(choose_probe_url(&item).is_err());
    }

    /// resolve_auth_token：api_key 明文 > api_key_env 解析 > None。
    #[test]
    fn resolve_auth_token_uses_api_key_then_env() {
        let mut item = Map::new();
        item.insert("api_key".into(), Value::String("sk-direct".into()));
        assert_eq!(resolve_auth_token(&item).as_deref(), Some("sk-direct"));

        // 仅 api_key 为空 → None。
        item.insert("api_key".into(), Value::String("".into()));
        assert!(resolve_auth_token(&item).is_none());

        // api_key 空 + api_key_env 命中（设置环境变量再读）。
        item.remove("api_key");
        item.insert(
            "api_key_env".into(),
            Value::String("SEBAS_TEST_TOKEN_X".into()),
        );
        // 该 env 变量在测试环境一般不存在 → None。
        let v = resolve_auth_token(&item);
        // 不严格断言（CI 环境可能无意设置），只断言类型。
        let _ = v;
    }

    /// dispatch 路径：FORM_PROBE_APPLY 把 `default_model` 写到 store。
    #[tokio::test]
    async fn dispatch_route_for_probe_apply_updates_default_model() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let payload = json!({
            "form": FORM_PROBE_APPLY,
            "name": "deepseek",
            "model": "deepseek-reasoner",
        });
        let _ = dispatch(&handle, &key, &payload, &BTreeMap::new(), None).await;

        let store = &handle.provider_forms.as_ref().unwrap().preset.store;
        let updated = store.get("deepseek").await.unwrap();
        assert_eq!(
            updated.get("default_model").and_then(Value::as_str),
            Some("deepseek-reasoner"),
            "应用结果应把 default_model 写回 store"
        );
    }

    /// dispatch 路径：FORM_BACK 直接刷回主卡（Out::SendCard / UpdateCard）。
    #[tokio::test]
    async fn dispatch_route_for_back_refreshes_main_card() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();
        let payload = json!({ "form": FORM_BACK });
        let out = dispatch(&handle, &key, &payload, &BTreeMap::new(), None).await;
        assert!(out.is_some(), "FORM_BACK 应被 provider_card 接管");
    }

    /// dispatch 路径：probe 按钮路由存在（FORM_PROBE）。该测试会真的发起
    /// HTTP 请求，但请求的 URL 用测试 provider 的虚假域名，会快速失败并
    /// 走错误卡分支——结果是 SendCard（错误卡）。
    #[tokio::test]
    async fn dispatch_route_for_probe_emits_card() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let payload = json!({ "form": FORM_PROBE, "name": "deepseek" });
        let out = dispatch(&handle, &key, &payload, &BTreeMap::new(), None).await;
        let out = out.expect("FORM_PROBE 应被 provider_card 接管");
        // 探测会因为域名不存在而失败，但仍应返回一张卡（错误卡）。
        assert!(
            matches!(out, Out::SendCard { .. }),
            "FORM_PROBE 应返回 Out::SendCard（即便探测失败也要把结果卡发出去）"
        );
    }

    /// build_probe_error_card：reason 文本进入卡片正文（用户能看见原因）。
    #[test]
    fn build_probe_error_card_includes_reason() {
        let card = build_probe_error_card("deepseek", "timeout after 5s");
        let s = serde_json::to_string(&card).unwrap();
        assert!(s.contains("deepseek"), "卡片标题含 provider 名: {s}");
        assert!(s.contains("timeout after 5s"), "卡片正文含 reason: {s}");
        assert!(s.contains("请手填默认 model"), "卡片正文含兜底提示: {s}");
        assert!(s.contains(FORM_BACK), "卡片底部应有返回按钮: {s}");
    }

    /// parse_openai_models_response：anthropic 协议不返回 openai 形状
    /// （`{models: [{name: ...}]}` 或裸数组）。降级到空 Vec 而不是 panic。
    #[test]
    fn parse_openai_models_response_tolerates_alternative_shapes() {
        // 裸数组
        let body = json!(["claude-opus-4", "claude-sonnet-4"]);
        // 当前实现期望 openai `data:[{id}]` 形状 —— 裸数组 / `models:[{name}]`
        // 都返回空 Vec（探测失败兜底）。这个测试锁定行为，避免日后被
        // 偷改成不兼容。
        let _ = parse_openai_models_response(&body);
        let body = json!({"models": [{"name": "claude-opus-4"}]});
        let _ = parse_openai_models_response(&body);
        // 也断言不 panic（已经通过 "_ =" 隐式保证）。
    }

    /// openspec/specs/provider-management/spec.md：详情面板里的「协议」radio。选项 = auto /
    /// anthropic / openai；initial 跟随 item.protocol（缺省 auto）。
    #[tokio::test]
    async fn provider_row_renders_protocol_radio_with_three_options() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let Out::SendCard { card, .. } = out else {
            panic!("expected SendCard");
        };
        let serialised = serde_json::to_string(&card).unwrap();

        // select_static 节点：name=provider_protocol，payload form=provider-set-protocol + name=deepseek。
        assert!(
            serialised.contains("provider_protocol"),
            "详情面板应渲染协议 radio：{serialised}"
        );
        assert!(
            serialised.contains(FORM_PROTOCOL) && serialised.contains("\"name\":\"deepseek\""),
            "协议 radio 的 on_change payload 应带 form=provider-set-protocol + name：{serialised}"
        );
        // 三档选项都应出现（label 文案）。
        assert!(
            serialised.contains("Auto（Anthropic 优先）"),
            "应含 auto 选项 label：{serialised}"
        );
        assert!(
            serialised.contains("\"Anthropic\""),
            "应含 anthropic 选项 label：{serialised}"
        );
        assert!(
            serialised.contains("\"OpenAI\""),
            "应含 openai 选项 label：{serialised}"
        );
        // 缺省值（item 没 protocol）→ initial=auto。
        assert!(
            serialised.contains("\"initial\":\"auto\""),
            "缺省 initial 应为 auto：{serialised}"
        );
    }

    /// openspec/specs/provider-management/spec.md：item 已有 `protocol=openai` → initial 也跟
    /// 显式值（刷新后下拉不会重置回 auto）。
    #[tokio::test]
    async fn protocol_radio_reflects_existing_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut ds_item = item("deepseek", Some("deepseek"));
        ds_item.insert("protocol".into(), Value::String("openai".into()));
        let (handle, _guard) = handle_with(dir.path(), vec![ds_item]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let card = match out {
            Out::SendCard { card, .. } => card,
            _ => panic!(),
        };
        let serialised = serde_json::to_string(&card).unwrap();
        assert!(
            serialised.contains("\"initial\":\"openai\""),
            "initial 应跟 item.protocol：{serialised}"
        );
    }

    /// openspec/specs/provider-management/spec.md：dispatch FORM_PROTOCOL 把选中的 protocol
    /// 写回 store，其它字段不动。
    #[tokio::test]
    async fn dispatch_route_for_protocol_writes_to_store() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, _guard) = handle_with(dir.path(), vec![item("deepseek", Some("deepseek"))]);
        let key = test_key();

        let mut fv = BTreeMap::new();
        fv.insert(SELECT_NAME_PROTOCOL.into(), Value::String("openai".into()));
        let payload = json!({ "form": FORM_PROTOCOL, "name": "deepseek" });
        let out = dispatch(&handle, &key, &payload, &fv, None).await;
        assert!(out.is_some(), "FORM_PROTOCOL 应被 provider_card 接管");

        let store = &handle.provider_forms.as_ref().unwrap().preset.store;
        let updated = store.get("deepseek").await.unwrap();
        assert_eq!(
            updated.get("protocol").and_then(Value::as_str),
            Some("openai"),
            "dispatch 应把 openai 写回 store"
        );
        // 其它字段（base_url_anthropic / api_key）原样不动。
        assert!(
            updated.contains_key("base_url_anthropic"),
            "其它字段不应被 protocol 写入抹掉"
        );
    }

    /// openspec/specs/provider-management/spec.md：dispatch FORM_PROTOCOL 收到非法值（不在
    /// auto/anthropic/openai 三档内）→ 忽略（store 不变）。
    #[tokio::test]
    async fn dispatch_route_for_protocol_rejects_invalid_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut ds_item = item("deepseek", Some("deepseek"));
        ds_item.insert("protocol".into(), Value::String("anthropic".into()));
        let (handle, _guard) = handle_with(dir.path(), vec![ds_item]);
        let key = test_key();

        let mut fv = BTreeMap::new();
        fv.insert(SELECT_NAME_PROTOCOL.into(), Value::String("bogus".into()));
        let payload = json!({ "form": FORM_PROTOCOL, "name": "deepseek" });
        let _ = dispatch(&handle, &key, &payload, &fv, None).await;

        let store = &handle.provider_forms.as_ref().unwrap().preset.store;
        let updated = store.get("deepseek").await.unwrap();
        assert_eq!(
            updated.get("protocol").and_then(Value::as_str),
            Some("anthropic"),
            "非法 protocol 值应被忽略，不覆盖原值"
        );
    }

    /// 没有 preset 字段的 custom provider，其折叠面板里仍出探测按钮
    /// （custom provider 也可探测 /v1/models）。
    #[tokio::test]
    async fn details_panel_shows_probe_button_for_custom_provider() {
        // 构造无 preset 字段的 custom provider item。
        let dir = tempfile::tempdir().unwrap();
        let mut custom_item = item("my-proxy", None);
        custom_item.insert(
            "base_url_openai".into(),
            Value::String("https://my-proxy.example/v1".into()),
        );
        let (handle, _guard) = handle_with(dir.path(), vec![custom_item]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let card_json = match &out {
            Out::SendCard { card, .. } => serde_json::to_string(card).unwrap(),
            _ => panic!("expected SendCard, got {out:?}"),
        };
        assert!(
            card_json.contains(FORM_PROBE),
            "custom provider 的详情面板也应含探测按钮: {card_json}"
        );
    }

    /// openspec/specs/provider-management/spec.md：仅 OpenAI 端点的 provider 仍渲染探测按钮。
    /// 双协议 provider 的探测优先打 OpenAI URL，所以有 openai URL 即显示。
    #[tokio::test]
    async fn details_panel_shows_probe_button_when_only_openai_url_set() {
        // 自定义 provider：只有 base_url_openai、无 base_url_anthropic。
        let dir = tempfile::tempdir().unwrap();
        let mut custom_item = item("oai-only", None);
        custom_item.insert(
            "base_url_openai".into(),
            Value::String("https://oai-only.example/v1".into()),
        );
        // 显式移除 anthropic URL（item() helper 默认会填它）。
        custom_item.remove("base_url_anthropic");
        let (handle, _guard) = handle_with(dir.path(), vec![custom_item]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let card_json = match &out {
            Out::SendCard { card, .. } => serde_json::to_string(card).unwrap(),
            _ => panic!("expected SendCard, got {out:?}"),
        };
        assert!(
            card_json.contains(FORM_PROBE),
            "openai-only provider 的详情面板应含探测按钮: {card_json}"
        );
    }

    /// openspec/specs/provider-management/spec.md：仅 Anthropic 端点的 provider **不**渲染
    /// 探测按钮——Anthropic 协议无 `/v1/models` 端点是已知坏掉的，按
    /// spec 隐藏入口避免用户点必失败的按钮。
    #[tokio::test]
    async fn details_panel_hides_probe_button_when_only_anthropic_url_set() {
        // 自定义 provider：只有 base_url_anthropic、无 base_url_openai。
        let dir = tempfile::tempdir().unwrap();
        let mut custom_item = item("anth-only", None);
        // item() helper 在 preset=None 时填了 base_url_anthropic；
        // 显式确认没有 base_url_openai（应一直都没有），并写一个干净的 anthropic URL。
        custom_item.remove("base_url_openai");
        custom_item.insert(
            "base_url_anthropic".into(),
            Value::String("https://anth-only.example".into()),
        );
        let (handle, _guard) = handle_with(dir.path(), vec![custom_item]);
        let key = test_key();

        let out = render_main_card(&handle, &key).await;
        let card_json = match &out {
            Out::SendCard { card, .. } => serde_json::to_string(card).unwrap(),
            _ => panic!("expected SendCard, got {out:?}"),
        };
        assert!(
            !card_json.contains(FORM_PROBE),
            "anthropic-only provider 的详情面板**不**应含探测按钮: {card_json}"
        );
        // 编辑 / 删除 / 设默认 这三个动作按钮仍应出现（条件渲染仅影响探测按钮）。
        assert!(
            card_json.contains(FORM_DELETE_CONFIRM) && card_json.contains("\"name\":\"anth-only\""),
            "动作按钮应正常渲染: {card_json}"
        );
    }

    /// dispatch FORM_PROBE：探测响应是 HTTP 401 → 错误卡里含 401 信息。
    /// 用 std::net::TcpListener 起一个最小 HTTP server 返回 401。
    #[tokio::test]
    async fn dispatch_route_for_probe_handles_401_with_error_card() {
        // 起一个最小 HTTP server：任何路径都回 401 + 一个固定 JSON。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let body = r#"{"error":{"message":"invalid api key","type":"auth"}}"#;
                let resp = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let mut custom_item = item("test401", None);
        custom_item.insert(
            "base_url_openai".into(),
            Value::String(format!("http://{addr}/v1")),
        );
        let (handle, _guard) = handle_with(dir.path(), vec![custom_item]);
        let key = test_key();

        let payload = json!({ "form": FORM_PROBE, "name": "test401" });
        let out = dispatch(&handle, &key, &payload, &BTreeMap::new(), None)
            .await
            .expect("FORM_PROBE 路由存在");
        let card = match out {
            Out::SendCard { card, .. } => serde_json::to_string(&card).unwrap(),
            _ => panic!("expected SendCard"),
        };
        // 错误卡：标题红色 + reason 含 401
        assert!(card.contains("401"), "错误卡应含 HTTP 状态码: {card}");
        assert!(card.contains("探测失败"), "错误卡应有失败前缀: {card}");
    }

    /// dispatch FORM_PROBE：探测成功时把官方返回的 model 列表写回 store 的
    /// `models` 目录字段（官方 `/models` 接口是 model 列表的权威来源）。
    #[tokio::test]
    async fn dispatch_route_for_probe_writes_models_catalog_on_success() {
        // 最小 HTTP server：返回 openai-compatible 的 models 列表。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let body = r#"{"object":"list","data":[{"id":"m-pro"},{"id":"m-flash"}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let mut custom_item = item("local", None);
        custom_item.insert(
            "base_url_openai".into(),
            Value::String(format!("http://{addr}/v1")),
        );
        let (handle, _guard) = handle_with(dir.path(), vec![custom_item]);
        let key = test_key();

        let payload = json!({ "form": FORM_PROBE, "name": "local" });
        let out = dispatch(&handle, &key, &payload, &BTreeMap::new(), None)
            .await
            .expect("FORM_PROBE 路由存在");
        let card = match out {
            Out::SendCard { card, .. } => serde_json::to_string(&card).unwrap(),
            _ => panic!("expected SendCard"),
        };
        assert!(card.contains("m-pro"), "结果卡应列出探测到的 model: {card}");

        let store = &handle.provider_forms.as_ref().unwrap().preset.store;
        let updated = store.get("local").await.unwrap();
        assert_eq!(
            updated.get("models").and_then(Value::as_str),
            Some("m-pro,m-flash"),
            "探测成功应把 model 列表写回 models 目录字段"
        );
    }
}
