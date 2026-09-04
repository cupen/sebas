//! Split state store: runtime 状态与 provider 数据。
//!
//! **优先走 SQLite 状态库**（`init_engine` 后）：`load/save/update` 全部委托
//! 给 `StateStoreEngine`（core 进程内的 `DbStateEngine`，add-state-store）。
//! 文件路径（`~/.sebas/state.json` + `providers.json`）仅在后端不可用
//! （engine 未初始化，如 DB 初始化失败/测试夹具）时作为降级回退。
//!
//! - `~/.sebas/state.json`（`SEBAS_STATE_FILE` 可覆盖）：runtime 决策
//!   （`version` + `mode` + `default_selection`），spawn 翻译的输入。
//! - `~/.sebas/providers.json`（`SEBAS_GATEWAY_PROVIDER_OVERLAY` 可覆盖）：
//!   provider 数据（`providers` CRUD delta + `deleted` 墓碑 + `model_aliases`）。
//!
//! ## 演进史
//!
//! - 最初：providers.json 只放 provider CRUD delta。
//! - openspec/specs/provider-management/spec.md：合并进 state.json v2 单文件，providers.json 被
//!   迁移删除 —— 但 gateway 一直读 providers.json，卡片编辑到不了 gateway
//!   （断链）。
//! - gateway-admin-api-and-model-aliases：拆回。providers.json 成为飞书
//!   `/provider` 卡片与 gateway admin API 双写者共用的单一真源；state.json
//!   只留 runtime 段。
//! - add-state-store：SQLite 成为唯一写路径（core 进程）；providers.json /
//!   state.json 保留在磁盘但不再被引擎路径读取，仅作降级回退。
//!
//! ## state.json wire（runtime）
//!
//! ```json
//! {
//!   "version": 2,
//!   "mode": { "kind": "direct", "provider": "deepseek" },
//!   "default_selection": { "provider": "deepseek", "model": "deepseek-chat" }
//! }
//! ```
//!
//! ## providers.json wire
//!
//! ```json
//! {
//!   "providers": { "deepseek": { "preset": "deepseek", ... } },
//!   "deleted":  ["openai"],
//!   "model_aliases": { ... }   // gateway 拥有，本模块透传保留
//! }
//! ```
//!
//! ## 版本感知迁移（load 时一次性）
//!
//! | state.json | providers.json | 行为 |
//! |---|---|---|
//! | 不存在 | 不存在 | 全部 default |
//! | 不存在 | 存在 | providers 来自 overlay；materialize runtime v2 state.json；**保留** providers.json |
//! | v1/v0（无 version 或 version=1）| 任意 | runtime 来自 state.json（mode+default）；providers 来自 overlay；写 runtime v2；保留 overlay |
//! | v2（含 providers/deleted 段，2026-08-17 时代的单文件 schema）| 任意 | **反向搬出**：stranded providers/deleted 合并进 providers.json（state 侧优先，与旧合并语义一致）→ 重写 state.json 只留 runtime 段。幂等可重入：崩溃在「overlay 已写、state 未清」之间时下次 load 重做合并，不丢数据 |
//! | v2（纯 runtime 段，新 schema）| 任意 | 直接用 |
//!
//! 错误语义：单文件解析失败只丢那半边（state 坏 → runtime default；overlay
//! 坏 → providers default），另一侧照常 —— runtime 状态不应让 sebas 启动
//! 失败，provider 数据破损的自愈备份由 `sebas::provider::build_form` 的
//! broken-overlay self-heal 负责。

use crate::provider_state::ProviderMode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 状态存储引擎 trait: 抽象 SQLite/文件后端。
#[async_trait::async_trait]
pub trait StateStoreEngine: Send + Sync {
    async fn load_persisted_state(&self) -> PersistedState;
    async fn save_persisted_state(&self, state: PersistedState) -> anyhow::Result<()>;
    async fn load_settings(&self) -> Result<Option<serde_json::Value>, String>;
    async fn save_settings(&self, cfg: serde_json::Value) -> Result<(), String>;
    /// Load projects as JSON Value (array of project rows).
    async fn load_projects(&self) -> Result<Vec<serde_json::Value>, String>;
    /// Save projects (replace all).
    async fn save_projects(&self, projects: Vec<serde_json::Value>) -> Result<(), String>;
    /// Add a project entry.
    async fn add_project(&self, path: &str, name: &str, added_at: i64) -> Result<(), String>;
    /// Remove a project by path.
    async fn remove_project(&self, path: &str) -> Result<bool, String>;
}

/// 全局状态存储引擎 (add-state-store)。
static ENGINE: OnceLock<Box<dyn StateStoreEngine + Send + Sync>> = OnceLock::new();
/// 状态变更通知广播 (add-state-store 4.2): 写者提交成功后按 scope 投递,
/// 订阅者据此重投影。合并语义: 一串提交可以合并为一个通知(由订阅端
/// debounce / 服务端合并窗口决定, 本通道只保证"提交后至少一帧")。
static CHANGE_TX: OnceLock<tokio::sync::broadcast::Sender<StateChange>> = OnceLock::new();

