//! Split state store: runtime 状态与 provider 数据。
//!
//! **状态库是唯一权威**（`init_engine` 后）：`load/save/update` 全部委托给
//! `StateStoreEngine`（core 进程内的 `DbStateEngine`）。**没有文件回退**
//! （retire-legacy-state-json 3.2）：`~/.sebas/state.json` 与
//! `~/.sebas/providers.json` 既不写也不读，也不做任何遗留导入——
//! `SEBAS_STATE_FILE` / `SEBAS_ROUTER_PROVIDER_OVERLAY` 随之退休。
//!
//! - provider 数据（`providers` CRUD delta + `deleted` 墓碑 + `model_aliases`）
//!   落 `settings.db` 的 `providers` / `model_aliases` 表；
//! - runtime 决策（`mode` + `default_selection`）落 `settings` 表的
//!   `runtime_state` 键。
//!
//! ## 引擎不可用时
//!
//! 引擎未初始化（如 DB 初始化失败、无 core 的独立进程、测试夹具）意味着
//! **状态不可用**，不是「回退到文件」：`load()` 按 default 呈现并 warn 点名
//! 成因，`save()` / `update()` 以 typed 错误拒绝（绝不静默吞掉写）。
//! 遗留文件留在盘上不动，操作员可自行删除。
//!
//! ## 演进史
//!
//! - 最初：providers.json 只放 provider CRUD delta。
//! - openspec/specs/provider-management/spec.md：合并进 state.json v2 单文件。
//! - router-admin-api-and-model-aliases：拆回，providers.json 成双写者真源。
//! - add-state-store：SQLite 成为唯一写路径，两文件退化为降级回退。
//! - retire-legacy-state-json：**回退也删掉**（文件既不写也不读、不导入）。
//!
//! 遗留 v0/v1 文件的语义是**读入拒绝**：即便盘上还留着旧 `state.json`，它的
//! `mode` / `default_selection` / `providers` 一律不进库。

use crate::provider_state::ProviderMode;
use sebas_models::project::ProjectRow;
use sebas_models::session_map::SessionMapRow;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// 状态存储引擎 trait: 抽象 SQLite/文件后端。
#[async_trait::async_trait]
pub trait StateStoreEngine: Send + Sync {
    async fn load_persisted_state(&self) -> PersistedState;
    async fn save_persisted_state(&self, state: PersistedState) -> anyhow::Result<()>;
    async fn load_settings(&self) -> Result<Option<serde_json::Value>, String>;
    async fn save_settings(&self, cfg: serde_json::Value) -> Result<(), String>;
    /// Load projects (migrate-project-registry 2.2：类型化规范记录，
    /// 不再走 serde_json::Value 往返)。
    async fn load_projects(&self) -> Result<Vec<ProjectRow>, String>;
    /// Save projects (replace all).
    async fn save_projects(&self, projects: Vec<ProjectRow>) -> Result<(), String>;
    /// Add a project entry（节点维度进存储：本地 `local`，远端节点名）。
    async fn add_project(
        &self,
        node_id: &str,
        path: &str,
        name: &str,
        added_at: i64,
    ) -> Result<(), String>;
    /// Remove a project by path.
    async fn remove_project(&self, path: &str) -> Result<bool, String>;
    /// 记录项目级默认 agent（workbench-agent-wire-fix 2.6）。按稳定 id 定位。
    async fn set_project_default_agent(&self, id: &str, agent: &str) -> Result<(), String>;

    // ---- agents 域（add-agent-settings-and-session-titles 1.3）----
    //
    // 默认实现只服务未覆盖它们的既有测试替身（它们的被测面不含 agent 目录）；
    // 生产实现必须如实落库（`DbStateEngine`，settings.db 的 agents 表）。

    /// 全部 agent 行（snapshot 投影源；空表 = 目录只余内置 native）。
    async fn load_agents(&self) -> Result<Vec<sebas_models::agent::AgentRow>, String> {
        tracing::debug!("agent catalog load hit the no-op engine default");
        let _ = self;
        Ok(Vec::new())
    }

    /// 单行 upsert（put op 的存储半边；存在性由调用方裁决 create/update）。
    async fn put_agent(&self, row: sebas_models::agent::AgentRow) -> Result<(), String> {
        tracing::debug!(id = %row.id, "agent put hit the no-op engine default");
        let _ = row;
        Ok(())
    }

    /// 按 id 删除一行。返回是否存在（不存在 = typed rejection 的依据）。
    async fn delete_agent(&self, id: &str) -> Result<bool, String> {
        tracing::debug!(id = %id, "agent delete hit the no-op engine default");
        let _ = id;
        Ok(false)
    }

    /// 清除引用某 agent 的项目默认（删除守卫的 projects 半边）。返回被清除
    /// 的项目数。
    async fn clear_project_default_agent(&self, agent: &str) -> Result<usize, String> {
        let _ = agent;
        Ok(0)
    }

    // ---- 会话映射（persist-session-map 2.1）：按变更持久化到状态库 ----
    //
    // 默认实现只服务未覆盖它们的既有测试替身（它们的被测面不含会话映射）；
    // 生产实现必须如实落库（`DbStateEngine`），绝不拿默认 no-op 冒充成功。

    /// 加载全部持久化映射行（core 启动恢复用；空库 → 空表）。
    async fn load_session_map(&self) -> Result<Vec<SessionMapRow>, String> {
        let _ = self;
        Ok(Vec::new())
    }

    /// 按变更保存一条映射（upsert，主键 = 会话键）。响应返回前即已提交
    /// （state-store「Mutation durability」）。
    async fn save_session_entry(&self, entry: SessionMapRow) -> Result<(), String> {
        tracing::debug!(chat_id = %entry.chat_id, "session entry save hit the no-op engine default");
        let _ = entry;
        Ok(())
    }

    /// 删除一条映射（会话被移除而非退役时）。
    async fn delete_session_entry(&self, chat_id: String, thread_id: Option<String>) -> Result<(), String> {
        tracing::debug!(chat_id = %chat_id, "session entry delete hit the no-op engine default");
        let _ = (chat_id, thread_id);
        Ok(())
    }
}

/// 全局状态存储引擎 (add-state-store)。
///
/// 槽里存的是 `&'static`（`Box::leak`）：`engine()` 因此仍返回静态引用，
/// 调用方零改动。槽本身可换（[`install_engine`]），这是**测试夹具**换引擎
/// 的唯一入口——生产只在启动期 `init_engine` 一次。
static ENGINE: OnceLock<std::sync::RwLock<Option<&'static (dyn StateStoreEngine + Send + Sync)>>> =
    OnceLock::new();
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

