//! `/provider` 命令 + provider CRUD 表单的路由集成测试：经 DispatchHandle
//! 驱动 列表 → 新增 → 提交 → 删除，验证按钮/表单回调被正确路由且
//! 存储（FileStore 委托给 unified state.json，见 openspec/specs/provider-management/spec.md
//! 与 docs/design-history.md ADR-4）随变更持久化。

// ENV_LOCK 串行化 env 变更：每个 #[tokio::test] 独立 runtime，跨 await 持
// std 锁只会让其它测试等待，不构成死锁——这是刻意的。
#![allow(clippy::await_holding_lock)]

use sebas_channels::{ChannelAction, ChannelEvent, ChannelKey};
use sebas_channels::card::{FormField, FormSpec};
use sebas_dispatch::CrudStore;
use sebas_dispatch::Out;
use sebas_dispatch::crud::{CrudForm, FileStore, Item};
use sebas_dispatch::engine::DispatchHandle;
use sebas_dispatch::state::SessionMap;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

// provider 状态统一（docs/design-history.md ADR-4，行为契约见
// openspec/specs/provider-management/spec.md）：FileStore 持久化到 unified
// state.json（路径由 SEBAS_STATE_FILE 决定）。所有走 FileStore / state_store 的测试都要
// 把 SEBAS_STATE_FILE 指到 tempdir，避免污染开发机 ~/.sebas/state.json，
// 且避免同进程内测试互相覆盖。全局 mutex 串行化 env 访问。
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn isolate(dir: &tempfile::TempDir) {
    // provider 数据已拆回 providers.json（state_store 双文件），两个 env
    // 都要隔离，避免读到开发机真实文件。
    // SAFETY: ENV_LOCK held by caller.
    unsafe {
        std::env::set_var("SEBAS_STATE_FILE", dir.path().join("state.json").to_str().unwrap());
        std::env::set_var(
            "SEBAS_ROUTER_PROVIDER_OVERLAY",
            dir.path().join("providers.json").to_str().unwrap(),
        );
    }
}

fn deisolate() {
    // SAFETY: ENV_LOCK held by caller.
    unsafe {
        std::env::remove_var("SEBAS_STATE_FILE");
        std::env::remove_var("SEBAS_ROUTER_PROVIDER_OVERLAY");
    }
}

fn spec() -> FormSpec {
    FormSpec::new(
        "provider-custom",
        "Provider（自定义）",
        vec![
            FormField::Text {
                name: "name".into(),
                label: "名称".into(),
                required: true,
                placeholder: "".into(),
                secret: false,
                disabled: false,
            },
            FormField::Text {
                name: "base_url".into(),
                label: "Base URL".into(),
                required: true,
                placeholder: "".into(),
                secret: false,
                disabled: false,
            },
            FormField::Text {
                name: "api_key".into(),
                label: "API Key".into(),
                required: false,
                placeholder: "粘贴密钥".into(),
                secret: true,
                disabled: false,
            },
        ],
    )
}

fn key() -> ChannelKey { ChannelKey::feishu("oc_provider", None) }

fn item(name: &str) -> Item {
    let mut m = Map::new();
    m.insert("name".into(), Value::String(name.into()));
    m.insert(
        "base_url".into(),
        Value::String(format!("https://{name}.example")),
    );
    m
}

fn item_with_key(name: &str, key: &str) -> Item {
    let mut m = item(name);
    m.insert("api_key".into(), Value::String(key.into()));
    m
}

fn card_action(payload: Value, msg_id: &str) -> ChannelAction {
    ChannelAction {
        session_id: String::new(),
        request_id: None,
        decision: None,
        value: json!({
            "action": { "value": payload },
            "context": { "open_message_id": msg_id }
        }),
    }
}

fn provider_router(dir: &tempfile::TempDir) -> (DispatchHandle, tokio::sync::mpsc::Receiver<Out>) {
    let store = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item("deepseek")],
    )
    .unwrap();
    let form = CrudForm::new(spec(), "name", store.clone());
    let forms = sebas_dispatch::ProviderForms {
        preset: Arc::new(CrudForm::new(
            FormSpec::new("provider-preset", "Provider（预设）", vec![]),
            "name",
            store,
        )),
        custom: Arc::new(form),
    };
    DispatchHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(forms)),
        None,
    )
}