/// 状态变更通知 (design D6): 单一事件流 + scope 标签。
/// 提交后投递, 允许合并(一串提交一个通知)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum StateChange {
    /// 某域发生变更。`scope` ∈ providers | aliases | settings | projects |
    /// sessions。
    Changed { scope: String },
    /// 全部域重置(引擎重建/全量同步)。
    Reset,
}

/// 初始化全局状态存储引擎 + 变更通知广播。
pub fn init_engine(engine: Box<dyn StateStoreEngine + Send + Sync>) {
    let (tx, _) = tokio::sync::broadcast::channel(64);
    CHANGE_TX
        .set(tx)
        .ok()
        .expect("state change broadcast 已初始化");
    ENGINE
        .set(engine)
        .ok()
        .expect("state store engine 已初始化");
}

/// 提交成功后按 scope 发一条变更通知。广播无人订阅时是 no-op。
pub fn notify_change(scope: &str) {
    if let Some(tx) = CHANGE_TX.get() {
        let _ = tx.send(StateChange::Changed {
            scope: scope.to_string(),
        });
    }
}

/// 订阅状态变更通知。
pub fn subscribe_changes() -> Option<tokio::sync::broadcast::Receiver<StateChange>> {
    CHANGE_TX.get().map(|tx| tx.subscribe())
}

/// 获取引擎引用。
pub fn engine() -> Option<&'static Box<dyn StateStoreEngine + Send + Sync>> {
    ENGINE.get()
}

/// 一条记录：字段名 -> 值。Provider CRUD 用。
pub type Item = Map<String, Value>;

/// 目标 schema 版本号。`PersistedState::default()` 和 `save()` 都写这个版本。
pub const STATE_VERSION_V2: u32 = 2;
/// 旧版 schema：没有 `version` 字段或 version=1 — 只含 mode + default_provider_for_direct。
pub const STATE_VERSION_V1: u32 = 1;

/// Runtime 「DIRECT 默认」选择（openspec/specs/provider-management/spec.md）。
///
/// 把旧 `default_provider_for_direct: Option<String>` 和 overlay item 上的
/// `default_model` 合并到一个 `(provider, model)` 元组：
/// - `provider`：DIRECT 模式下默认启用的 provider 名（必须存在于 `providers`
///   或 `gateway_cfg`，否则 spawn-time 兜底回退 Off + warn）；
/// - `model`：spawn 时追加的 `--model <id>`（仅在 Direct 模式下生效；Gateway
///   模式由 gateway 自己路由）。
///
/// Overlay item 上的 `default_model` 仍是 UI 源（`/provider` 详情面板的「默认
/// model」文本框），但 spawn 时只信 `default_selection.model`。"set as default"
/// 动作负责把 overlay 的 `default_model` 同步进 `default_selection.model`（见
/// `sebas_router::router::provider_card::handle_set_default_direct` 的 merge helper）。
///
/// wire shape：
/// ```json
/// "default_selection": { "provider": "deepseek", "model": "deepseek-chat" }
/// ```
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

/// 一条模型别名（与 gateway admin API 的 wire 同形状）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelAliasEntry {
    pub provider: String,
    /// 缺省 = 别名即 upstream model（透传）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_model: Option<String>,
}

/// 内存聚合视图：runtime（state.json）+ provider 数据（providers.json）。
///
/// 仅作为 load() 的返回值与 update() 闭包的操作对象；`save()` 会把它**拆开**
/// 写回两个文件（providers/deleted → providers.json；其余 → state.json）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersistedState {
    #[serde(default = "default_state_version")]
    pub version: u32,
    #[serde(default)]
    pub providers: BTreeMap<String, Item>,
    #[serde(default)]
    pub deleted: Vec<String>,
    #[serde(default)]
    pub mode: ProviderMode,
    /// openspec/specs/provider-management/spec.md：DIRECT 模式默认 (provider, model)。serde 别名接受旧字段
    /// `default_provider_for_direct` —— 旧 state.json 解析到这里时
    /// `model=None`，upgrade step 在 `repair_mode` 后落地为新 wire 形状。
    #[serde(default, alias = "default_provider_for_direct")]
    pub default_selection: Option<DefaultSelection>,
    /// 模型别名（add-state-store 5.3）：gateway admin API 拥有。随状态库
    /// 流转——DB 侧存 model_aliases 表，文件侧透传 providers.json 的
    /// `model_aliases` 段。
    #[serde(default)]
    pub model_aliases: BTreeMap<String, ModelAliasEntry>,
}

fn default_state_version() -> u32 {
    STATE_VERSION_V2
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION_V2,
            providers: BTreeMap::new(),
            deleted: Vec::new(),
            mode: ProviderMode::default(),
            default_selection: None,
            model_aliases: BTreeMap::new(),
        }
    }
}

/// State file 路径：`~/.sebas/state.json`，可用 `SEBAS_STATE_FILE` 覆盖
/// （与 `provider_state.rs` 同惯例）。
pub fn state_path() -> PathBuf {
    let raw = std::env::var("SEBAS_STATE_FILE").unwrap_or_else(|_| "~/.sebas/state.json".into());
    PathBuf::from(expand_tilde(&raw))
}

