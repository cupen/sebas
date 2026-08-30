//! 通用 CRUD 表单状态机：给定 [`FormSpec`] + [`CrudStore`]，把卡片回调
//! （按钮点击 / 表单提交）翻译成 [`Out`] 指令，实现 列表 / 新增 / 编辑 /
//! 删除 的最小闭环。不依赖任何业务实体——`Item` 是 serde_json 对象，
//! 字段由 `FormSpec` 与 `id_field` 决定，存储通过 trait 注入
//! （参考实现：[`InMemoryStore`]）。
//!
//! ## 回调负载协议（按钮 `behaviors[].value` / 表单提交的 `action.value`）
//!
//! - `{"form": <form_name>, "op": "create"}` —— 打开空表单
//! - `{"form": <form_name>, "op": "edit", "id": <id>}` —— 打开预填表单
//! - `{"form": <form_name>, "op": "submit"[, "id": <id>]}` —— 提交表单；
//!   携带 `id` 且存在则更新，否则新增（模块生成 id）
//! - `{"form": <form_name>, "op": "delete", "id": <id>}` —— 删除并回到列表
//! - `{"form": <form_name>, "op": "cancel"}` —— 回到列表
//!
//! 所有交互都优先原地更新触发回调的那张卡片（`context.open_message_id`），
//! 拿不到 message_id 时才退回新发一张卡片。

use crate::router::Out;
use sebas_feishu::cards::{Card, CardButton, CardElement, CardText};
use sebas_feishu::events::SessionKey;
// `SelectOption` 仅在 #[cfg(test)] 的 provider_spec helper 里用到，lib 构建
// 会触发 unused_imports warning，故显式 allow。
#[allow(unused_imports)]
use sebas_feishu::forms::{FormField, FormSpec, SelectOption, render_form_card, values_to_strings};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// 一条记录：字段名 -> 值（必须含 `id_field` 指向的 id）。
pub type Item = Map<String, Value>;

pub const OP_CREATE: &str = "create";
pub const OP_EDIT: &str = "edit";
pub const OP_SUBMIT: &str = "submit";
pub const OP_DELETE: &str = "delete";
pub const OP_CANCEL: &str = "cancel";
/// 交互式表单字段（带 `behaviors` 的 select）在用户切换选项时触发的
/// 重算回调：跑 normalizer 把派生字段写回表单预填值，但不写存储。
/// （提交还是走 OP_SUBMIT。）
pub const OP_RECOMPUTE: &str = "recompute";
/// 「获取模型列表」按钮：用当前表单值调外部接口取数，回填到指定文本
/// 字段的预填值后重渲表单。不写存储（提交才落盘）。
pub const OP_FETCH_MODELS: &str = "fetch-models";

const KEY_FORM: &str = "form";
const KEY_OP: &str = "op";
const KEY_ID: &str = "id";

/// 提交规范化钩子：对表单提交产生的 item 做字段级修正
/// （如 provider preset 默认值填充），在写入存储之前执行。
pub type ItemNormalizer = Arc<dyn Fn(&mut Item) + Send + Sync>;