fn engine_slot()
-> &'static std::sync::RwLock<Option<&'static (dyn StateStoreEngine + Send + Sync)>> {
    ENGINE.get_or_init(|| std::sync::RwLock::new(None))
}

/// 初始化全局状态存储引擎 + 变更通知广播。**每个进程只允许一次**
/// （重复调用 panic —— 引擎是进程级权威，静默换掉它等于切换真相来源）。
pub fn init_engine(engine: Box<dyn StateStoreEngine + Send + Sync>) {
    let (tx, _) = tokio::sync::broadcast::channel(64);
    CHANGE_TX.set(tx).expect("state change broadcast 已初始化");
    let mut slot = engine_slot().write().unwrap_or_else(|e| e.into_inner());
    assert!(slot.is_none(), "state store engine 已初始化");
    *slot = Some(Box::leak(engine));
}

/// 测试夹具专用：无条件替换全局引擎（旧引擎泄漏，进程生命周期内无害）。
///
/// 全局引擎是进程级的，所以调用方必须自己串行化（见
/// [`crate::test_engine::install_fresh`]，它连锁一起给）。
#[doc(hidden)]
pub fn install_engine(engine: Box<dyn StateStoreEngine + Send + Sync>) {
    let mut slot = engine_slot().write().unwrap_or_else(|e| e.into_inner());
    *slot = Some(Box::leak(engine));
}

/// 测试夹具专用：清空全局引擎（回到「状态库不可用」姿态）。
#[doc(hidden)]
pub fn clear_engine() {
    let mut slot = engine_slot().write().unwrap_or_else(|e| e.into_inner());
    *slot = None;
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
pub fn engine() -> Option<&'static (dyn StateStoreEngine + Send + Sync)> {
    let slot = ENGINE.get()?;
    let guard = slot.read().unwrap_or_else(|e| e.into_inner());
    *guard
}

/// 一条记录：字段名 -> 值。Provider CRUD 用。
pub type Item = Map<String, Value>;

// provider 状态词表与 overlay wire 形状已迁往 `sebas_domain::provider`
// （add-domain-layer 3.3，design D5「仍是词表的部分」），原位再导出。
pub use sebas_domain::provider::{DefaultSelection, ModelAliasEntry};

/// 目标 schema 版本号。`PersistedState::default()` 和 `save()` 都写这个版本。
pub const STATE_VERSION_V2: u32 = 2;
/// 旧版 schema：没有 `version` 字段或 version=1 — 只含 mode + default_provider_for_direct。
pub const STATE_VERSION_V1: u32 = 1;

/// Runtime 「DIRECT 默认」选择（openspec/specs/provider-management/spec.md）。
///
/// 把旧 `default_provider_for_direct: Option<String>` 和 overlay item 上的
/// `default_model` 合并到一个 `(provider, model)` 元组：
/// - `provider`：DIRECT 模式下默认启用的 provider 名（必须存在于 `providers`
///   或 `router_cfg`，否则 spawn-time 兜底回退 Off + warn）；
/// - `model`：spawn 时追加的 `--model <id>`（仅在 Direct 模式下生效；Router
///   模式由 router 自己路由）。
///
/// Overlay item 上的 `default_model` 仍是 UI 源（`/provider` 详情面板的「默认
/// model」文本框），但 spawn 时只信 `default_selection.model`。"set as default"
/// 动作负责把 overlay 的 `default_model` 同步进 `default_selection.model`（见
/// `sebas_dispatch::engine::provider_card::handle_set_default_direct` 的 merge helper）。
///
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
    /// 模型别名（add-state-store 5.3）：router admin API 拥有。随状态库
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

/// 状态库不可用的成因（`None` = 引擎在场）。读取方据此如实呈现 unavailable，
/// 而不是拿文件派生值冒充现状（state-store spec「unavailability is reported,
/// not hidden」）。
pub fn unavailable_cause() -> Option<&'static str> {
    if engine().is_some() {
        None
    } else {
        Some("state store engine 未初始化（无 DB 后端）：状态不可用，按默认呈现（不再回退读取遗留文件）")
    }
}

/// provider overlay 文件路径：**已退休**（retire-legacy-state-json 3.4）。
/// 没有文件读取点，本函数只保留给「退役变量的提示文案」使用。
pub const RETIRED_STATE_FILE_VAR: &str = "SEBAS_STATE_FILE";
/// 见 [`RETIRED_STATE_FILE_VAR`]。
pub const RETIRED_PROVIDER_OVERLAY_VAR: &str = "SEBAS_ROUTER_PROVIDER_OVERLAY";

/// 检出当前进程环境里已退休仍被导出的状态文件变量（启动日志提示用）。
/// 只报告，**不读取其值**——退休变量的语义是「无效果」（design D5）。
pub fn retired_file_env_vars_present() -> Vec<&'static str> {
    [RETIRED_STATE_FILE_VAR, RETIRED_PROVIDER_OVERLAY_VAR]
        .into_iter()
        .filter(|v| std::env::var(v).is_ok())
        .collect()
}

// `expand_tilde` 唯一实现在 `sebas_domain::prim`（add-domain-layer 2.4）；
// 既有调用点经 `use` 零改动。
pub use sebas_domain::prim::expand_tilde;

/// 在同步代码里等待 engine 的 future。三种上下文：
///
/// - **多线程运行时内**（生产：`sebas run` 起的 runtime，也是 `load()` 被同步
///   桥接调用时的唯一生产姿态）：先 `block_in_place` 让出当前 worker，再
///   `block_on`——直接 `block_on` 会 panic（"Cannot start a runtime from
///   within a runtime"）。
/// - **完全没有运行时**（纯同步调用点，如启动期的同步夹具）：现建一个临时
///   current-thread runtime 驱动。这保住了「`load()` 是同步函数」这一既有契约
///   ——退休文件回退之后它仍然要能在任何同步上下文里被调用。
/// - **current-thread 运行时内**：无法安全阻塞（`block_in_place` 会 panic，
///   原地 `block_on` 会死锁），这是调用方的错误用法——留原样 panic，panic
///   信息已足够定位。
fn block_on_engine<F: std::future::Future>(fut: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("构建临时 runtime 以同步驱动状态库调用")
            .block_on(fut),
    }
}

/// 读当前状态。引擎在场时**只走库**；不在场时按 default 呈现并 warn 点名成因
/// ——绝不回退读 `state.json` / `providers.json`（retire-legacy-state-json 3.2）。
///
/// tombstone repair 在读时统一施加（`repair_mode`）：`mode` 指向已删除
/// provider 时重置为 Off。读时修比写时修安全——写时拿不到 providers +
/// deleted 的最新视图。
pub fn load() -> PersistedState {
    let Some(engine) = engine() else {
        tracing::warn!("{}", unavailable_cause().unwrap_or_default());
        return PersistedState::default();
    };
    repair_mode(block_on_engine(engine.load_persisted_state()))
}