/// provider overlay 文件路径：`~/.sebas/providers.json`，可用
/// `SEBAS_GATEWAY_PROVIDER_OVERLAY` 覆盖（与 `src::provider::overlay_path`
/// 同惯例）。provider 数据的单一真源（卡片 + gateway admin API 双写者共用）。
pub fn providers_path() -> PathBuf {
    let raw = std::env::var("SEBAS_GATEWAY_PROVIDER_OVERLAY")
        .unwrap_or_else(|_| "~/.sebas/providers.json".into());
    PathBuf::from(expand_tilde(&raw))
}

/// 复制 `sebas::config::expand_tilde`（router 不能反向依赖 sebas root）。
pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into();
    }
    p.to_string()
}

/// 在同步代码里等待 engine 的 future。调用点可能位于 tokio worker 线程的
/// async 上下文中（启动期 provider 表单构建、HTTP handler 的同步桥等），
/// 直接 `Handle::block_on` 会 panic（"Cannot start a runtime from within a
/// runtime"），必须先 `block_in_place` 让出当前 worker。ENGINE 只在
/// `sebas run` 的多线程运行时里初始化，不受 block_in_place 的
/// current_thread 限制影响。
fn block_on_engine<F: std::future::Future>(fut: F) -> F::Output {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(fut))
}

/// 版本感知加载 + 反向迁移：见模块文档的版本矩阵。
///
/// 错误一侧只丢一侧（state 坏 → runtime default；overlay 坏 → providers
/// default），另一侧照常。
///
/// 当 `init_engine` 已调用时, 委托给引擎。
pub fn load() -> PersistedState {
    if let Some(engine) = ENGINE.get() {
        return block_on_engine(engine.load_persisted_state());
    }
    load_at(&state_path(), &providers_path())
}

/// 原子写：把聚合 state 拆开写两个文件（各自 tmp + rename）。
/// providers.json 侧走 Map 级 RMW：只覆写 `providers`/`deleted` 两个 key，
/// 文件内其它段（如 gateway 的 `model_aliases`）原样保留。
///
/// 当 `init_engine` 已调用时, 委托给引擎。
pub fn save(s: &PersistedState) -> anyhow::Result<()> {
    if let Some(engine) = ENGINE.get() {
        let state = s.clone();
        return block_on_engine(engine.save_persisted_state(state))
            .map_err(|e| anyhow::anyhow!("{}", e));
    }
    save_at(&state_path(), &providers_path(), s)
}

/// 读 → 改 → 写一气呵成。`f` 闭包基于当前 state 做条件决策；返回改后的 state。
///
/// 当 `init_engine` 已调用时, 委托给引擎。
pub fn update<F>(f: F) -> anyhow::Result<PersistedState>
where
    F: FnOnce(&mut PersistedState),
{
    if let Some(engine) = ENGINE.get() {
        let mut state = block_on_engine(engine.load_persisted_state());
        f(&mut state);
        let snapshot = state.clone();
        block_on_engine(engine.save_persisted_state(state))
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        return Ok(snapshot);
    }
    let mut s = load();
    f(&mut s);
    save(&s)?;
    Ok(s)
}

/// "delete default provider" 原子操作：删除 provider + 同步清掉
/// `default_selection`（若指向被删的）+ 写盘。
///
/// mode 的清理留给 load() 的 `repair_mode`：如果 mode 还指向刚被删的
/// provider，下次 load 会重置为 Off（repair 时机 = 读时，比写时更安全——
/// 写时拿不到 providers + deleted 的最新视图）。
///
/// 返回改后的 state（无论闭包是否真改了东西）。
pub fn delete_provider_and_clear_default(id: &str) -> anyhow::Result<PersistedState> {
    update(|s| {
        s.providers.remove(id);
        if !s.deleted.iter().any(|d| d == id) {
            s.deleted.push(id.to_string());
        }
        if s.default_selection.as_ref().map(|d| d.provider.as_str()) == Some(id) {
            s.default_selection = None;
        }
    })
}

// ---- 内部实现（路径参数化，测试不走 env var 可并行） ----

/// state.json 侧的加载结果。
struct RuntimeSide {
    state_exists: bool,
    mode: ProviderMode,
    default_selection: Option<DefaultSelection>,
    /// 2026-08-17 单文件 schema 时代滞留在 state.json 里的 provider 数据
    /// （反向迁移源）。空 = 无需搬出。
    stranded_providers: BTreeMap<String, Item>,
    stranded_deleted: Vec<String>,
    /// 是否需要重写 state.json（legacy 物化 v2 / stranded 搬出后清理）。
    needs_rewrite: bool,
}