/// 「获取模型列表」按钮的取数钩子：输入是当前表单值（已过 normalizer，
/// 含派生字段）拼出的 item，输出是字符串列表或单行错误原因。
/// 结果只回填表单预填值，不触碰存储。
pub type ModelFetcher = Arc<
    dyn Fn(
            Item,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<String>, String>> + Send>,
        > + Send
        + Sync,
>;

/// 存储抽象：CRUD 状态机只依赖这五个操作。
pub trait CrudStore: Send + Sync {
    fn list(&self) -> impl std::future::Future<Output = Vec<Item>> + Send;
    fn get(&self, id: &str) -> impl std::future::Future<Output = Option<Item>> + Send;
    fn insert(&self, item: Item) -> impl std::future::Future<Output = Result<(), String>> + Send;
    fn update(&self, item: Item) -> impl std::future::Future<Output = Result<(), String>> + Send;
    fn delete(&self, id: &str) -> impl std::future::Future<Output = Result<(), String>> + Send;
}

/// 进程内内存存储：重启即失，作为参考实现与测试替身。
/// 需要持久化的场景实现 [`CrudStore`] 后注入即可（如文件/数据库存储）。
#[derive(Clone)]
pub struct InMemoryStore {
    id_field: String,
    items: Arc<Mutex<Vec<Item>>>,
}

impl InMemoryStore {
    pub fn new(id_field: impl Into<String>) -> Self {
        Self {
            id_field: id_field.into(),
            items: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn with_items(id_field: impl Into<String>, items: Vec<Item>) -> Self {
        Self {
            id_field: id_field.into(),
            items: Arc::new(Mutex::new(items)),
        }
    }
}

impl CrudStore for InMemoryStore {
    async fn list(&self) -> Vec<Item> {
        self.items.lock().await.clone()
    }

    async fn get(&self, id: &str) -> Option<Item> {
        self.items
            .lock()
            .await
            .iter()
            .find(|i| i.get(&self.id_field).and_then(Value::as_str) == Some(id))
            .cloned()
    }

    async fn insert(&self, item: Item) -> Result<(), String> {
        self.items.lock().await.push(item);
        Ok(())
    }

    async fn update(&self, item: Item) -> Result<(), String> {
        let id = item
            .get(&self.id_field)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("item 缺少 id 字段 '{}'", self.id_field))?
            .to_string();
        let mut items = self.items.lock().await;
        let slot = items
            .iter_mut()
            .find(|i| i.get(&self.id_field).and_then(Value::as_str) == Some(id.as_str()));
        match slot {
            Some(slot) => {
                *slot = item;
                Ok(())
            }
            None => Err(format!("记录不存在: {id}")),
        }
    }

    async fn delete(&self, id: &str) -> Result<(), String> {
        let mut items = self.items.lock().await;
        let before = items.len();
        items.retain(|i| i.get(&self.id_field).and_then(Value::as_str) != Some(id));
        if items.len() == before {
            Err(format!("记录不存在: {id}"))
        } else {
            Ok(())
        }
    }
}

/// 文件存储：把 CRUD 变更以 delta 形式持久化，与只读种子
/// （如 config.toml 里的条目）合并后得到最终视图。
///
/// openspec/specs/provider-management/spec.md.8：自 v2 schema 起，所有 provider CRUD 变更与
/// runtime state（mode / default_selection）统一写入
/// `~/.sebas/state.json`（详见 `crate::state_store`）。本类型保留
/// `load(path, id_field, seed)` 旧 API（`path` 仅作为历史 hint 保留）
/// —— 真实持久化委托给 `state_store::update` —— 实现「删 default
/// provider」操作的单原子写：providers + deleted + mode + default
/// 全部走同一个文件、一次写。
///
/// 文件格式（v2 state.json `providers` / `deleted` 字段）：
/// ```json
/// {
///   "providers": { "<id>": { ...字段... } },
///   "deleted": ["<id>", ...]
/// }
/// ```
/// - `providers`：新增/修改过的条目（覆盖种子）；
/// - `deleted`：从种子删除的名字（墓碑，防止重启后从只读源复活）。
///
/// 旧 overlay 文件（`~/.sebas/providers.json`）由 `state_store` 一次性
/// 迁移到 `state.json` 后删除（见 docs/design-history.md ADR-4）；本类型不再写它。
#[derive(Clone)]
pub struct FileStore {
    /// **保留用于向后兼容（签名不变），写入时忽略** —— 持久化统一走
    /// `state.json`。`state_store::providers_path()`（`SEBAS_GATEWAY_PROVIDER_OVERLAY`
    /// 或默认 `~/.sebas/providers.json`）仍由 `state_store::load()` 在
    /// 首次迁移路径上读取，本类型不再触达。
    #[allow(dead_code)]
    path: PathBuf,
    id_field: String,
    state: Arc<Mutex<FileState>>,
}

#[derive(Default)]
struct FileState {
    /// 合并后的最终视图（种子 + 变更）。
    items: Vec<Item>,
    /// 新增/修改项：<id> -> item（来自 `state.json.providers`）。
    overrides: BTreeMap<String, Item>,
    /// 删除墓碑：从种子/配置中删除的名字（来自 `state.json.deleted`）。
    deleted: Vec<String>,
}

impl FileStore {
    /// `seed` 是只读源（config.toml）里已有的条目；持久化的变更从
    /// `state_store::load()` 读取（自动处理 `providers.json` →
    /// `state.json` 一次性迁移）。
    ///
    /// `path` 参数**仅保留签名兼容**，不读取也不写入 —— 持久化统一
    /// 走 `state.json`（`SEBAS_STATE_FILE` 或默认
    /// `~/.sebas/state.json`）。`state_store` 内部仍会按需读取
    /// 旧 overlay 文件做一次性迁移。
    pub fn load(
        path: impl Into<PathBuf>,
        id_field: impl Into<String>,
        seed: Vec<Item>,
    ) -> Result<Self, String> {
        let path = path.into();
        let id_field = id_field.into();
        // 从统一 store 读当前 overrides / deleted。state_store 内部
        // 处理 v0/v1 state.json + legacy providers.json 的合并迁移
        // （背景见 docs/design-history.md ADR-4）。
        let persisted = crate::state_store::load();
        let mut state = FileState {
            items: seed,
            overrides: persisted.providers,
            deleted: persisted.deleted,
        };
        state
            .items
            .retain(|i| !state.deleted.contains(&id_of(i, &id_field).to_string()));
        for item in state.overrides.values() {
            upsert_item(&mut state.items, item.clone(), &id_field);
        }
        Ok(Self {
            path,
            id_field,
            state: Arc::new(Mutex::new(state)),
        })
    }

    /// 写入到统一 `state.json`（openspec/specs/provider-management/spec.md）：tmp + rename
    /// 原子写，由 `state_store::update` 完成。**不再触碰 `self.path`**。
    async fn persist(&self, state: &FileState) -> Result<(), String> {
        let overrides = state.overrides.clone();
        let deleted = state.deleted.clone();
        crate::state_store::update(|s| {
            s.providers = overrides;
            s.deleted = deleted;
        })
        .map_err(|e| format!("state_store update 失败: {e}"))?;
        Ok(())
    }
}

impl CrudStore for FileStore {
    fn list(&self) -> impl std::future::Future<Output = Vec<Item>> + Send {
        let this = self.clone();
        async move { this.state.lock().await.items.clone() }
    }

    fn get(&self, id: &str) -> impl std::future::Future<Output = Option<Item>> + Send {
        let this = self.clone();
        let id = id.to_string();
        async move {
            this.state
                .lock()
                .await
                .items
                .iter()
                .find(|i| id_of(i, &this.id_field) == id)
                .cloned()
        }
    }

    fn insert(&self, item: Item) -> impl std::future::Future<Output = Result<(), String>> + Send {
        let this = self.clone();
        async move {
            let mut state = this.state.lock().await;
            let id = id_of(&item, &this.id_field).to_string();
            state.overrides.insert(id.clone(), item.clone());
            state.deleted.retain(|d| d != &id);
            upsert_item(&mut state.items, item, &this.id_field);
            this.persist(&state).await
        }
    }

    fn update(&self, item: Item) -> impl std::future::Future<Output = Result<(), String>> + Send {
        // 与 insert 同语义：按 id 覆盖（新增或修改统一走 overrides）。
        self.insert(item)
    }

    fn delete(&self, id: &str) -> impl std::future::Future<Output = Result<(), String>> + Send {
        let this = self.clone();
        let id = id.to_string();
        async move {
            let mut state = this.state.lock().await;
            state.items.retain(|i| id_of(i, &this.id_field) != id);
            state.overrides.remove(&id);
            if !state.deleted.contains(&id) {
                state.deleted.push(id.clone());
            }
            this.persist(&state).await
        }
    }
}

/// 一个实体的 CRUD 表单实例：schema（[`FormSpec`]）+ 存储（[`CrudStore`]）。
pub struct CrudForm<S: CrudStore> {
    pub spec: FormSpec,
    pub id_field: String,
    pub store: S,
    /// 提交时的字段规范化钩子（如 provider preset 默认值填充）。
    /// 在 item_from_form 之后、写入存储之前执行。
    normalizer: Option<ItemNormalizer>,
    /// 「获取模型列表」按钮：目标字段名 + 取数钩子。设置后编辑/新建表单
    /// 底部会多一个按钮，点击后用当前表单值取数并回填该字段预填值。
    model_fetcher: Option<(String, ModelFetcher)>,
}

/// 两套共享同一存储的 CRUD 表单，用于「同一个实体有两种入口」的场景
/// （如 provider：「预设」只填 name+key、「自定义」手填全部字段）。
pub struct ProviderForms {
    pub preset: Arc<CrudForm<FileStore>>,
    pub custom: Arc<CrudForm<FileStore>>,
}

impl ProviderForms {
    /// 按 `form_name` 把回调路由到对应表单；都不匹配则 `None`。
    pub fn dispatch(&self, form_name: &str) -> Option<&Arc<CrudForm<FileStore>>> {
        if form_name == self.preset.spec.form_name {
            Some(&self.preset)
        } else if form_name == self.custom.spec.form_name {
            Some(&self.custom)
        } else {
            None
        }
    }

    /// 「取消」按钮专用：从任意表单回到 ProviderForms 的双入口列表卡。
    /// 与单表单 `render_list_card` 不同——保留「＋ 新增（预设/自定义）」
    /// 两个按钮以及双 form 调度。
    pub async fn cancel(&self, key: SessionKey, message_id: Option<String>) -> crate::router::Out {
        self.handle_cancel(key, message_id).await
    }

    async fn handle_cancel(
        &self,
        key: SessionKey,
        message_id: Option<String>,
    ) -> crate::router::Out {
        let card = self.build_list_card().await;
        let card_value = serde_json::to_value(&card).expect("provider list card serializes");
        match message_id {
            Some(msg_id) => crate::router::Out::UpdateCardByMsgId {
                key,
                msg_id,
                card: card_value,
            },
            None => crate::router::Out::SendCard {
                key,
                card: card_value,
                msg_id: None,
                perm_request_id: None,
                perm_meta: None,
                root_id: None,
            },
        }
    }

    async fn build_list_card(&self) -> Card {
        let items = self.preset.store.list().await;
        let spec = &self.preset.spec;
        let mut card = Card::new(&format!("{}列表", spec.title), &spec.template);
        if items.is_empty() {
            card.push_text("暂无记录");
        } else {
            card.push_text(format!("共 {} 条", items.len()));
        }
        card.push_divider();
        for item in &items {
            let id = item
                .get(&self.preset.id_field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let has_preset = item
                .get("preset")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty());
            let row_spec = if has_preset {
                &self.preset.spec
            } else {
                &self.custom.spec
            };
            let lines: Vec<String> = row_spec
                .fields
                .iter()
                .filter_map(|f| {
                    item.get(f.name())
                        .map(|v| format!("**{}**：{}", f.label(), field_display(f, v)))
                })
                .collect();
            if !lines.is_empty() {
                card.push_text(lines.join("\n"));
            }
            let edit_form = if has_preset {
                &self.preset
            } else {
                &self.custom
            };
            card.push_actions(vec![
                CardButton {
                    text: CardText {
                        tag: "plain_text".into(),
                        content: "编辑".into(),
                    },
                    r#type: "default".into(),
                    value: payload(edit_form.spec.form_name.as_str(), OP_EDIT, Some(&id)),
                },
                CardButton {
                    text: CardText {
                        tag: "plain_text".into(),
                        content: "删除".into(),
                    },
                    r#type: "danger".into(),
                    value: payload(edit_form.spec.form_name.as_str(), OP_DELETE, Some(&id)),
                },
            ]);
            card.push_divider();
        }
        card.push_actions(vec![
            CardButton {
                text: CardText {
                    tag: "plain_text".into(),
                    content: "＋ 新增（预设）".into(),
                },
                r#type: "primary".into(),
                value: payload(self.preset.spec.form_name.as_str(), OP_CREATE, None),
            },
            CardButton {
                text: CardText {
                    tag: "plain_text".into(),
                    content: "＋ 新增（自定义）".into(),
                },
                r#type: "default".into(),
                value: payload(self.custom.spec.form_name.as_str(), OP_CREATE, None),
            },
        ]);
        card
    }

    /// 编辑时按 item 的 preset 字段判定走哪张表单：有 preset 走 preset
    /// 表单，否则走 custom 表单。item 缺失则 None（由调用方决定兜底）。
    pub async fn pick_for_edit(&self, id: &str) -> Option<Arc<CrudForm<FileStore>>> {
        let item = self.preset.store.get(id).await?;
        if item
            .get("preset")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty())
        {
            Some(self.preset.clone())
        } else {
            Some(self.custom.clone())
        }
    }

    /// `/provider` 命令入口：拉任意一张表单的存储列表（两张共享同一 store），
    /// 渲染一张带两个「＋ 新增」按钮的列表卡。
    pub async fn open(&self, key: SessionKey) -> crate::router::Out {
        self.render_list_card(key).await
    }

    async fn render_list_card(&self, key: SessionKey) -> crate::router::Out {
        let card = self.build_list_card().await;
        // 列表卡没有 context open_message_id：新发一张。
        crate::router::Out::SendCard {
            key,
            card: serde_json::to_value(&card).expect("provider list card serializes"),
            msg_id: None,
            perm_request_id: None,
            perm_meta: None,
            root_id: None,
        }
    }
}

fn payload(form_name: &str, op: &str, id: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert(KEY_FORM.into(), Value::String(form_name.into()));
    m.insert(KEY_OP.into(), Value::String(op.into()));
    if let Some(id) = id {
        m.insert(KEY_ID.into(), Value::String(id.into()));
    }
    Value::Object(m)
}

impl<S: CrudStore> CrudForm<S> {
    pub fn new(spec: FormSpec, id_field: impl Into<String>, store: S) -> Self {
        Self {
            spec,
            id_field: id_field.into(),
            store,
            normalizer: None,
            model_fetcher: None,
        }
    }

    /// 注入提交规范化钩子（对每次表单提交的 item 生效，种子数据不经过它）。
    pub fn with_normalizer(mut self, f: ItemNormalizer) -> Self {
        self.normalizer = Some(f);
        self
    }

    /// 注入「获取模型列表」按钮：`field` 是结果要回填的文本字段名，
    /// `fetcher` 用当前表单 item 取数。结果只进表单预填值，不写存储。
    pub fn with_model_fetcher(mut self, field: impl Into<String>, fetcher: ModelFetcher) -> Self {
        self.model_fetcher = Some((field.into(), fetcher));
        self
    }

    /// 打开 CRUD：发送列表卡（含「＋ 新增」和每条记录的 编辑/删除）。
    pub async fn open(&self, key: SessionKey) -> Out {
        self.reply(key, self.render_list_card().await, None)
    }

    /// 表单名（回调负载里的 `form` 字段），用于路由到具体实例。
    pub fn form_name(&self) -> &str {
        &self.spec.form_name
    }

    /// 处理一次卡片回调（按钮点击或表单提交），返回要执行的 [`Out`]。
    /// 回调负载见模块文档；表单字段值走 `form_value`（组件 `name` -> 值）。
    pub async fn handle(
        &self,
        key: SessionKey,
        value: &Value,
        form_value: &BTreeMap<String, Value>,
        message_id: Option<String>,
    ) -> Out {
        // 不是本表单的负载（如权限卡的 value）——安全兜底回列表，不误伤。
        if value.get(KEY_FORM).and_then(Value::as_str) != Some(self.spec.form_name.as_str()) {
            return self.reply(key, self.render_list_card().await, message_id);
        }

        let op = value
            .get(KEY_OP)
            .and_then(Value::as_str)
            .unwrap_or_default();
        match op {
            OP_CREATE => {
                let card = self.render_edit_card(&BTreeMap::new(), None);
                self.reply(key, card, message_id)
            }
            OP_EDIT => {
                let id = value
                    .get(KEY_ID)
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let item = self.store.get(id).await;
                let initial = item
                    .as_ref()
                    .map(|it| self.item_to_initial(it))
                    .unwrap_or_default();
                self.reply(key, self.render_edit_card(&initial, Some(id)), message_id)
            }
            OP_SUBMIT => {
                let id = value.get(KEY_ID).and_then(Value::as_str);
                self.apply_submit(id, form_value).await;
                self.reply(key, self.render_list_card().await, message_id)
            }
            OP_RECOMPUTE => {
                let id = value.get(KEY_ID).and_then(Value::as_str);
                let initial = self.recompute_initial(id, form_value).await;
                let card = self.render_edit_card(&initial, id);
                self.reply(key, card, message_id)
            }
            OP_FETCH_MODELS => {
                let id = value.get(KEY_ID).and_then(Value::as_str);
                // 与 recompute 同款拼 item：当前表单值 + 存储旧值兜底敏感
                // 字段 + normalizer 派生字段（preset 补全 base_url 等）。
                let existing = match id {
                    Some(id) => self.store.get(id).await,
                    None => None,
                };
                let mut item = self.item_from_form(id, form_value, existing.as_ref());
                if let Some(f) = &self.normalizer {
                    f(&mut item);
                }
                let mut initial = self.item_to_initial(&item);
                let note = match &self.model_fetcher {
                    Some((field, fetch)) => match fetch(item).await {
                        Ok(models) if !models.is_empty() => {
                            initial.insert(field.clone(), models.join(","));
                            format!(
                                "✅ 已获取 {} 个模型并填入下方字段；确认后点「提交」保存（不提交不落盘）。",
                                models.len()
                            )
                        }
                        Ok(_) => "⚠️ 接口返回空列表，请手填。".to_string(),
                        Err(e) => format!("⚠️ 获取失败：{e}。请手填。"),
                    },
                    None => "⚠️ 该表单未配置模型获取。".to_string(),
                };
                let mut card = self.render_edit_card(&initial, id);
                // 结果提示放表单容器上方，重渲后第一眼可见。
                card.body
                    .elements
                    .insert(0, CardElement::Markdown { content: note });
                self.reply(key, card, message_id)
            }
            OP_DELETE => {
                if let Some(id) = value.get(KEY_ID).and_then(Value::as_str)
                    && let Err(e) = self.store.delete(id).await
                {
                    tracing::warn!(form = %self.spec.form_name, id, error = %e, "crud delete failed");
                }
                self.reply(key, self.render_list_card().await, message_id)
            }
            // cancel 由表单容器的 reset 按钮触发；未知 op 也回列表。
            _ => self.reply(key, self.render_list_card().await, message_id),
        }
    }

    async fn apply_submit(&self, id: Option<&str>, form_value: &BTreeMap<String, Value>) {
        match id {
            Some(id) => {
                let existing = self.store.get(id).await;
                let exists = existing.is_some();
                let mut item = self.item_from_form(Some(id), form_value, existing.as_ref());
                if let Some(f) = &self.normalizer {
                    f(&mut item);
                }
                let result = if exists {
                    self.store.update(item).await
                } else {
                    self.store.insert(item).await
                };
                if let Err(e) = result {
                    tracing::warn!(form = %self.spec.form_name, id, error = %e, "crud submit failed");
                }
            }
            None => {
                let id = format!("{}-{}", self.spec.form_name, now_unix_millis());
                let mut item = self.item_from_form(Some(id.as_str()), form_value, None);
                if let Some(f) = &self.normalizer {
                    f(&mut item);
                }
                if let Err(e) = self.store.insert(item).await {
                    tracing::warn!(form = %self.spec.form_name, id, error = %e, "crud insert failed");
                }
            }
        }
    }

    /// 用户在表单里切换了某个交互式字段（带 `on_change` 的 select）：
    /// 把当前表单值按 normalizer 跑一遍得到派生字段，再转成 initial
    /// 让编辑卡片就地重渲。注意：不写存储——提交还是要点提交按钮。
    async fn recompute_initial(
        &self,
        id: Option<&str>,
        form_value: &BTreeMap<String, Value>,
    ) -> BTreeMap<String, String> {
        // 编辑场景下用存储里的旧 item 来兜底敏感字段（避免把已有密钥抹掉）。
        let existing = match id {
            Some(id) => self.store.get(id).await,
            None => None,
        };
        let mut item = self.item_from_form(id, form_value, existing.as_ref());
        if let Some(f) = &self.normalizer {
            f(&mut item);
        }
        self.item_to_initial(&item)
    }

    fn item_from_form(
        &self,
        id: Option<&str>,
        form_value: &BTreeMap<String, Value>,
        existing: Option<&Item>,
    ) -> Item {
        let mut item = Map::new();
        if let Some(id) = id {
            item.insert(self.id_field.clone(), Value::String(id.to_string()));
        }
        let values = values_to_strings(form_value);
        for f in &self.spec.fields {
            let submitted = values.get(f.name());
            if f.is_secret() {
                // 敏感字段：留空保留旧值（编辑场景），避免误清空密钥；
                // 新增场景留空则不写入。
                if let Some(v) = submitted
                    && !v.is_empty()
                {
                    item.insert(f.name().to_string(), Value::String(v.clone()));
                } else if let Some(old) = existing.and_then(|e| e.get(f.name())) {
                    item.insert(f.name().to_string(), old.clone());
                }
            } else if let Some(v) = submitted {
                item.insert(f.name().to_string(), Value::String(v.clone()));
            }
        }
        item
    }

    /// 编辑表单的预填值：敏感字段不预填（避免把密钥回显到卡片）。
    fn item_to_initial(&self, item: &Item) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for f in &self.spec.fields {
            if f.is_secret() {
                continue;
            }
            if let Some(v) = item.get(f.name()) {
                out.insert(f.name().to_string(), scalar_display(v));
            }
        }
        out
    }

    fn render_edit_card(&self, initial: &BTreeMap<String, String>, id: Option<&str>) -> Card {
        let mut submit = Map::new();
        submit.insert(KEY_FORM.into(), Value::String(self.spec.form_name.clone()));
        submit.insert(KEY_OP.into(), Value::String(OP_SUBMIT.into()));
        if let Some(id) = id {
            submit.insert(KEY_ID.into(), Value::String(id.to_string()));
        }
        let mut card = render_form_card(&self.spec, initial, Value::Object(submit));
        // 「获取模型列表」（可选）与「取消/返回列表」都必须是表单容器外的
        // 普通回调按钮：容器内的 reset 按钮不允许带 behaviors（飞书 API
        // 11310），无法驱动服务端状态。获取按钮在容器外也能拿到整张卡的
        // form_value（含容器内各字段当前值），所以新建未提交也能取数。
        let mut actions: Vec<CardButton> = Vec::new();
        if self.model_fetcher.is_some() {
            actions.push(CardButton {
                text: CardText {
                    tag: "plain_text".into(),
                    content: "🔍 获取模型列表".into(),
                },
                r#type: "default".into(),
                value: self.payload(OP_FETCH_MODELS, id),
            });
        }
        actions.push(CardButton {
            text: CardText {
                tag: "plain_text".into(),
                content: "取消".into(),
            },
            r#type: "default".into(),
            value: self.payload(OP_CANCEL, None),
        });
        card.push_actions(actions);
        card
    }

    async fn render_list_card(&self) -> Card {
        let items = self.store.list().await;
        let mut card = Card::new(&format!("{}列表", self.spec.title), &self.spec.template);
        if items.is_empty() {
            card.push_text("暂无记录");
        } else {
            card.push_text(format!("共 {} 条", items.len()));
        }
        card.push_divider();
        for item in items {
            let id = item
                .get(&self.id_field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let lines: Vec<String> = self
                .spec
                .fields
                .iter()
                .filter_map(|f| {
                    item.get(f.name())
                        .map(|v| format!("**{}**：{}", f.label(), field_display(f, v)))
                })
                .collect();
            if !lines.is_empty() {
                card.push_text(lines.join("\n"));
            }
            card.push_actions(vec![
                CardButton {
                    text: CardText {
                        tag: "plain_text".into(),
                        content: "编辑".into(),
                    },
                    r#type: "default".into(),
                    value: self.payload(OP_EDIT, Some(&id)),
                },
                CardButton {
                    text: CardText {
                        tag: "plain_text".into(),
                        content: "删除".into(),
                    },
                    r#type: "danger".into(),
                    value: self.payload(OP_DELETE, Some(&id)),
                },
            ]);
            card.push_divider();
        }
        card.push_actions(vec![CardButton {
            text: CardText {
                tag: "plain_text".into(),
                content: "＋ 新增".into(),
            },
            r#type: "primary".into(),
            value: self.payload(OP_CREATE, None),
        }]);
        card
    }

    fn payload(&self, op: &str, id: Option<&str>) -> Value {
        let mut m = Map::new();
        m.insert(KEY_FORM.into(), Value::String(self.spec.form_name.clone()));
        m.insert(KEY_OP.into(), Value::String(op.into()));
        if let Some(id) = id {
            m.insert(KEY_ID.into(), Value::String(id.into()));
        }
        Value::Object(m)
    }

    fn reply(&self, key: SessionKey, card: Card, message_id: Option<String>) -> Out {
        let card = serde_json::to_value(&card).expect("crud card serializes");
        match message_id {
            Some(msg_id) => Out::UpdateCardByMsgId { key, msg_id, card },
            None => Out::SendCard {
                key,
                card,
                msg_id: None,
                perm_request_id: None,
                perm_meta: None,
                root_id: None,
            },
        }
    }
}

fn field_display(f: &FormField, v: &Value) -> String {
    if f.is_secret() {
        return match v {
            Value::String(s) if !s.is_empty() => "••••••".to_string(),
            _ => "未设置".to_string(),
        };
    }
    scalar_display(v)
}

fn scalar_display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "—".into(),
        other => other.to_string(),
    }
}