/// 写状态。引擎不在场时以 typed 错误拒绝——**不写文件**、也不静默丢弃写
/// （静默成功比失败更危险：调用方会以为已持久化）。
pub fn save(s: &PersistedState) -> anyhow::Result<()> {
    let Some(engine) = engine() else {
        return Err(anyhow::anyhow!(
            "{}",
            unavailable_cause().unwrap_or("state store 不可用")
        ));
    };
    let state = s.clone();
    block_on_engine(engine.save_persisted_state(state)).map_err(|e| anyhow::anyhow!("{}", e))
}

/// 读 → 改 → 写一气呵成。`f` 闭包基于当前 state 做条件决策；返回改后的 state。
///
/// 读或写任一侧不可用即 Err（同 [`save`]）。
pub fn update<F>(f: F) -> anyhow::Result<PersistedState>
where
    F: FnOnce(&mut PersistedState),
{
    let Some(engine) = engine() else {
        return Err(anyhow::anyhow!(
            "{}",
            unavailable_cause().unwrap_or("state store 不可用")
        ));
    };
    let mut state = block_on_engine(engine.load_persisted_state());
    f(&mut state);
    let snapshot = state.clone();
    block_on_engine(engine.save_persisted_state(state)).map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(snapshot)
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

// ---- 域 mutation 分发（core channel 服务端与 webui InProcessBackend 共用；
// make-core-own-provider-data 1.1/1.2：单一实现避免两侧漂移）----

/// providers 域条目的已知字段集（make-core-own-provider-data 1.2）。未知
/// 字段 = 非法 payload，mutation 以 typed rejection 拒绝（不静默吞错）。
/// `name` 是卡片表单写入的展示名；其余与 router `ProviderConfig` /
/// `/provider` 表单字段一一对应。
const PROVIDER_ITEM_KNOWN_FIELDS: &[&str] = &[
    "name",
    "preset",
    "base_url_anthropic",
    "base_url_openai_chat",
    "base_url_openai_responses",
    "api_key",
    "api_key_env",
    "default_model",
    "protocol",
    "models",
    "model_map",
];

/// 校验单个 provider 条目（1.2）并就地归一化 `models`（task 1.1「写回为
/// 条目」）：未知字段 / 类型错误 → Err（错误信息含 provider 名与字段名，
/// 调用方把它作为 rejection cause 透传）；`models` 数组元素经 router 侧
/// `ModelEntry` 兼容反序列化（裸字符串 → 仅隐含 text 的条目），未知能力
/// 标记 → 显式拒绝；逗号分隔字符串 → 条目数组。写出的 `models` 一律是
/// `{"id", "tags"}` 条目对象的规范数组。preset 派生条目显式写 URL 的拒绝
/// 在 router 侧 resolve 管线里管（core 不复制该规则——条目形状合法性在
/// 此把关即可，避免跨 crate 语义复制）。
fn validate_and_normalize_provider_item(name: &str, item: &mut Item) -> Result<(), String> {
    for key in item.keys() {
        if !PROVIDER_ITEM_KNOWN_FIELDS.contains(&key.as_str()) {
            return Err(format!("put: provider '{name}' 含未知字段 '{key}'"));
        }
    }
    for field in [
        "preset",
        "api_key",
        "api_key_env",
        "default_model",
        "protocol",
    ] {
        if let Some(v) = item.get(field)
            && !v.is_null()
            && !v.is_string()
        {
            return Err(format!(
                "put: provider '{name}' 字段 '{field}' 必须是字符串"
            ));
        }
    }
    match item.get("models") {
        None | Some(Value::Null) => {
            item.remove("models");
        }
        Some(Value::String(s)) => {
            let entries: Vec<Value> = s
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(sebas_router::models::ModelEntry::text_only)
                .map(|e| serde_json::to_value(&e).expect("ModelEntry serializes"))
                .collect();
            item.insert("models".into(), Value::Array(entries));
        }
        Some(Value::Array(arr)) => {
            let mut entries = Vec::with_capacity(arr.len());
            for el in arr {
                let e: sebas_router::models::ModelEntry = serde_json::from_value(el.clone())
                    .map_err(|e| format!("put: provider '{name}' 字段 'models' 条目非法: {e}"))?;
                entries.push(serde_json::to_value(&e).expect("ModelEntry serializes"));
            }
            item.insert("models".into(), Value::Array(entries));
        }
        Some(_) => {
            return Err(format!(
                "put: provider '{name}' 字段 'models' 必须是条目数组或逗号分隔字符串"
            ));
        }
    }
    Ok(())
}

/// providers 域 mutation 分发（5.3 admin 写路径通道代理；自
/// make-core-own-provider-data 起这是 provider 数据唯一写通道）。
/// payload `op` 子操作：
/// - `{"op":"put","name":"...","item":{...}}` → upsert provider（1.2：条目
///   先过字段/类型校验，非法 payload 走 rejection）
/// - `{"op":"delete","name":"..."}` → 删除 + 写墓碑
/// - `{"op":"save","state":{PersistedState 形状}}` → 全量替换
///
/// 全部经 RMW（读 → 改 → save_persisted_state），与 router 卡片写路径同语义。
pub async fn providers_mutation(
    engine: &(dyn StateStoreEngine + Send + Sync),
    payload: &Value,
) -> Result<(), String> {
    let op = payload.get("op").and_then(Value::as_str).unwrap_or("save");
    let mut state = engine.load_persisted_state().await;
    match op {
        "put" => {
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "put: 缺少 name 字段".to_string())?;
            let mut item = payload
                .get("item")
                .and_then(Value::as_object)
                .cloned()
                .ok_or_else(|| "put: 缺少 item 对象".to_string())?;
            validate_and_normalize_provider_item(name, &mut item)?;
            state.providers.insert(name.to_string(), item);
            // 撤销同名墓碑（re-add）。
            state.deleted.retain(|d| d != name);
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        "delete" => {
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "delete: 缺少 name 字段".to_string())?;
            let existed = state.providers.remove(name).is_some();
            if !state.deleted.iter().any(|d| d == name) {
                state.deleted.push(name.to_string());
            }
            if state
                .default_selection
                .as_ref()
                .map(|d| d.provider.as_str())
                == Some(name)
            {
                state.default_selection = None;
            }
            if !existed {
                return Err(format!("provider '{name}' 不存在"));
            }
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        "save" => {
            let raw = payload
                .get("state")
                .cloned()
                .ok_or_else(|| "save: 缺少 state 字段".to_string())?;
            let mut incoming: PersistedState = serde_json::from_value(raw)
                .map_err(|e| format!("save: 非法 PersistedState: {e}"))?;
            // 校验每个条目（1.2：与 put 同一把关）并归一化 models（1.1）。
            for (name, item) in &mut incoming.providers {
                validate_and_normalize_provider_item(name, item)?;
            }
            // 保留 mode/default_selection（admin 面不管运行时状态，只写 provider 数据）。
            let mut merged = incoming;
            merged.mode = state.mode.clone();
            merged.default_selection = state.default_selection.clone();
            engine
                .save_persisted_state(merged)
                .await
                .map_err(|e| e.to_string())
        }
        other => Err(format!("providers: 未知 op '{other}'")),
    }
}

// ---- providers 域抓取 op（add-fetch-models D5：provider 域上的只读动作，
// ---- 与 put/delete/save 同域；不落任何存储分区）----

/// 纯解析：provider 条目（含 preset 代码表物化）→ 抓取目标 `(url, key)`。
/// URL 槽位优先级 openai_chat → openai_responses → anthropic（specs delta
/// 原文；失败不回退其他槽位，由 [`sebas_router::probe::fetch_models`] 的
/// 单 URL 语义承担）。无可用槽位 → Err（typed rejection naming that
/// reason，且不发上游请求）。
///
/// 密钥解析优先级：条目明文 `api_key` → 条目 `api_key_env` → preset 代码表
/// `api_key_env`（preset 派生条目不落盘 env 名时跟随代码）。两者皆无 →
/// `None`：跳过 Authorization 头（与旧卡片探测同一姿态）。
pub fn provider_fetch_target(item: &Item) -> Result<(String, Option<String>), String> {
    let slots = (
        crate::engine::provider_card::effective_field(item, "base_url_openai_chat"),
        crate::engine::provider_card::effective_field(item, "base_url_openai_responses"),
        crate::engine::provider_card::effective_field(item, "base_url_anthropic"),
    );
    let (url, _kind) = sebas_router::probe::resolve_fetch_url(
        slots.0.as_deref(),
        slots.1.as_deref(),
        slots.2.as_deref(),
    )
    .ok_or_else(|| "未配置任何 base URL 槽位".to_string())?;
    // 密钥：明文 → env 名（条目 → preset 代码表）→ None。
    let plain = item
        .get("api_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let key = if let Some(k) = plain {
        Some(k)
    } else {
        let env_name = item
            .get("api_key_env")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                let preset_name = item.get("preset").and_then(Value::as_str)?;
                sebas_router::config::presets()
                    .iter()
                    .find(|p| p.name == preset_name)
                    .map(|p| p.api_key_env.to_string())
            })
            .filter(|s| !s.is_empty());
        match env_name.and_then(|name| std::env::var(name).ok()) {
            Some(v) if !v.trim().is_empty() => Some(v),
            _ => None,
        }
    };
    Ok((url, key))
}