fn load_at(state_p: &Path, overlay_p: &Path) -> PersistedState {
    let runtime = load_runtime_side(state_p);
    let (mut providers, mut deleted, model_aliases, _overlay_raw_ok) =
        load_overlay_sections(overlay_p);

    // 反向迁移：stranded providers/deleted 合并进 providers.json（state 侧
    // 优先 —— 旧单文件时代的 load 已把 overlay 合并进 state，state 是超集），
    // 然后 state.json 只留 runtime 段。崩溃重入安全：overlay 已写但 state
    // 未清时，下次 load 重做同一合并（幂等，值相同）。
    if !runtime.stranded_providers.is_empty() || !runtime.stranded_deleted.is_empty() {
        for (k, v) in runtime.stranded_providers {
            providers.insert(k, v);
        }
        for d in runtime.stranded_deleted {
            if !deleted.contains(&d) {
                deleted.push(d);
            }
        }
        if save_overlay(overlay_p, &providers, &deleted, Some(&model_aliases)).is_ok() {
            let clean = RuntimeWire {
                version: STATE_VERSION_V2,
                mode: runtime.mode.clone(),
                default_selection: runtime.default_selection.clone(),
            };
            let _ = write_runtime(state_p, &clean);
        }
        return repair_mode(PersistedState {
            version: STATE_VERSION_V2,
            providers,
            deleted,
            mode: runtime.mode,
            default_selection: runtime.default_selection,
            model_aliases,
        });
    }

    // legacy v1/v0 state.json（或 state.json 不存在但 overlay 在）：materialize
    // runtime v2 state.json（保持旧「首次 load 落盘 v2」行为）；providers.json
    // 保留不动。
    let overlay_present = providers_raw_present(overlay_p);
    if runtime.needs_rewrite || (overlay_present && !runtime.state_exists) {
        let wire = RuntimeWire {
            version: STATE_VERSION_V2,
            mode: runtime.mode.clone(),
            default_selection: runtime.default_selection.clone(),
        };
        let _ = write_runtime(state_p, &wire);
    }
    repair_mode(PersistedState {
        version: STATE_VERSION_V2,
        providers,
        deleted,
        mode: runtime.mode,
        default_selection: runtime.default_selection,
        model_aliases,
    })
}

/// 读 state.json → runtime 段。解析失败 / 不存在 → runtime default
/// （不物化）。v1/v0 → needs_rewrite（首次 load materialize v2）。
/// v2 带 providers/deleted → stranded（反向迁移源）+ needs_rewrite。
fn load_runtime_side(state_p: &Path) -> RuntimeSide {
    let raw = match std::fs::read_to_string(state_p) {
        Ok(r) => r,
        Err(_) => {
            // state.json 不存在：runtime default。是否物化 v2 state.json
            // 由 load_at 决定（overlay 也在时才物化，保持旧 Path B 行为）。
            return RuntimeSide {
                state_exists: false,
                mode: ProviderMode::default(),
                default_selection: None,
                stranded_providers: BTreeMap::new(),
                stranded_deleted: Vec::new(),
                needs_rewrite: false,
            };
        }
    };
    match parse_version(&raw) {
        Some(STATE_VERSION_V2) => {
            // v2：可能是新 schema（纯 runtime）或旧单文件 schema（带
            // providers/deleted）。PersistedState 的 serde 容忍两者。
            match serde_json::from_str::<PersistedState>(&raw) {
                Ok(s) => {
                    let stranded = !s.providers.is_empty() || !s.deleted.is_empty();
                    RuntimeSide {
                        state_exists: true,
                        mode: s.mode,
                        default_selection: s.default_selection,
                        stranded_providers: s.providers,
                        stranded_deleted: s.deleted,
                        needs_rewrite: stranded,
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        path = %state_p.display(),
                        error = %e,
                        "state.json v2 解析失败，runtime 回退默认"
                    );
                    RuntimeSide {
                        state_exists: true,
                        mode: ProviderMode::default(),
                        default_selection: None,
                        stranded_providers: BTreeMap::new(),
                        stranded_deleted: Vec::new(),
                        needs_rewrite: false,
                    }
                }
            }
        }
        Some(STATE_VERSION_V1) | None => {
            // legacy v0/v1：state.json 只含 mode + default。
            let legacy = parse_legacy_state(&raw).unwrap_or_default();
            RuntimeSide {
                state_exists: true,
                mode: legacy.mode,
                default_selection: legacy.default_provider_for_direct.map(DefaultSelection::new),
                stranded_providers: BTreeMap::new(),
                stranded_deleted: Vec::new(),
                needs_rewrite: true,
            }
        }
        Some(other) => {
            tracing::warn!(
                path = %state_p.display(),
                version = other,
                "未知的 state.json version，runtime 回退默认"
            );
            RuntimeSide {
                state_exists: true,
                mode: ProviderMode::default(),
                default_selection: None,
                stranded_providers: BTreeMap::new(),
                stranded_deleted: Vec::new(),
                needs_rewrite: false,
            }
        }
    }
}

/// overlay 文件是否物理存在（内容有效性无关）——决定 state.json 缺失时
/// 是否 materialize v2 runtime（保持旧 Path B「overlay-only 机器首次 load
/// 落盘 state.json」行为）。
fn providers_raw_present(overlay_p: &Path) -> bool {
    overlay_p.exists()
}

/// 读 providers.json → (providers, deleted, model_aliases, raw_ok)。
/// 解析失败 / 不存在 → (空, 空, 空, false)。破损文件的备份自愈由上层
/// （provider.rs build_form）负责。
fn load_overlay_sections(
    overlay_p: &Path,
) -> (
    BTreeMap<String, Item>,
    Vec<String>,
    BTreeMap<String, ModelAliasEntry>,
    bool,
) {
    let raw = match std::fs::read_to_string(overlay_p) {
        Ok(r) => r,
        Err(_) => return (BTreeMap::new(), Vec::new(), BTreeMap::new(), false),
    };
    match serde_json::from_str::<OverlayWire>(&raw) {
        Ok(ov) => (ov.providers, ov.deleted, ov.model_aliases, true),
        Err(e) => {
            tracing::warn!(
                path = %overlay_p.display(),
                error = %e,
                "providers.json 解析失败，providers 回退默认"
            );
            (BTreeMap::new(), Vec::new(), BTreeMap::new(), false)
        }
    }
}

