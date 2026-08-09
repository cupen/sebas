//! `/provider` 命令 + provider CRUD 表单的路由集成测试：经 RouterHandle
//! 驱动 列表 → 新增 → 提交 → 删除，验证按钮/表单回调被正确路由且
//! 存储（FileStore + overlay 文件）随变更持久化。

use feishu::events::{CardAction, FeishuIn, SessionKey};
use feishu::forms::{FormField, FormSpec};
use router::CrudStore;
use router::Out;
use router::crud::{CrudForm, FileStore, Item};
use router::router::RouterHandle;
use router::state::SessionMap;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

fn spec() -> FormSpec {
    FormSpec::new(
        "provider",
        "Provider",
        vec![
            FormField::Text {
                name: "name".into(),
                label: "名称".into(),
                required: true,
                placeholder: "".into(),
                secret: false,
            },
            FormField::Text {
                name: "base_url".into(),
                label: "Base URL".into(),
                required: true,
                placeholder: "".into(),
                secret: false,
            },
            FormField::Text {
                name: "api_key".into(),
                label: "API Key".into(),
                required: false,
                placeholder: "粘贴密钥".into(),
                secret: true,
            },
        ],
    )
}

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_provider".into(),
        thread_id: None,
    }
}

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

fn card_action(payload: Value, msg_id: &str) -> CardAction {
    CardAction {
        session_id: String::new(),
        request_id: None,
        decision: None,
        value: json!({
            "action": { "value": payload },
            "context": { "open_message_id": msg_id }
        }),
    }
}

fn provider_router(dir: &tempfile::TempDir) -> (RouterHandle, tokio::sync::mpsc::Receiver<Out>) {
    let store = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item("deepseek")],
    )
    .unwrap();
    let form = CrudForm::new(spec(), "name", store);
    RouterHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(form)),
    )
}

#[tokio::test]
async fn provider_command_opens_list_card_with_seed() {
    let dir = tempfile::tempdir().unwrap();
    let (router, mut rx) = provider_router(&dir);

    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/provider".into(),
            reply_to: None,
        })
        .await;

    let out = rx.recv().await.unwrap();
    match out {
        Out::SendCard { card, .. } => {
            let s = card.to_string();
            assert!(s.contains("deepseek"), "{s}");
            assert!(s.contains("＋ 新增"), "{s}");
            assert!(s.contains("\"op\":\"create\""), "{s}");
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}

#[tokio::test]
async fn provider_create_submit_delete_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let (router, mut rx) = provider_router(&dir);

    // 点「＋ 新增」→ 表单卡原地出现。
    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: card_action(json!({"form": "provider", "op": "create"}), "om_1"),
        })
        .await;
    let out = rx.recv().await.unwrap();
    let Out::UpdateCardByMsgId { msg_id, card, .. } = out else {
        panic!("expected UpdateCardByMsgId, got {out:?}");
    };
    assert_eq!(msg_id, "om_1");
    assert!(card.to_string().contains("\"tag\":\"form\""), "{card}");

    // 提交 → 列表卡原地更新，记录进 FileStore 并落盘。
    let mut fv = BTreeMap::new();
    fv.insert("name".into(), json!("openai"));
    fv.insert("base_url".into(), json!("https://api.openai.com"));
    router
        .dispatch(FeishuIn::FormCb {
            key: key(),
            value: json!({"form": "provider", "op": "submit"}),
            form_value: fv,
            message_id: Some("om_1".into()),
        })
        .await;
    let out = rx.recv().await.unwrap();
    let Out::UpdateCardByMsgId { card, .. } = out else {
        panic!("expected UpdateCardByMsgId, got {out:?}");
    };
    assert!(card.to_string().contains("openai"), "{card}");

    // 删除种子里的 deepseek（写墓碑）。
    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: card_action(
                json!({"form": "provider", "op": "delete", "id": "deepseek"}),
                "om_2",
            ),
        })
        .await;
    let out = rx.recv().await.unwrap();
    assert!(matches!(out, Out::UpdateCardByMsgId { .. }), "{out:?}");

    // 重启视角：重新加载 overlay + 种子 → 只剩 openai。
    let reloaded = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item("deepseek")],
    )
    .unwrap();
    let items = reloaded.list().await;
    assert_eq!(items.len(), 1, "deepseek 墓碑必须生效");
    assert_eq!(items[0].get("name").and_then(Value::as_str), Some("openai"));
}

#[tokio::test]
async fn permission_shaped_button_is_not_routed_to_provider_crud() {
    let dir = tempfile::tempdir().unwrap();
    let (router, mut rx) = provider_router(&dir);

    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: CardAction {
                session_id: "s1".into(),
                request_id: Some("r1".into()),
                decision: Some("allow_once".into()),
                value: json!({
                    "action": { "value": {"session_id": "s1", "request_id": "r1", "decision": "allow_once"} },
                    "context": { "open_message_id": "om_perm" }
                }),
            },
        })
        .await;

    // 没有活跃 ACP 会话 → 死会话卡（SendCard）；如果误路由到 CRUD 会是
    // UpdateCardByMsgId（带 om_perm），用这个区分。
    let out = rx.recv().await.unwrap();
    assert!(matches!(out, Out::SendCard { .. }), "{out:?}");
}

#[tokio::test]
async fn secret_key_is_masked_in_list_and_not_prefilled_in_edit() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item_with_key("deepseek", "sk-super-secret")],
    )
    .unwrap();
    let form = CrudForm::new(spec(), "name", store);
    let (router, mut rx) = RouterHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(form)),
    );

    // 列表卡：密钥掩码显示，绝不回显明文。
    router
        .dispatch(FeishuIn::Text {
            key: key(),
            text: "/provider".into(),
            reply_to: None,
        })
        .await;
    let out = rx.recv().await.unwrap();
    let Out::SendCard { card, .. } = out else {
        panic!("expected SendCard");
    };
    let s = card.to_string();
    assert!(s.contains("••••••"), "{s}");
    assert!(!s.contains("sk-super-secret"), "list must mask secret: {s}");

    // 编辑表单：不预填密钥。
    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: card_action(
                json!({"form": "provider", "op": "edit", "id": "deepseek"}),
                "om_e",
            ),
        })
        .await;
    let out = rx.recv().await.unwrap();
    let Out::UpdateCardByMsgId { card, .. } = out else {
        panic!("expected UpdateCardByMsgId");
    };
    let s = card.to_string();
    assert!(
        !s.contains("sk-super-secret"),
        "edit form must not prefill secret: {s}"
    );
}

#[tokio::test]
async fn empty_secret_submit_preserves_existing_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = FileStore::load(
        dir.path().join("providers.json"),
        "name",
        vec![item_with_key("deepseek", "sk-old")],
    )
    .unwrap();
    let form = CrudForm::new(spec(), "name", store);
    let (router, mut rx) = RouterHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(form)),
    );

    // 编辑提交：api_key 留空（不提交），改 base_url。
    let mut fv = BTreeMap::new();
    fv.insert("name".into(), json!("deepseek"));
    fv.insert("base_url".into(), json!("https://new.example"));
    router
        .dispatch(FeishuIn::FormCb {
            key: key(),
            value: json!({"form": "provider", "op": "submit", "id": "deepseek"}),
            form_value: fv,
            message_id: Some("om_s".into()),
        })
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
}