/// providers 域抓取 op（core channel 与 webui InProcessBackend 共用；spec
/// "Model list fetch over the channel"）：以 provider 名从 store 解析条目，
/// 执行**一次只读 GET**（5s 超时、无重试），返回上游 model id 列表。
///
/// 抓取**不改 provider 任何字段、不持久化任何东西**——本函数从不调用
/// `save_persisted_state`；id 进入模型列表只能走后续的普通编辑（put）。
/// 错误 = typed rejection：`fetch_models: ` 前缀 + 净化原因（只含状态码/
/// 类别，绝无 key 材料与上游 body）。
pub async fn providers_fetch_models(
    engine: &(dyn StateStoreEngine + Send + Sync),
    name: &str,
) -> Result<Vec<String>, String> {
    let state = engine.load_persisted_state().await;
    let item = state
        .providers
        .get(name)
        .ok_or_else(|| format!("fetch_models: provider '{name}' 不存在（store 无此条目）"))?;
    let (url, key) =
        provider_fetch_target(item).map_err(|e| format!("fetch_models: provider '{name}' {e}"))?;
    sebas_router::probe::fetch_models(&sebas_router::probe::fetch_client(), &url, key.as_deref())
        .await
        .map_err(|e| format!("fetch_models: provider '{name}' 上游抓取失败: {e}"))
}