/// state.json 的 runtime wire（新 schema：无 providers/deleted 字段）。
#[derive(Serialize, Deserialize)]
struct RuntimeWire {
    #[serde(default = "default_state_version")]
    version: u32,
    #[serde(default)]
    mode: ProviderMode,
    #[serde(default, alias = "default_provider_for_direct")]
    default_selection: Option<DefaultSelection>,
}

/// providers.json 的 wire（本模块解释 providers/deleted/model_aliases；
/// 其它未知段在 save_overlay 的 RMW 里保留）。
#[derive(Default, Deserialize)]
struct OverlayWire {
    #[serde(default)]
    providers: BTreeMap<String, Item>,
    #[serde(default)]
    deleted: Vec<String>,
    #[serde(default)]
    model_aliases: BTreeMap<String, ModelAliasEntry>,
}

/// 旧版 state.json 的 wire 形状：只 mode + default_provider_for_direct。
#[derive(Default, Deserialize)]
struct LegacyState {
    #[serde(default)]
    mode: ProviderMode,
    #[serde(default)]
    default_provider_for_direct: Option<String>,
}

fn parse_legacy_state(raw: &str) -> Option<LegacyState> {
    serde_json::from_str(raw).ok()
}

/// 从 JSON 字符串里抽顶层 `version` 字段（数字）。无字段 / 非数字 → `None`。
fn parse_version(raw: &str) -> Option<u32> {
    let v: Value = serde_json::from_str(raw).ok()?;
    v.get("version").and_then(Value::as_u64).map(|n| n as u32)
}

/// 写 state.json（tmp + rename）。父目录缺失则创建。
fn write_runtime(path: &Path, wire: &RuntimeWire) -> anyhow::Result<()> {
    write_json_atomic(path, &serde_json::to_value(wire)?)
}

/// 写 providers.json：Map 级 RMW —— 读现有文件为 raw Map（保留
/// `model_aliases` 等未知段），只覆写 `providers`/`deleted` 两个 key。
/// 现有文件缺失 / 解析失败 → 从空 Map 开始（破损文件的备份自愈在上层）。
fn save_overlay(
    path: &Path,
    providers: &BTreeMap<String, Item>,
    deleted: &[String],
    model_aliases: Option<&BTreeMap<String, ModelAliasEntry>>,
) -> anyhow::Result<()> {
    let mut root: Map<String, Value> = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    root.insert(
        "providers".to_string(),
        serde_json::to_value(providers)?,
    );
    root.insert("deleted".to_string(), serde_json::to_value(deleted)?);
    if let Some(aliases) = model_aliases {
        root.insert(
            "model_aliases".to_string(),
            serde_json::to_value(aliases)?,
        );
    }
    write_json_atomic(path, &Value::Object(root))
}

/// tmp + rename 原子写 + fsync。父目录缺失则创建。
fn write_json_atomic(path: &Path, value: &Value) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("创建 {} 父目录失败: {e}", path.display()))?;
    }
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| anyhow::anyhow!("序列化 {} 失败: {e}", path.display()))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, body)
        .map_err(|e| anyhow::anyhow!("写入临时文件 {} 失败: {e}", tmp.display()))?;
    if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&tmp) {
        let _ = file.sync_all();
    }
    std::fs::rename(&tmp, path)
        .map_err(|e| anyhow::anyhow!("rename {} -> {} 失败: {e}", tmp.display(), path.display()))?;
    Ok(())
}

fn save_at(state_p: &Path, overlay_p: &Path, s: &PersistedState) -> anyhow::Result<()> {
    // providers.json 先写（真源），state.json 后写 —— 崩溃在两写之间时，
    // providers 数据已落盘，runtime 段最多回退 default（可自愈）。
    save_overlay(overlay_p, &s.providers, &s.deleted, Some(&s.model_aliases))?;
    write_runtime(
        state_p,
        &RuntimeWire {
            version: STATE_VERSION_V2,
            mode: s.mode.clone(),
            default_selection: s.default_selection.clone(),
        },
    )
}