#[tokio::test]
async fn provider_command_opens_main_card_with_seed() {
    // bead sebas-63f.5：`/provider` 命令现在打开「Provider 管理」主卡
    // （mode + default-direct 下拉 + 列表下拉 + 新建子区/详情面板），
    // 取代了旧的「列表 + 每条 编辑/删除」双入口卡。
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate(&dir);
    let (router, mut rx) = provider_router(&dir);

    router
        .dispatch(ChannelEvent::Text { key: key(), text: "/provider".into(), reply_target: None })
        .await;

    let out = rx.recv().await.unwrap();
    match out {
        Out::SendCard { card, .. } => {
            let s = card.to_json();
            // 标题。
            assert!(s.contains("Provider 管理"), "{s}");
            // 种子的 provider 名出现在下拉里。
            assert!(s.contains("deepseek"), "{s}");
            // 新建子区按钮：「＋ 新增（预设）」与「＋ 新增（自定义）」。
            assert!(s.contains("＋ 新增（预设）"), "{s}");
            assert!(s.contains("＋ 新增（自定义）"), "{s}");
            // 新按钮 payload 用 form 名（provider-create-preset /
            // provider-create-custom）取代旧的 `{form, op: "create"}`。
            assert!(s.contains("\"form\":\"provider-create-preset\""), "{s}");
            assert!(s.contains("\"form\":\"provider-create-custom\""), "{s}");
            // 三个 mode 按钮。
            for m in ["off", "direct", "router"] {
                assert!(
                    s.contains(&format!("\"mode\":\"{m}\"")),
                    "应渲染 mode={m} 按钮：{s}"
                );
            }
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
    deisolate();
}

#[tokio::test]
async fn provider_create_submit_delete_round_trip() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate(&dir);
    let (router, mut rx) = provider_router(&dir);

    // 点「＋ 新增」→ 表单卡原地出现。
    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key(),
            action: card_action(json!({"form": "provider-custom", "op": "create"}), "om_1"),
                    })
        .await;
    let out = rx.recv().await.unwrap();
    let Out::UpdateCardByMsgId { msg_id, card, .. } = out else {
        panic!("expected UpdateCardByMsgId, got {out:?}");
    };
    assert_eq!(msg_id, "om_1");
    assert!(card.to_json().contains("\"tag\":\"form\""), "{card:?}");

    // 提交 → 列表卡原地更新，记录进 FileStore 并落盘。
    let mut fv = BTreeMap::new();
    fv.insert("name".into(), json!("openai"));
    fv.insert("base_url".into(), json!("https://api.openai.com"));
    router
        .dispatch(ChannelEvent::FormCb { key: key(), value: json!({"form": "provider-custom", "op": "submit"}), form_value: fv, card_ref: Some("om_1".into()) })
        .await;
    let out = rx.recv().await.unwrap();
    let Out::UpdateCardByMsgId { card, .. } = out else {
        panic!("expected UpdateCardByMsgId, got {out:?}");
    };
    assert!(card.to_json().contains("openai"), "{card:?}");

    // 删除种子里的 deepseek（写墓碑）。
    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key(),
            action: card_action(
                json!({"form": "provider-custom", "op": "delete", "id": "deepseek"}),
                "om_2",
            ),
                    })
        .await;
    let out = rx.recv().await.unwrap();
    assert!(matches!(out, Out::UpdateCardByMsgId { .. }), "{out:?}");

    // 重启视角：重新加载 unified state.json + 种子 → 只剩 openai。
    // 状态统一后（docs/design-history.md ADR-4）：FileStore::load 不再读
    // providers.json，直接走 state_store::load（SEBAS_STATE_FILE 已指到 dir/state.json）。
    let reloaded = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item("deepseek")],
    )
    .unwrap();
    let items = reloaded.list().await;
    assert_eq!(items.len(), 1, "deepseek 墓碑必须生效");
    assert_eq!(items[0].get("name").and_then(Value::as_str), Some("openai"));
    deisolate();
}

#[tokio::test]
async fn permission_shaped_button_is_not_routed_to_provider_crud() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate(&dir);
    let (router, mut rx) = provider_router(&dir);

    router
        .dispatch(ChannelEvent::ButtonCb { key: key(),
            action: ChannelAction {
                session_id: "s1".into(),
                request_id: Some("r1".into()),
                decision: Some("allow_once".into()),
                value: json!({
                    "action": { "value": {"session_id": "s1", "request_id": "r1", "decision": "allow_once"} },
                    "context": { "open_message_id": "om_perm" },
                    "chat_type": "p2p",
                }),
            },
                    })
        .await;

    // 没有活跃 ACP 会话 → 死会话卡（SendCard）；如果误路由到 CRUD 会是
    // UpdateCardByMsgId（带 om_perm），用这个区分。
    let out = rx.recv().await.unwrap();
    assert!(matches!(out, Out::SendCard { .. }), "{out:?}");
    deisolate();
}

#[tokio::test]
async fn cancel_button_returns_to_list_not_to_dead_session_card() {
    // 回归：表单容器外的「取消」按钮带 op=cancel，必须被 on_button 识别为
    // CRUD 路由并回列表卡；如果被吞掉就会落到 ACP session 死会话路径。
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate(&dir);
    let (router, mut rx) = provider_router(&dir);

    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key(),
            action: card_action(
                json!({"form": "provider-custom", "op": "cancel"}),
                "om_cancel",
            ),
                    })
        .await;

    let out = rx.recv().await.unwrap();
    match out {
        Out::UpdateCardByMsgId { msg_id, card, .. } => {
            assert_eq!(msg_id, "om_cancel", "cancel 应原地更新原表单卡");
            // 列表卡包含「＋ 新增（预设/自定义）」按钮，证明回到 CRUD 列表。
            let s = card.to_json();
            assert!(s.contains("＋ 新增（预设）"), "{s}");
            assert!(s.contains("＋ 新增（自定义）"), "{s}");
        }
        other => panic!("cancel 应回到 CRUD 列表卡，得到 {other:?}"),
    }
    deisolate();
}