/// aliases 域 mutation 分发（5.3 admin 写路径通道代理）。
/// payload `op` 子操作：
/// - `{"op":"put","alias":"...","entry":{"provider":"...","upstream_model":"..."}}`
/// - `{"op":"delete","alias":"..."}`
/// - `{"op":"save","aliases":{alias: entry,...}}` → 全量替换
pub async fn aliases_mutation(
    engine: &(dyn StateStoreEngine + Send + Sync),
    payload: &Value,
) -> Result<(), String> {
    let op = payload.get("op").and_then(Value::as_str).unwrap_or("save");
    let mut state = engine.load_persisted_state().await;
    /// entry 的已知字段集（1.2：未知字段拒绝）。
    const ENTRY_KNOWN_FIELDS: &[&str] = &["provider", "upstream_model"];
    let validate_entry = |where_: &str, entry: &Value| -> Result<(), String> {
        let obj = entry
            .as_object()
            .ok_or_else(|| format!("{where_}: entry 必须是对象"))?;
        for key in obj.keys() {
            if !ENTRY_KNOWN_FIELDS.contains(&key.as_str()) {
                return Err(format!("{where_}: entry 含未知字段 '{key}'"));
            }
        }
        let provider = obj
            .get("provider")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("{where_}: entry 缺少 provider 字段"))?;
        if let Some(up) = obj.get("upstream_model")
            && !up.is_null()
            && !up.is_string()
        {
            return Err(format!(
                "{where_}: entry 字段 'upstream_model' 必须是字符串"
            ));
        }
        let _ = provider;
        Ok(())
    };
    match op {
        "put" => {
            let alias = payload
                .get("alias")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "put: 缺少 alias 字段".to_string())?;
            let entry = payload
                .get("entry")
                .cloned()
                .ok_or_else(|| "put: 缺少 entry 对象".to_string())?;
            validate_entry("put", &entry)?;
            let entry: ModelAliasEntry =
                serde_json::from_value(entry).map_err(|e| format!("put: entry 非法: {e}"))?;
            state.model_aliases.insert(alias.to_string(), entry);
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        "delete" => {
            let alias = payload
                .get("alias")
                .and_then(Value::as_str)
                .ok_or_else(|| "delete: 缺少 alias 字段".to_string())?;
            if state.model_aliases.remove(alias).is_none() {
                return Err(format!("alias '{alias}' 不存在"));
            }
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        "save" => {
            let raw = payload
                .get("aliases")
                .cloned()
                .ok_or_else(|| "save: 缺少 aliases 字段".to_string())?;
            let map = raw
                .as_object()
                .ok_or_else(|| "save: aliases 必须是对象".to_string())?;
            let mut incoming: BTreeMap<String, ModelAliasEntry> = BTreeMap::new();
            for (alias, entry) in map {
                validate_entry("save", entry)?;
                incoming.insert(
                    alias.clone(),
                    serde_json::from_value(entry.clone())
                        .map_err(|e| format!("save: alias '{alias}' entry 非法: {e}"))?,
                );
            }
            state.model_aliases = incoming;
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        other => Err(format!("aliases: 未知 op '{other}'")),
    }
}

/// settings 域 mutation 分发（make-core-own-provider-data 1.1：defaults 并入
/// settings 域，与 provider 数据同事务落盘）。
/// - `{"op":"set_defaults","provider":"...","model":"..."?}` → 写
///   `default_selection`（RMW，与 providers/aliases 同一次 save 提交）；
/// - `{"op":"clear_defaults"}` → 清除默认；
/// - 其它（无 `op` / `{"value": {...}}`）→ 既有 CardConfig 保存，wire 形状
///   不变（sebas-im 设置面同款）。
pub async fn settings_mutation(
    engine: &(dyn StateStoreEngine + Send + Sync),
    payload: &Value,
) -> Result<(), String> {
    match payload.get("op").and_then(Value::as_str) {
        Some("set_defaults") => {
            let provider = payload
                .get("provider")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "set_defaults: 缺少 provider 字段".to_string())?;
            let model = payload
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let mut state = engine.load_persisted_state().await;
            state.default_selection = Some(match model {
                Some(m) => DefaultSelection::with_model(provider, m),
                None => DefaultSelection::new(provider),
            });
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        Some("clear_defaults") => {
            let mut state = engine.load_persisted_state().await;
            state.default_selection = None;
            engine
                .save_persisted_state(state)
                .await
                .map_err(|e| e.to_string())
        }
        _ => {
            let value = payload
                .get("value")
                .cloned()
                .unwrap_or_else(|| payload.clone());
            engine.save_settings(value).await
        }
    }
}

/// projects 域 mutation 分发：payload 用 `op` 字段区分子操作。
/// - `{"op": "add", "path": "...", "name": "...", "node_id"?}` → 新增（缺省
///   `local`；migrate-project-registry 3.1 起携带节点维度，added_at 取当前时间）
/// - `{"op": "remove", "path": "..."}` → 删除（不存在返回错误）
/// - `{"op": "save", "projects": [...]}` → 全量替换（规范记录形状）
/// - `{"op": "reorder", "projects": [...]}` → 顺序重排（带 `sort_order` 列的
///   规范记录形状；语义与 save 相同——顺序即 `sort_order` 列，未知 id 落为
///   add_time 顺序尾部是 webui 侧构造 next 列表时完成的）
pub async fn project_mutation(
    engine: &(dyn StateStoreEngine + Send + Sync),
    payload: &Value,
) -> Result<(), String> {
    let op = payload.get("op").and_then(Value::as_str).unwrap_or("save");
    match op {
        "add" => {
            let path = payload
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "add: 缺少 path 字段".to_string())?;
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "add: 缺少 name 字段".to_string())?;
            let node_id = payload
                .get("node_id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or(sebas_models::project::LOCAL_NODE_ID);
            let added_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            engine.add_project(node_id, path, name, added_at).await
        }
        "remove" => {
            let path = payload
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "remove: 缺少 path 字段".to_string())?;
            match engine.remove_project(path).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(format!("remove: project '{path}' 不存在")),
                Err(e) => Err(e),
            }
        }
        "save" | "reorder" => {
            let projects: Vec<ProjectRow> = payload
                .get("projects")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|v| {
                    serde_json::from_value::<ProjectRow>(v)
                        .map_err(|e| format!("{op}: 项目记录形状不符: {e}"))
                })
                .collect::<Result<_, _>>()?;
            engine.save_projects(projects).await
        }
        // workbench-agent-wire-fix 2.6：项目级默认 agent（按稳定 id）。
        "set_default_agent" => {
            let id = payload
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| "set_default_agent: 缺少 id 字段".to_string())?;
            let agent = payload
                .get("agent")
                .and_then(Value::as_str)
                .ok_or_else(|| "set_default_agent: 缺少 agent 字段".to_string())?;
            engine.set_project_default_agent(id, agent).await
        }
        other => Err(format!("projects: 未知 op '{other}'")),
    }
}

// ---- agents 域 mutation 分发（add-agent-settings-and-session-titles 1.3；
// ---- core channel 服务端与 webui InProcessBackend 共用同一实现）----

/// agent 条目（agents 域 put 载荷）的已知字段集。未知字段 = 非法 payload，
/// typed rejection（与 providers 域同一把关姿态）。字段语义见
/// `sebas_models::agent::AgentDefinition`。
pub const AGENT_ITEM_KNOWN_FIELDS: &[&str] = &[
    "driver",
    "path",
    "args",
    "display",
    "models",
    "startup_timeout_secs",
    "idle_kill_secs",
    "work_dir",
];

/// 校验 agent 条目并归一化为 `AgentDefinition`。`driver` 只认封闭标签
/// `claude` | `acp`；`path`/`display`/`work_dir` 是字符串槽位（null 归一为
/// 未配置）；`args`/`models` 是字符串数组（缺省 = 空/未覆盖）；超时两槽位
/// 是非负整数，下限压到 1（与 config 侧 `startup_timeout_for` 的 max(1)
/// 同语义——0 会让 spawn 立即超时）。
pub fn validate_agent_definition(item: &Item) -> Result<sebas_models::agent::AgentDefinition, String> {
    use sebas_models::agent::AgentDefinition;
    for key in item.keys() {
        if !AGENT_ITEM_KNOWN_FIELDS.contains(&key.as_str()) {
            return Err(format!("put: agent 含未知字段 '{key}'"));
        }
    }
    let driver = item
        .get("driver")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "put: agent 缺少 driver 字段".to_string())?;
    if !sebas_models::agent::is_valid_driver(driver) {
        return Err(format!(
            "put: agent driver '{driver}' 非法（只支持 claude | acp）"
        ));
    }
    let string_slot = |field: &str| -> Result<Option<String>, String> {
        match item.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("put: agent 字段 '{field}' 必须是字符串")),
        }
    };
    let list_slot = |field: &str| -> Result<Option<Vec<String>>, String> {
        match item.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(arr)) => {
                let mut out = Vec::with_capacity(arr.len());
                for el in arr {
                    out.push(
                        el.as_str()
                            .ok_or_else(|| {
                                format!("put: agent 字段 '{field}' 条目必须是字符串")
                            })?
                            .to_string(),
                    );
                }
                Ok(Some(out))
            }
            Some(_) => Err(format!("put: agent 字段 '{field}' 必须是字符串数组")),
        }
    };
    let uint_slot = |field: &str, default: u64| -> Result<u64, String> {
        match item.get(field) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => {
                let n = v.as_u64().ok_or_else(|| {
                    format!("put: agent 字段 '{field}' 必须是非负整数")
                })?;
                Ok(n.max(1))
            }
        }
    };
    let path = string_slot("path")?;
    if driver == "acp" && path.as_deref().map(str::trim).filter(|s| !s.is_empty()).is_none() {
        return Err("put: acp agent 需要非空 path（command 的 argv[0]）".to_string());
    }
    Ok(AgentDefinition {
        driver: driver.to_string(),
        path,
        args: list_slot("args")?.unwrap_or_default(),
        display: string_slot("display")?,
        models: list_slot("models")?,
        startup_timeout_secs: uint_slot("startup_timeout_secs", 30)?,
        idle_kill_secs: uint_slot("idle_kill_secs", 172800)?,
        work_dir: string_slot("work_dir")?,
    })
}

