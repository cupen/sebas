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
    let form = CrudForm::new(spec(), "name", store.clone());
    let forms = router::ProviderForms {
        preset: Arc::new(CrudForm::new(
            FormSpec::new("provider-preset", "Provider（预设）", vec![]),
            "name",
            store,
        )),
        custom: Arc::new(form),
    };
    RouterHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(forms)),
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
            action: card_action(json!({"form": "provider-custom", "op": "create"}), "om_1"),
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
            value: json!({"form": "provider-custom", "op": "submit"}),
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
                json!({"form": "provider-custom", "op": "delete", "id": "deepseek"}),
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
async fn cancel_button_returns_to_list_not_to_dead_session_card() {
    // 回归：表单容器外的「取消」按钮带 op=cancel，必须被 on_button 识别为
    // CRUD 路由并回列表卡；如果被吞掉就会落到 ACP session 死会话路径。
    let dir = tempfile::tempdir().unwrap();
    let (router, mut rx) = provider_router(&dir);

    router
        .dispatch(FeishuIn::ButtonCb {
            key: key(),
            action: card_action(json!({"form": "provider-custom", "op": "cancel"}), "om_cancel"),
        })
        .await;

    let out = rx.recv().await.unwrap();
    match out {
        Out::UpdateCardByMsgId { msg_id, card, .. } => {
            assert_eq!(msg_id, "om_cancel", "cancel 应原地更新原表单卡");
            // 列表卡包含「＋ 新增（预设/自定义）」按钮，证明回到 CRUD 列表。
            let s = card.to_string();
            assert!(s.contains("＋ 新增（预设）"), "{s}");
            assert!(s.contains("＋ 新增（自定义）"), "{s}");
        }
        other => panic!("cancel 应回到 CRUD 列表卡，得到 {other:?}"),
    }
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
    let form = CrudForm::new(spec(), "name", store.clone());
    let forms = router::ProviderForms {
        preset: Arc::new(CrudForm::new(
            FormSpec::new("provider-preset", "Provider（预设）", vec![]),
            "name",
            store,
        )),
        custom: Arc::new(form),
    };
    let (router, mut rx) = RouterHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(forms)),
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
                json!({"form": "provider-custom", "op": "edit", "id": "deepseek"}),
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
    let form = CrudForm::new(spec(), "name", store.clone());
    let forms = router::ProviderForms {
        preset: Arc::new(CrudForm::new(
            FormSpec::new("provider-preset", "Provider（预设）", vec![]),
            "name",
            store,
        )),
        custom: Arc::new(form),
    };
    let (router, mut rx) = RouterHandle::new_with_provider_form(
        SessionMap::new(),
        Default::default(),
        16,
        Some(Arc::new(forms)),
    );

    // 编辑提交：api_key 留空（不提交），改 base_url。
    let mut fv = BTreeMap::new();
    fv.insert("name".into(), json!("deepseek"));
    fv.insert("base_url".into(), json!("https://new.example"));
    router
        .dispatch(FeishuIn::FormCb {
            key: key(),
            value: json!({"form": "provider-custom", "op": "submit", "id": "deepseek"}),
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