fn now_unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn id_of<'a>(item: &'a Item, id_field: &str) -> &'a str {
    item.get(id_field)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn upsert_item(items: &mut Vec<Item>, item: Item, id_field: &str) {
    let id = id_of(&item, id_field).to_string();
    if let Some(slot) = items.iter_mut().find(|i| id_of(i, id_field) == id) {
        *slot = item;
    } else {
        items.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::lock_state_file;

    // openspec/specs/provider-management/spec.md：FileStore 持久化到 unified state.json（路径由
    // SEBAS_STATE_FILE 决定）。所有写盘的测试都要先把 SEBAS_STATE_FILE 指
    // 向 tempdir，避免污染开发机 ~/.sebas/state.json，并避免互相覆盖。
    // 全局 mutex 串行化 env 访问。

    fn item(id: &str, protocol: &str) -> Item {
        let mut m = Map::new();
        m.insert("name".into(), Value::String(id.into()));
        m.insert(
            "base_url_anthropic".into(),
            Value::String(format!("https://{id}.example")),
        );
        let _ = protocol;
        m
    }

    /// 把 SEBAS_STATE_FILE 与 SEBAS_GATEWAY_PROVIDER_OVERLAY 都指向
    /// tempdir（state.json + providers.json），返回 state 路径。
    /// provider 数据已拆回 providers.json，两个 env 都必须隔离。
    fn isolate(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let path = dir.path().join("state.json");
        // SAFETY: ENV_LOCK held by caller.
        unsafe {
            std::env::set_var("SEBAS_STATE_FILE", path.to_str().unwrap());
            std::env::set_var(
                "SEBAS_GATEWAY_PROVIDER_OVERLAY",
                dir.path().join("providers.json").to_str().unwrap(),
            );
        }
        path
    }

    fn deisolate() {
        // SAFETY: ENV_LOCK held by caller.
        unsafe {
            std::env::remove_var("SEBAS_STATE_FILE");
            std::env::remove_var("SEBAS_GATEWAY_PROVIDER_OVERLAY");
        }
    }

    #[tokio::test]
    async fn load_without_file_uses_seed() {
        let _g = lock_state_file();
        let dir = tempfile::tempdir().unwrap();
        let _path = isolate(&dir);
        let store = FileStore::load(
            dir.path().join("providers.json"),
            "name",
            vec![item("deepseek", "anthropic")],
        )
        .unwrap();
        assert_eq!(store.list().await.len(), 1);
        deisolate();
    }

    #[tokio::test]
    async fn insert_persists_and_delete_tombstones() {
        let _g = lock_state_file();
        let dir = tempfile::tempdir().unwrap();
        let _path = isolate(&dir);
        let store = FileStore::load(
            dir.path().join("providers.json"),
            "name",
            vec![item("deepseek", "anthropic")],
        )
        .unwrap();

        // 新增：覆盖种子 + 落盘。
        store.insert(item("openai", "openai")).await.unwrap();
        assert_eq!(store.list().await.len(), 2);

        // 删除种子里已有的条目：写墓碑，重启后不复活。
        store.delete("deepseek").await.unwrap();
        assert_eq!(store.list().await.len(), 1);
        let reloaded = FileStore::load(
            dir.path().join("providers.json"),
            "name",
            vec![item("deepseek", "anthropic")],
        )
        .unwrap();
        assert_eq!(reloaded.list().await.len(), 1);
        assert_eq!(
            reloaded.list().await[0].get("name").and_then(Value::as_str),
            Some("openai")
        );
        deisolate();
    }

    #[tokio::test]
    async fn update_overrides_seed_value() {
        let _g = lock_state_file();
        let dir = tempfile::tempdir().unwrap();
        let _path = isolate(&dir);
        let mut seed = item("deepseek", "anthropic");
        seed.insert("base_url_anthropic".into(), Value::String("old".into()));
        let store = FileStore::load(dir.path().join("providers.json"), "name", vec![seed]).unwrap();

        let mut updated = item("deepseek", "openai");
        updated.insert("base_url_anthropic".into(), Value::String("new".into()));
        store.update(updated).await.unwrap();

        let reloaded = FileStore::load(
            dir.path().join("providers.json"),
            "name",
            vec![item("deepseek", "anthropic")],
        )
        .unwrap();
        let got = reloaded.get("deepseek").await.unwrap();
        assert_eq!(
            got.get("base_url_anthropic").and_then(Value::as_str),
            Some("new")
        );
        deisolate();
    }

    fn provider_spec() -> FormSpec {
        FormSpec::new(
            "provider",
            "Provider",
            vec![
                FormField::Text {
                    name: "name".into(),
                    label: "名称".into(),
                    required: true,
                    placeholder: String::new(),
                    secret: false,
                    disabled: false,
                },
                FormField::Select {
                    name: "preset".into(),
                    label: "预设".into(),
                    required: false,
                    options: vec![
                        SelectOption {
                            value: "".into(),
                            label: "无".into(),
                        },
                        SelectOption {
                            value: "deepseek".into(),
                            label: "deepseek".into(),
                        },
                    ],
                    on_change: Some(serde_json::json!({"form": "provider", "op": "recompute"})),
                },
                FormField::Text {
                    name: "base_url_anthropic".into(),
                    label: "Base URL(Anthropic)".into(),
                    required: false,
                    placeholder: String::new(),
                    secret: false,
                    disabled: false,
                },
                FormField::Text {
                    name: "api_key".into(),
                    label: "API Key".into(),
                    required: false,
                    placeholder: String::new(),
                    secret: true,
                    disabled: false,
                },
            ],
        )
    }

    #[tokio::test]
    async fn recompute_runs_normalizer_without_persisting() {
        let _g = lock_state_file();
        let dir = tempfile::tempdir().unwrap();
        let _path = isolate(&dir);
        let store = FileStore::load(dir.path().join("providers.json"), "name", Vec::new()).unwrap();
        let form = CrudForm::new(provider_spec(), "name", store.clone()).with_normalizer(Arc::new(
            |item: &mut Item| {
                // 与 apply_preset_defaults 等价的最小复刻：选 deepseek 时补全
                // base_url_anthropic/api_key_env，留空字段不覆盖用户输入。
                if item.get("preset").and_then(Value::as_str) == Some("deepseek") {
                    let anth_empty = item
                        .get("base_url_anthropic")
                        .and_then(Value::as_str)
                        .is_none_or(|s| s.is_empty());
                    if anth_empty {
                        item.insert(
                            "base_url_anthropic".into(),
                            Value::String("https://api.deepseek.com/anthropic".into()),
                        );
                    }
                    let has_env = item
                        .get("api_key_env")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.is_empty());
                    if !has_env {
                        item.insert(
                            "api_key_env".into(),
                            Value::String("DEEPSEEK_API_KEY".into()),
                        );
                    }
                }
            },
        ));

        // 模拟用户选 deepseek 后从客户端回传：preset=deepseek, base_url_anthropic="".
        let mut fv = BTreeMap::new();
        fv.insert("preset".into(), Value::String("deepseek".into()));
        fv.insert("name".into(), Value::String("ds".into()));
        fv.insert("base_url_anthropic".into(), Value::String("".into()));

        let initial = form.recompute_initial(None, &fv).await;
        // 派生字段被填回 initial，准备渲染。
        assert_eq!(
            initial.get("base_url_anthropic").map(String::as_str),
            Some("https://api.deepseek.com/anthropic")
        );

        // 关键：recompute 不应写入存储。
        assert_eq!(store.list().await.len(), 0);
        deisolate();
    }

    #[tokio::test]
    async fn recompute_preserves_existing_secret_on_edit() {
        // 编辑已有条目时，recompute 不能把已有的密钥抹掉。
        let _g = lock_state_file();
        let dir = tempfile::tempdir().unwrap();
        let _path = isolate(&dir);
        let mut seed = item("ds", "anthropic");
        seed.insert(
            "base_url_anthropic".into(),
            Value::String("https://old".into()),
        );
        seed.insert("api_key".into(), Value::String("sk-keep".into()));
        let store = FileStore::load(dir.path().join("providers.json"), "name", vec![seed]).unwrap();
        let form = CrudForm::new(provider_spec(), "name", store.clone()).with_normalizer(Arc::new(
            |item: &mut Item| {
                if item.get("preset").and_then(Value::as_str) == Some("deepseek") {
                    let base_empty = item
                        .get("base_url_anthropic")
                        .and_then(Value::as_str)
                        .is_none_or(|s| s.is_empty());
                    if base_empty {
                        item.insert(
                            "base_url_anthropic".into(),
                            Value::String("https://api.deepseek.com/anthropic".into()),
                        );
                    }
                }
            },
        ));

        // 模拟编辑场景：用户只切了 preset，没碰 api_key 字段。
        let mut fv = BTreeMap::new();
        fv.insert("name".into(), Value::String("ds".into()));
        fv.insert("preset".into(), Value::String("deepseek".into()));
        fv.insert("base_url_anthropic".into(), Value::String("".into()));
        fv.insert("api_key".into(), Value::String("".into()));

        let initial = form.recompute_initial(Some("ds"), &fv).await;
        // 关键断言是 secret 没出现在 initial 里——它本来也不该回显。
        assert!(initial.get("api_key").is_none());
        // base_url_anthropic 被 preset 默认值覆盖了（用户的意图：切 preset 就用新端点）。
        assert_eq!(
            initial.get("base_url_anthropic").map(String::as_str),
            Some("https://api.deepseek.com/anthropic")
        );
        // 存储原样不动。
        let stored = store.get("ds").await.unwrap();
        assert_eq!(
            stored.get("api_key").and_then(Value::as_str),
            Some("sk-keep"),
            "recompute 不能写入存储"
        );
        deisolate();
    }
}