/// agents 域 mutation 分发。payload `op` 子操作：
/// - `{"op":"put","id":"...","agent":{driver, path, args, ...}}` → upsert
///   （同 id 覆盖 launch 定义，created_at 保留首次创建时刻）；
/// - `{"op":"delete","id":"..."}` → 删行 + 清除引用该 id 的项目默认
///   （删除守卫；两库各一笔事务——settings 行与 projects 默认分属两库，
///   跨库单事务按分层纪律不存在，design 决策 10 的顺序语义：先清默认再删行，
///   失败即整体报错不留悬空引用）。
///
/// `native` 是内置内核保留 id：不落表、不可写、不可删（spec「built-in
/// `native` kernel … without being stored or deletable」），两个 op 都显式
/// 拒绝。
pub async fn agents_mutation(
    engine: &(dyn StateStoreEngine + Send + Sync),
    payload: &Value,
) -> Result<(), String> {
    use sebas_models::agent::RESERVED_NATIVE_ID;
    let op = payload
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| "agents: 缺少 op 字段".to_string())?;
    match op {
        "put" => {
            let id = payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "put: 缺少 id 字段".to_string())?;
            if id == RESERVED_NATIVE_ID {
                return Err(format!("put: '{RESERVED_NATIVE_ID}' 是内置内核保留 id，不可占用"));
            }
            let item = payload
                .get("agent")
                .and_then(Value::as_object)
                .cloned()
                .ok_or_else(|| "put: 缺少 agent 对象".to_string())?;
            let def = validate_agent_definition(&item)?;
            let mut row = sebas_models::agent::AgentRow::from_definition(id, &def, "ui");
            // 同 id 覆盖保留首次创建时刻与原 source（更新不改来源账目）。
            if let Some(existing) = engine
                .load_agents()
                .await?
                .into_iter()
                .find(|r| r.id == id)
            {
                row.created_at = existing.created_at;
                row.source = existing.source;
            }
            engine.put_agent(row).await?;
            Ok(())
        }
        "delete" => {
            let id = payload
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "delete: 缺少 id 字段".to_string())?;
            if id == RESERVED_NATIVE_ID {
                return Err(format!(
                    "delete: '{RESERVED_NATIVE_ID}' 是内置内核，不可删除"
                ));
            }
            // 删除守卫（先清默认）：projects 侧引用清干净后再删行。
            let cleared = engine.clear_project_default_agent(id).await?;
            if cleared > 0 {
                tracing::info!(
                    agent = %id,
                    projects = cleared,
                    "cleared project default_agent references for deleted agent"
                );
            }
            let existed = engine.delete_agent(id).await?;
            if !existed {
                return Err(format!("agent '{id}' 不存在"));
            }
            Ok(())
        }
        other => Err(format!("agents: 未知 op '{other}'")),
    }
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
            "mode points to a tombstoned provider, resetting to Off (repair-on-load)"
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

    /// add-fetch-models 1.4 验收：preset 派生 provider 无存储 URL 时抓取仍
    /// 发起——`provider_fetch_target` 从代码表物化出抓取 URL（deepseek 的
    /// openai_chat 槽位），不因「条目未落盘 url」而拒绝。本测试只断言解析
    /// 结果（纯函数，不外联）：全路径的 mock 上游往返由 core channel 侧
    /// `tests/state_channel_contract_test.rs` 对自定义 provider 覆盖。
    #[test]
    fn preset_derived_provider_resolves_fetch_target_from_code_table() {
        let mut item = Map::new();
        item.insert("name".into(), Value::String("my-deepseek".into()));
        item.insert("preset".into(), Value::String("deepseek".into()));
        item.insert("api_key".into(), Value::String("sk-preset-derived".into()));

        let (url, key) = provider_fetch_target(&item).expect("preset table materializes a URL");
        assert_eq!(
            url,
            format!("{}/models", "https://api.deepseek.com"),
            "fetch URL comes from the preset code table's openai_chat slot"
        );
        assert_eq!(key.as_deref(), Some("sk-preset-derived"));

        // 无明文 key 时跟随代码表 api_key_env 解析（env 未设置 → None，仍可
        // 发起匿名抓取——与卡片探测同一姿态），绝不因此拒绝。
        let mut item = item.clone();
        item.remove("api_key");
        let (_, key) = provider_fetch_target(&item).expect("still resolvable");
        let env_set = std::env::var("DEEPSEEK_API_KEY").is_ok_and(|v| !v.trim().is_empty());
        assert_eq!(
            key.is_some(),
            env_set,
            "key resolution follows the code-table env name"
        );
    }

    /// add-fetch-models 1.2 验收（解析层）：无任何槽位、无 preset → Err。
    #[test]
    fn provider_fetch_target_rejects_when_no_usable_base_url() {
        let mut item = Map::new();
        item.insert("name".into(), Value::String("bare".into()));
        assert!(provider_fetch_target(&item).is_err());

        // 空白槽位视同未配置。
        item.insert("base_url_openai_chat".into(), Value::String("  ".into()));
        item.insert("base_url_anthropic".into(), Value::String("".into()));
        assert!(provider_fetch_target(&item).is_err());
    }

    /// redesign-provider-models-settings 1.1 验收：遗留字符串 models 列表
    /// 可读且无错（归一化为仅隐含 text 的条目）；写回（put/save 落库形状）
    /// 一律是 `{"id","tags"}` 条目对象。
    #[test]
    fn provider_models_legacy_strings_read_and_normalize_to_entries() {
        let mut item = Map::new();
        item.insert(
            "base_url_anthropic".into(),
            Value::String("https://x".into()),
        );
        item.insert("models".into(), serde_json::json!(["m1", "m2"]));
        validate_and_normalize_provider_item("lg", &mut item).expect("legacy list reads");
        assert_eq!(
            item.get("models"),
            Some(&serde_json::json!([
                {"id": "m1", "tags": []},
                {"id": "m2", "tags": []},
            ])),
            "write-back is the entry-object shape"
        );

        let mut item = Map::new();
        item.insert("models".into(), Value::String("m1, m2".into()));
        validate_and_normalize_provider_item("cm", &mut item).expect("comma string reads");
        assert_eq!(
            item.get("models"),
            Some(&serde_json::json!([
                {"id": "m1", "tags": []},
                {"id": "m2", "tags": []},
            ]))
        );

        let mut item = Map::new();
        item.insert("models".into(), Value::Null);
        validate_and_normalize_provider_item("nn", &mut item).expect("null models");
        assert!(item.get("models").is_none());
    }

    /// redesign-provider-models-settings 1.4 验收（store 侧）：条目对象带
    /// 标记写出；未知标记显式拒绝（不静默吞）；`text` 拒收（隐含不落盘）；
    /// 非数组非字符串类型拒绝。
    #[test]
    fn provider_models_entry_writes_canonical_unknown_tags_rejected() {
        let mut item = Map::new();
        item.insert(
            "models".into(),
            serde_json::json!([{"id": "m1", "tags": ["vision"]}]),
        );
        validate_and_normalize_provider_item("en", &mut item).expect("entries write");
        assert_eq!(
            item.get("models"),
            Some(&serde_json::json!([{"id": "m1", "tags": ["vision"]}])),
        );

        let mut item = Map::new();
        item.insert(
            "models".into(),
            serde_json::json!([{"id": "m1", "tags": ["telepathy"]}]),
        );
        let err = validate_and_normalize_provider_item("um", &mut item).unwrap_err();
        assert!(err.contains("um") && err.contains("telepathy"), "{err}");

        let mut item = Map::new();
        item.insert(
            "models".into(),
            serde_json::json!([{"id": "m1", "tags": ["text"]}]),
        );
        assert!(
            validate_and_normalize_provider_item("tx", &mut item).is_err(),
            "text must never be stored"
        );

        let mut item = Map::new();
        item.insert("models".into(), serde_json::json!(42));
        assert!(validate_and_normalize_provider_item("nm", &mut item).is_err());
    }

    fn item_with(fields: &[(&str, &str)]) -> Item {
        let mut m = Map::new();
        for (k, v) in fields {
            m.insert((*k).into(), Value::String((*v).into()));
        }
        m
    }

    // ---- repair-on-load（纯函数，无需文件） ----

    /// repair-on-load：mode 指向 deleted provider → 自动重置为 Off + 清 default。
    #[test]
    fn repair_mode_clears_stale_direct_pointer() {
        let s = PersistedState {
            version: STATE_VERSION_V2,
            providers: BTreeMap::from([(
                "deepseek".to_string(),
                item_with(&[("name", "deepseek")]),
            )]),
            deleted: vec!["openai".to_string()],
            mode: ProviderMode::Direct {
                provider: "openai".into(),
            },
            default_selection: Some(DefaultSelection::new("openai")),
            model_aliases: BTreeMap::new(),
        };
        let repaired = repair_mode(s);
        assert_eq!(repaired.mode, ProviderMode::Off);
        assert_eq!(repaired.default_selection, None);
    }

    /// repair-on-load 不动「mode 指向不在 providers 里」的情况（合法状态：
    /// 切到 Direct 但还没配 provider）。
    #[test]
    fn repair_mode_keeps_pointer_to_missing_provider_when_no_tombstone() {
        let s = PersistedState {
            version: STATE_VERSION_V2,
            providers: BTreeMap::new(),
            deleted: Vec::new(),
            mode: ProviderMode::Direct {
                provider: "ghost".into(),
            },
            default_selection: None,
            model_aliases: BTreeMap::new(),
        };
        let repaired = repair_mode(s);
        assert_eq!(
            repaired.mode,
            ProviderMode::Direct {
                provider: "ghost".into()
            }
        );
    }

    // ---- 文件回退已退休（retire-legacy-state-json 3.2 / 3.4） ----

    /// 遗留文件在盘上有值 + 库空 → **不导入**：`load()` 仍返回 default，
    /// 且文件逐字节未变（不读、不改、不删）。
    #[test]
    fn legacy_files_are_not_imported_and_not_touched() {
        let _engine = crate::test_engine::install_fresh();
        let dir = tempfile::tempdir().unwrap();
        let state_p = dir.path().join("state.json");
        let prov_p = dir.path().join("providers.json");
        let state_body = r#"{"version":2,"mode":{"kind":"direct","provider":"legacy"},"default_selection":{"provider":"legacy"}}"#;
        let prov_body =
            r#"{"providers":{"legacy":{"name":"legacy"}},"deleted":["openai"]}"#;
        std::fs::write(&state_p, state_body).unwrap();
        std::fs::write(&prov_p, prov_body).unwrap();

        let s = load();
        assert_eq!(s, PersistedState::default(), "遗留文件的值不得进库/进状态");
        assert_eq!(
            std::fs::read_to_string(&state_p).unwrap(),
            state_body,
            "遗留 state.json 必须逐字节未变"
        );
        assert_eq!(
            std::fs::read_to_string(&prov_p).unwrap(),
            prov_body,
            "遗留 providers.json 必须逐字节未变"
        );
    }

    /// 退休的环境变量无效果：即便把它们指向含值的文件，`load()` 仍取库。
    #[test]
    fn retired_file_env_vars_have_no_effect() {
        // 锁序：env 锁在外、引擎锁在内（与 crud/provider_card 同序），否则与
        // 「持引擎锁再取 env 锁」的调用方构成跨线程锁序反转。
        let _lock = crate::test_util::lock_state_file();
        let _engine = crate::test_engine::install_fresh();
        let dir = tempfile::tempdir().unwrap();
        let bogus_state = dir.path().join("elsewhere-state.json");
        let bogus_overlay = dir.path().join("elsewhere-providers.json");
        std::fs::write(
            &bogus_state,
            r#"{"version":2,"mode":{"kind":"router"}}"#,
        )
        .unwrap();
        std::fs::write(
            &bogus_overlay,
            r#"{"providers":{"ghost":{"name":"ghost"}},"deleted":[]}"#,
        )
        .unwrap();

        // SAFETY: env 变更由全局锁串行化（`_lock` 已持）。
        unsafe {
            std::env::set_var(RETIRED_STATE_FILE_VAR, &bogus_state);
            std::env::set_var(RETIRED_PROVIDER_OVERLAY_VAR, &bogus_overlay);
        }
        let s = load();
        // SAFETY: 同上。
        unsafe {
            std::env::remove_var(RETIRED_STATE_FILE_VAR);
            std::env::remove_var(RETIRED_PROVIDER_OVERLAY_VAR);
        }

        assert_eq!(s, PersistedState::default(), "退休变量不得改变读取来源");
        assert!(!s.providers.contains_key("ghost"));
        assert_eq!(s.mode, ProviderMode::Off);
    }

    /// 库不可用 → `load()` 按 default 呈现（不读文件），`save()` 以 typed
    /// 错误拒绝（不写文件、不静默成功）。
    #[test]
    fn unavailable_store_reads_default_and_rejects_writes() {
        let _engine = crate::test_engine::install_none();
        assert!(unavailable_cause().is_some(), "库不可用必须能点名成因");
        assert_eq!(load(), PersistedState::default());

        let err = save(&PersistedState::default()).unwrap_err();
        assert!(err.to_string().contains("不可用"), "{err}");
        assert!(update(|s| s.deleted.push("x".into())).is_err());
    }

    /// 库在场时 `save` → `load` 往返，且 tombstone repair 在读时施加。
    #[test]
    fn engine_round_trips_and_repairs_on_load() {
        let _engine = crate::test_engine::install_fresh();
        let original = PersistedState {
            version: STATE_VERSION_V2,
            providers: BTreeMap::from([(
                "deepseek".to_string(),
                item_with(&[("name", "deepseek"), ("preset", "deepseek")]),
            )]),
            deleted: vec!["openai".to_string()],
            mode: ProviderMode::Direct {
                provider: "openai".into(),
            },
            default_selection: Some(DefaultSelection::new("openai")),
            model_aliases: BTreeMap::new(),
        };
        save(&original).unwrap();

        let loaded = load();
        assert!(loaded.providers.contains_key("deepseek"));
        assert!(loaded.deleted.contains(&"openai".to_string()));
        // mode 指向 tombstone → repair 为 Off + 清 default。
        assert_eq!(loaded.mode, ProviderMode::Off);
        assert_eq!(loaded.default_selection, None);
    }

    // ---- agents 域 mutation（add-agent-settings-and-session-titles 1.3）----

    fn agent_item(driver: &str, path: &str, args: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "driver": driver,
            "path": path,
            "args": args,
        })
    }

    /// put → snapshot 可见；同 id 二次 put 覆盖定义并保留创建时刻；
    /// 非法 payload（未知字段 / 非法 driver / acp 缺 path / native 保留 id）
    /// 全部 typed 拒绝且不落任何行。
    #[tokio::test]
    async fn agents_mutation_put_round_trips_and_rejects_bad_payloads() {
        async fn put(
            engine: &crate::test_engine::MemoryEngine,
            id: &str,
            item: serde_json::Value,
        ) -> Result<(), String> {
            agents_mutation(
                engine,
                &serde_json::json!({ "op": "put", "id": id, "agent": item }),
            )
            .await
        }
        let engine = crate::test_engine::MemoryEngine::new();

        put(&engine, "opencode", agent_item("acp", "opencode", &["acp"]))
            .await
            .expect("valid acp put");
        let rows = engine.load_agents().await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "opencode");
        assert_eq!(rows[0].driver, "acp");
        assert_eq!(rows[0].source, "ui");

        // 同 id 覆盖：定义更新、created_at 保留。
        let created_at = rows[0].created_at;
        put(
            &engine,
            "opencode",
            serde_json::json!({"driver": "acp", "path": "oc", "display": "Renamed"}),
        )
        .await
        .expect("update overwrites");
        let rows = engine.load_agents().await.unwrap();
        assert_eq!(rows.len(), 1, "同 id upsert 不产生第二行");
        assert_eq!(rows[0].display.as_deref(), Some("Renamed"));
        assert_eq!(rows[0].args, None, "整体替换：旧 args 不残留");
        assert_eq!(rows[0].created_at, created_at, "创建时刻保留");

        // 非法 payload 家族：全部拒绝且不落行。
        assert!(
            put(&engine, "native", agent_item("acp", "x", &[]))
                .await
                .unwrap_err()
                .contains("native"),
            "保留 id 拒绝写"
        );
        assert!(put(&engine, "g", agent_item("gemini", "x", &[])).await.is_err());
        assert!(put(&engine, "u", serde_json::json!({"driver": "acp"})).await.is_err());
        assert!(
            put(
                &engine,
                "k",
                serde_json::json!({"driver": "claude", "stale": 1})
            )
            .await
            .is_err(),
            "未知字段拒绝"
        );
        assert!(engine.load_agents().await.unwrap().len() == 1, "拒绝不落行");
    }

    /// delete → 行消失，且引用该 id 的项目默认被清（删除守卫）；未知 id
    /// 与 native 保留 id 的 delete 如实拒绝。
    #[tokio::test]
    async fn agents_mutation_delete_clears_project_defaults() {
        use sebas_models::project::project_id_for_on;
        let engine = crate::test_engine::MemoryEngine::new();
        engine.add_project("local", "/tmp/p1", "p1", 1).await.unwrap();
        engine
            .set_project_default_agent(&project_id_for_on("local", "/tmp/p1"), "opencode")
            .await
            .unwrap();
        agents_mutation(
            &engine,
            &serde_json::json!({
                "op": "put",
                "id": "opencode",
                "agent": agent_item("acp", "opencode", &["acp"]),
            }),
        )
        .await
        .unwrap();

        agents_mutation(
            &engine,
            &serde_json::json!({"op": "delete", "id": "opencode"}),
        )
        .await
        .expect("delete existing");
        assert!(engine.load_agents().await.unwrap().is_empty(), "行已消失");
        let projects = engine.load_projects().await.unwrap();
        assert_eq!(
            projects[0].default_agent, None,
            "项目默认被删除守卫清除"
        );

        // 未知 id / native 保留 id 的 delete 如实拒绝。
        let err = agents_mutation(
            &engine,
            &serde_json::json!({"op": "delete", "id": "ghost"}),
        )
        .await
        .unwrap_err();
        assert!(err.contains("ghost") && err.contains("不存在"), "{err}");
        assert!(
            agents_mutation(&engine, &serde_json::json!({"op": "delete", "id": "native"}))
                .await
                .is_err()
        );
    }

    /// 未知 op / 缺 op 的 payload 报错（domain 前缀点名）。
    #[tokio::test]
    async fn agents_mutation_rejects_unknown_ops() {
        let engine = crate::test_engine::MemoryEngine::new();
        let err = agents_mutation(&engine, &serde_json::json!({"op": "truncate"}))
            .await
            .unwrap_err();
        assert!(err.contains("agents: ") && err.contains("truncate"), "{err}");
        assert!(agents_mutation(&engine, &serde_json::json!({})).await.is_err());
    }
}