#[tokio::test]
async fn secret_key_is_never_displayed_in_plaintext_in_main_card() {
    // bead sebas-63f.5：主卡详情面板只显示「API Key：已配置/未配置」两态，
    // 取代了旧列表卡的 `••••••` 掩码（一样防泄露，只是文案更简洁）。
    // 编辑表单的密钥不预填行为由既有 `CrudForm::item_to_initial()` 保证
    // （见下方的「编辑表单不预填密钥」半边）。
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate(&dir);
    let store = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item_with_key("deepseek", "sk-super-secret")],
    )
    .unwrap();
    let form = CrudForm::new(spec(), "name", store.clone());
    let forms = sebas_dispatch::ProviderForms {
        preset: Arc::new(CrudForm::new(
            FormSpec::new("provider-preset", "Provider（预设）", vec![]),
            "name",
            store,
        )),
        custom: Arc::new(form),
    };
    let (router, mut rx) = DispatchHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(forms)),
        None,
    );

    // 主卡：deepseek 的折叠面板里 API Key 行应是「已配置」，且永远不
    // 出现明文密钥（无论新旧设计）。
    router
        .dispatch(ChannelEvent::Text { key: key(), text: "/provider".into(), reply_target: None })
        .await;
    let out = rx.recv().await.unwrap();
    let Out::SendCard { card, .. } = out else {
        panic!("expected SendCard");
    };
    let s = card.to_json();
    assert!(s.contains("已配置"), "应展示 API Key：已配置：{s}");
    assert!(
        !s.contains("sk-super-secret"),
        "api_key 明文不应出现在主卡：{s}"
    );
}

#[tokio::test]
async fn edit_form_does_not_prefill_secret() {
    // 主卡详情面板里点「编辑」按钮 → 走既有 `provider-custom` 表单的
    // OP_EDIT 路径；表单的 `item_to_initial()` 应跳过 secret 字段，绝不
    // 把密钥回显到表单（沿用 63f.5 之前的契约）。
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate(&dir);
    let store = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item_with_key("deepseek", "sk-super-secret")],
    )
    .unwrap();
    let form = CrudForm::new(spec(), "name", store.clone());
    let forms = sebas_dispatch::ProviderForms {
        preset: Arc::new(CrudForm::new(
            FormSpec::new("provider-preset", "Provider（预设）", vec![]),
            "name",
            store,
        )),
        custom: Arc::new(form),
    };
    let (router, mut rx) = DispatchHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(forms)),
        None,
    );

    // 直接触发既有表单的编辑路径（不走新主卡的按钮，因为我们只想验证
    // 旧契约：编辑表单不预填密钥）。
    router
        .dispatch(ChannelEvent::ButtonCb {
            key: key(),
            action: card_action(
                json!({"form": "provider-custom", "op": "edit", "id": "deepseek"}),
                "om_e",
            ),
                    })
        .await;
    let out = rx.recv().await.unwrap();
    let Out::UpdateCardByMsgId { card, .. } = out else {
        panic!("expected UpdateCardByMsgId");
    };
    let s = card.to_json();
    assert!(
        !s.contains("sk-super-secret"),
        "edit form must not prefill secret: {s}"
    );
    deisolate();
}

#[tokio::test]
async fn empty_secret_submit_preserves_existing_key() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    isolate(&dir);
    let store = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item_with_key("deepseek", "sk-old")],
    )
    .unwrap();
    let form = CrudForm::new(spec(), "name", store.clone());
    let forms = sebas_dispatch::ProviderForms {
        preset: Arc::new(CrudForm::new(
            FormSpec::new("provider-preset", "Provider（预设）", vec![]),
            "name",
            store,
        )),
        custom: Arc::new(form),
    };
    let (router, mut rx) = DispatchHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(forms)),
        None,
    );

    // 编辑提交：api_key 留空（不提交），改 base_url。
    let mut fv = BTreeMap::new();
    fv.insert("name".into(), json!("deepseek"));
    fv.insert("base_url".into(), json!("https://new.example"));
    router
        .dispatch(ChannelEvent::FormCb { key: key(), value: json!({"form": "provider-custom", "op": "submit", "id": "deepseek"}), form_value: fv, card_ref: Some("om_s".into()) })
        .await;
    let _ = rx.recv().await.unwrap();

    let reloaded = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item_with_key("deepseek", "sk-old")],
    )
    .unwrap();
    let got = reloaded.get("deepseek").await.expect("record exists");
    assert_eq!(
        got.get("api_key").and_then(Value::as_str),
        Some("sk-old"),
        "empty secret submit must preserve the existing key"
    );
    assert_eq!(
        got.get("base_url").and_then(Value::as_str),
        Some("https://new.example")
    );
    deisolate();
}