/// repair-on-load：若 mode 指向 `deleted` 墓碑里的 provider，重置为 Off。
///
/// **只修 tombstone，不修「not in providers」** —— 后者是合法状态（用户
/// 切到 Direct 模式但还没配任何 provider 时常见），不能误伤。这是「删除
/// default provider」操作的兜底 —— 即便两次写中间崩了，下次 load 也能
/// 自愈，不会让 `Direct{ deleted_provider }` 卡在那里。
///
/// "missing provider" 的判断留给 spawn-time `compute_provider_resolution`
/// 兜底（找不到就回退 Off + warn），不放在持久化层。
fn repair_mode(mut s: PersistedState) -> PersistedState {
    let tombstoned_provider = match &s.mode {
        ProviderMode::Direct { provider } => {
            if s.deleted.iter().any(|d| d == provider) {
                Some(provider.clone())
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(provider) = tombstoned_provider {
        tracing::info!(
            provider = %provider,
            "mode 指向已 tombstoned 的 provider，重置为 Off（repair-on-load）"
        );
        s.mode = ProviderMode::Off;
        if s.default_selection.as_ref().map(|d| d.provider.as_str()) == Some(provider.as_str()) {
            s.default_selection = None;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    fn item_with(fields: &[(&str, &str)]) -> Item {
        let mut m = Map::new();
        for (k, v) in fields {
            m.insert((*k).into(), Value::String((*v).into()));
        }
        m
    }

    /// 路径 C：两个文件都不存在 → 返回 default，且不落任何文件。
    #[test]
    fn load_returns_default_when_neither_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        let s = load_at(&state_p, &prov_p);
        assert_eq!(s, PersistedState::default());
        assert!(!state_p.exists());
        assert!(!prov_p.exists());
    }

    /// 老机器（只有 providers.json）：providers 保留、文件**不删**；
    /// materialize runtime v2 state.json。
    #[test]
    fn overlay_only_machine_keeps_overlay_and_materializes_state() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &prov_p,
            r#"{
                "providers": { "deepseek": { "name": "deepseek", "preset": "deepseek" } },
                "deleted": ["openai"]
            }"#,
        );

        let s = load_at(&state_p, &prov_p);
        assert!(s.providers.contains_key("deepseek"));
        assert!(s.deleted.contains(&"openai".to_string()));
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.version, STATE_VERSION_V2);
        // providers.json 保留（不再是 legacy 一次性迁移源）。
        assert!(prov_p.exists(), "providers.json 必须保留（provider 真源）");
        // state.json materialize 为 v2 runtime。
        assert!(state_p.exists());
        let raw = std::fs::read_to_string(&state_p).unwrap();
        assert!(!raw.contains("\"providers\""), "state.json 不应含 providers 段");

        // 二次 load 稳定。
        let s2 = load_at(&state_p, &prov_p);
        assert_eq!(s2, s);
    }

    /// v1 state.json + providers.json：runtime 来自 state，providers 来自
    /// overlay，两文件各归各位（overlay 不删）。
    #[test]
    fn migration_from_v1_state_with_overlay() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &state_p,
            r#"{
                "mode": { "kind": "direct", "provider": "deepseek" },
                "default_provider_for_direct": "deepseek"
            }"#,
        );
        write_file(
            &prov_p,
            r#"{
                "providers": { "deepseek": { "name": "deepseek", "preset": "deepseek" } },
                "deleted": []
            }"#,
        );

        let s = load_at(&state_p, &prov_p);
        assert_eq!(
            s.mode,
            ProviderMode::Direct {
                provider: "deepseek".into()
            }
        );
        assert_eq!(
            s.default_selection.as_ref().map(|d| d.provider.as_str()),
            Some("deepseek")
        );
        assert!(s.providers.contains_key("deepseek"));
        assert!(prov_p.exists(), "overlay 不删");
        // state.json 升级为 v2 runtime-only。
        let raw = std::fs::read_to_string(&state_p).unwrap();
        assert!(raw.contains("\"version\": 2"));
        assert!(!raw.contains("\"providers\""));
        assert!(!raw.contains("default_provider_for_direct"));
    }

    /// v1 state.json 无 overlay：runtime 保留，providers 空。
    #[test]
    fn migration_from_v1_state_without_overlay() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &state_p,
            r#"{ "mode": { "kind": "gateway" }, "default_provider_for_direct": null }"#,
        );

        let s = load_at(&state_p, &prov_p);
        assert_eq!(s.version, STATE_VERSION_V2);
        assert_eq!(s.mode, ProviderMode::Gateway);
        assert!(s.providers.is_empty());
        assert!(s.deleted.is_empty());
    }

    /// **反向迁移（核心场景）**：已迁移机器 state.json 里滞留 providers 段
    /// → 搬出到 providers.json，数据完整，state.json 不再含 providers 段。
    #[test]
    fn stranded_providers_migrate_out_of_state_json() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        // 2026-08-17 单文件 schema：providers/deleted 在 state.json 里。
        write_file(
            &state_p,
            r#"{
                "version": 2,
                "providers": { "deepseek": { "name": "deepseek", "preset": "deepseek" } },
                "deleted": ["openai"],
                "mode": { "kind": "direct", "provider": "deepseek" },
                "default_selection": { "provider": "deepseek", "model": "deepseek-chat" }
            }"#,
        );

        let s = load_at(&state_p, &prov_p);
        // 数据完整。
        assert!(s.providers.contains_key("deepseek"));
        assert!(s.deleted.contains(&"openai".to_string()));
        assert_eq!(
            s.default_selection,
            Some(DefaultSelection::with_model("deepseek", "deepseek-chat"))
        );
        // providers.json 已创建并承载 provider 数据。
        assert!(prov_p.exists());
        let prov_raw = std::fs::read_to_string(&prov_p).unwrap();
        let prov: Value = serde_json::from_str(&prov_raw).unwrap();
        assert!(prov["providers"]["deepseek"].is_object());
        assert_eq!(prov["deleted"][0], "openai");
        // state.json 只剩 runtime 段。
        let state_raw = std::fs::read_to_string(&state_p).unwrap();
        assert!(!state_raw.contains("\"providers\""), "state.json 应已清空 providers 段");
        assert!(!state_raw.contains("\"deleted\""));

        // 二次 load 幂等。
        let s2 = load_at(&state_p, &prov_p);
        assert_eq!(s2, s);
    }

    /// **崩溃重入**：反向搬出「overlay 已写、state 未清」中途崩溃 →
    /// 重入合并不丢数据（state 侧优先，值相同幂等）。
    #[test]
    fn stranded_migration_crash_reentry_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        // 模拟半途状态：providers.json 只搬出了部分（other），state.json
        // 仍滞留全部（deepseek + other）。
        write_file(
            &prov_p,
            r#"{ "providers": { "other": { "name": "other" } }, "deleted": [] }"#,
        );
        write_file(
            &state_p,
            r#"{
                "version": 2,
                "providers": {
                    "deepseek": { "name": "deepseek" },
                    "other": { "name": "other" }
                },
                "deleted": ["openai"],
                "mode": { "kind": "off" }
            }"#,
        );

        let s = load_at(&state_p, &prov_p);
        // 合并结果：两个 provider 都在，墓碑保留。
        assert!(s.providers.contains_key("deepseek"));
        assert!(s.providers.contains_key("other"));
        assert!(s.deleted.contains(&"openai".to_string()));
        // state.json 清干净。
        let state_raw = std::fs::read_to_string(&state_p).unwrap();
        assert!(!state_raw.contains("\"providers\""));
        // 三次 load 稳定。
        let s2 = load_at(&state_p, &prov_p);
        assert_eq!(s2, s);
    }

    /// repair-on-load：mode 指向 deleted provider → 自动重置为 Off + 清 default。
    #[test]
    fn repair_mode_clears_stale_direct_pointer() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &prov_p,
            r#"{ "providers": { "deepseek": { "name": "deepseek" } }, "deleted": ["openai"] }"#,
        );
        write_file(
            &state_p,
            r#"{ "version": 2, "mode": { "kind": "direct", "provider": "openai" }, "default_selection": "openai" }"#,
        );
        let s = load_at(&state_p, &prov_p);
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.default_selection, None);
    }

    /// repair-on-load 不动「mode 指向不在 providers 里」的情况。
    #[test]
    fn repair_mode_keeps_pointer_to_missing_provider_when_no_tombstone() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &state_p,
            r#"{ "version": 2, "mode": { "kind": "direct", "provider": "ghost" } }"#,
        );
        let s = load_at(&state_p, &prov_p);
        assert_eq!(
            s.mode,
            ProviderMode::Direct {
                provider: "ghost".into()
            }
        );
    }

    /// save 拆双文件 round-trip：providers/deleted → providers.json；
    /// runtime → state.json；聚合 load 全部还原。
    #[test]
    fn save_splits_and_round_trips_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        let original = PersistedState {
            version: STATE_VERSION_V2,
            providers: BTreeMap::from([(
                "deepseek".to_string(),
                item_with(&[("name", "deepseek"), ("preset", "deepseek")]),
            )]),
            deleted: vec!["openai".to_string()],
            mode: ProviderMode::Direct {
                provider: "deepseek".into(),
            },
            default_selection: Some(DefaultSelection::with_model("deepseek", "deepseek-chat")),
            model_aliases: BTreeMap::new(),
        };
        save_at(&state_p, &prov_p, &original).unwrap();

        // providers.json 承载 provider 数据。
        let prov_raw = std::fs::read_to_string(&prov_p).unwrap();
        assert!(prov_raw.contains("deepseek"));
        // state.json 只承载 runtime。
        let state_raw = std::fs::read_to_string(&state_p).unwrap();
        assert!(!state_raw.contains("\"providers\""));

        let loaded = load_at(&state_p, &prov_p);
        assert_eq!(loaded, original);
    }

    /// **未知段保留（model_aliases 协作）**：save 只覆写 providers/deleted
    /// 两个 key，providers.json 里 gateway 写入的 model_aliases 原样保留。
    #[test]
    fn save_overlay_preserves_unknown_sections() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &prov_p,
            r#"{
                "providers": { "alpha": { "name": "alpha" } },
                "deleted": [],
                "model_aliases": {
                    "my-claude": { "provider": "alpha", "upstream_model": "claude-sonnet-4" }
                }
            }"#,
        );
        let mut s = load_at(&state_p, &prov_p);
        // 卡片路径改一个 provider（模拟 FileStore persist）。
        s.providers.insert("beta".into(), item_with(&[("name", "beta")]));
        save_at(&state_p, &prov_p, &s).unwrap();

        let raw = std::fs::read_to_string(&prov_p).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert!(v["model_aliases"]["my-claude"].is_object(), "未知段必须保留: {raw}");
        assert!(v["providers"]["beta"].is_object());
        // 二次 load 仍还原 provider + 墓碑语义。
        let s2 = load_at(&state_p, &prov_p);
        assert!(s2.providers.contains_key("beta"));
    }

    /// openspec/specs/provider-management/spec.md：旧 v2 state.json 含 `default_provider_for_direct` 字段 →
    /// alias 解析到 default_selection，save 后落地新字段名。
    #[test]
    fn v2_with_legacy_default_provider_for_direct_upgrades_to_default_selection() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &state_p,
            r#"{
                "version": 2,
                "mode": { "kind": "direct", "provider": "deepseek" },
                "default_provider_for_direct": "deepseek"
            }"#,
        );

        let s = load_at(&state_p, &prov_p);
        assert_eq!(s.default_selection, Some(DefaultSelection::new("deepseek")));

        save_at(&state_p, &prov_p, &s).unwrap();
        let raw = std::fs::read_to_string(&state_p).unwrap();
        assert!(!raw.contains("default_provider_for_direct"));
        assert!(raw.contains("default_selection"));
    }

    /// openspec/specs/provider-management/spec.md：同字段 alias 与命名字段同时出现且矛盾 → state.json 视为
    /// corrupt，runtime 回退 default；providers 侧（overlay）不受影响。
    #[test]
    fn v2_conflicting_default_provider_and_selection_falls_back_runtime_only() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(
            &prov_p,
            r#"{ "providers": { "alpha": { "name": "alpha" } }, "deleted": [] }"#,
        );
        write_file(
            &state_p,
            r#"{
                "version": 2,
                "default_provider_for_direct": "old-provider",
                "default_selection": { "provider": "new-provider", "model": "new-model" }
            }"#,
        );
        let s = load_at(&state_p, &prov_p);
        // runtime corrupt → default；providers 侧照常。
        assert_eq!(s.mode, ProviderMode::Off);
        assert_eq!(s.default_selection, None);
        assert!(s.providers.contains_key("alpha"), "overlay 数据不受 state 损坏影响");
    }

    /// DefaultSelection wire 形状回归（openspec/specs/provider-management/spec.md）。
    #[test]
    fn default_selection_wire_shape() {
        let original = DefaultSelection::with_model("anthropic", "claude-3-5-sonnet");
        let swapped = r#"{"model":"claude-3-5-sonnet","provider":"anthropic"}"#;
        let parsed: DefaultSelection = serde_json::from_str(swapped).unwrap();
        assert_eq!(parsed, original);

        let s = DefaultSelection::new("deepseek");
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("\"model\""));
        let parsed: DefaultSelection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, s);
    }

    /// 「delete default provider」原子性。
    #[test]
    fn delete_provider_atomically_clears_default() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        let s = PersistedState {
            version: STATE_VERSION_V2,
            providers: BTreeMap::from([(
                "deepseek".to_string(),
                item_with(&[("name", "deepseek")]),
            )]),
            deleted: Vec::new(),
            mode: ProviderMode::Direct {
                provider: "deepseek".into(),
            },
            default_selection: Some(DefaultSelection::new("deepseek")),
            model_aliases: BTreeMap::new(),
        };
        save_at(&state_p, &prov_p, &s).unwrap();
        let mut loaded = load_at(&state_p, &prov_p);
        loaded.providers.remove("deepseek");
        loaded.deleted.push("deepseek".into());
        loaded.default_selection = None;
        save_at(&state_p, &prov_p, &loaded).unwrap();

        let s2 = load_at(&state_p, &prov_p);
        assert!(!s2.providers.contains_key("deepseek"));
        assert!(s2.deleted.contains(&"deepseek".to_string()));
        assert_eq!(s2.default_selection, None);
        assert_eq!(s2.mode, ProviderMode::Off, "repair 应清掉指向墓碑的 Direct mode");
    }

    /// save 父目录不存在 → 自动创建。
    #[test]
    fn save_creates_missing_parent_dir() {
        let dir = tempfile::tempdir().unwrap();
        let nested_state = dir.path().join("a").join("b").join("state.json");
        let nested_prov = dir.path().join("a").join("b").join("providers.json");
        save_at(&nested_state, &nested_prov, &PersistedState::default()).unwrap();
        assert!(nested_state.exists());
        assert!(nested_prov.exists());
    }

    /// 未知 version（如 99）：runtime 回退 default（providers 侧不受影响）。
    #[test]
    fn unknown_version_falls_back_runtime_only() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(&prov_p, r#"{ "providers": { "alpha": { "name": "alpha" } }, "deleted": [] }"#);
        write_file(&state_p, r#"{ "version": 99 }"#);
        let s = load_at(&state_p, &prov_p);
        assert_eq!(s.mode, ProviderMode::Off);
        assert!(s.providers.contains_key("alpha"));
    }

    /// corrupt state.json：runtime 回退 default；providers 侧照常加载。
    #[test]
    fn corrupt_state_json_falls_back_runtime_only() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(&prov_p, r#"{ "providers": { "alpha": { "name": "alpha" } }, "deleted": [] }"#);
        write_file(&state_p, "{not valid json");
        let s = load_at(&state_p, &prov_p);
        assert_eq!(s.mode, ProviderMode::Off);
        assert!(s.providers.contains_key("alpha"));
    }

    /// corrupt providers.json：providers 回退 default（文件保留原位，由
    /// 上层 self-heal 备份）；runtime 侧照常。
    #[test]
    fn corrupt_overlay_falls_back_providers_only() {
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        write_file(&prov_p, "{not valid json");
        write_file(
            &state_p,
            r#"{ "version": 2, "mode": { "kind": "gateway" } }"#,
        );
        let s = load_at(&state_p, &prov_p);
        assert_eq!(s.mode, ProviderMode::Gateway);
        assert!(s.providers.is_empty());
        // 破损文件保留在原位（备份是上层 provider.rs 的职责）。
        assert!(prov_p.exists());
    }
}
