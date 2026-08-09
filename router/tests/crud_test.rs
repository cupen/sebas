//! 通用 CRUD 表单状态机的集成测试：schema + 内存存储，驱动
//! 列表 / 新增 / 编辑 / 删除 / 取消 全流程。

use feishu::events::SessionKey;
use feishu::forms::{FormField, FormSpec};
use router::CrudStore;
use router::Out;
use router::crud::{
    CrudForm, InMemoryStore, Item, OP_CANCEL, OP_CREATE, OP_DELETE, OP_EDIT, OP_SUBMIT,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

fn spec() -> FormSpec {
    FormSpec::new(
        "note",
        "备忘",
        vec![
            FormField::Text {
                name: "title".into(),
                label: "标题".into(),
                required: true,
                placeholder: "标题".into(),
                secret: false,
            },
            FormField::Text {
                name: "body".into(),
                label: "内容".into(),
                required: false,
                placeholder: "正文".into(),
                secret: false,
            },
        ],
    )
}

fn key() -> SessionKey {
    SessionKey {
        chat_id: "oc_crud".into(),
        thread_id: None,
    }
}

fn payload(op: &str, id: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("form".into(), Value::String("note".into()));
    m.insert("op".into(), Value::String(op.into()));
    if let Some(id) = id {
        m.insert("id".into(), Value::String(id.into()));
    }
    Value::Object(m)
}

fn item(id: &str, title: &str) -> Item {
    let mut m = Map::new();
    m.insert("id".into(), Value::String(id.into()));
    m.insert("title".into(), Value::String(title.into()));
    m
}

fn form_value(title: &str) -> BTreeMap<String, Value> {
    let mut fv = BTreeMap::new();
    fv.insert("title".into(), json!(title));
    fv.insert("body".into(), json!("正文"));
    fv
}

#[tokio::test]
async fn open_sends_list_card_with_create_button() {
    let crud = CrudForm::new(spec(), "id", InMemoryStore::new("id"));
    match crud.open(key()).await {
        Out::SendCard { card, .. } => {
            let s = card.to_string();
            assert!(s.contains("暂无记录"), "{s}");
            assert!(s.contains("＋ 新增"), "{s}");
        }
        other => panic!("expected SendCard, got {other:?}"),
    }
}

#[tokio::test]
async fn create_submit_inserts_and_flips_card_in_place() {
    let store = InMemoryStore::new("id");
    let crud = CrudForm::new(spec(), "id", store.clone());

    let out = crud
        .handle(
            key(),
            &payload(OP_SUBMIT, None),
            &form_value("写周报"),
            Some("om_1".into()),
        )
        .await;
    match out {
        Out::UpdateCardByMsgId { msg_id, card, .. } => {
            assert_eq!(msg_id, "om_1");
            let s = card.to_string();
            assert!(s.contains("共 1 条"), "{s}");
            assert!(s.contains("写周报"), "{s}");
        }
        other => panic!("expected UpdateCardByMsgId, got {other:?}"),
    }

    let items = store.list().await;
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].get("title").and_then(Value::as_str),
        Some("写周报")
    );
    // 新增必须由模块生成 id。
    assert!(
        items[0].get("id").and_then(Value::as_str).is_some(),
        "create must mint an id"
    );
}

#[tokio::test]
async fn edit_prefills_then_submit_updates() {
    let store = InMemoryStore::with_items("id", vec![item("n1", "旧标题")]);
    let crud = CrudForm::new(spec(), "id", store.clone());

    // 点「编辑」→ 预填表单卡片（含 default_value 与 submit 负载里的 id）。
    let out = crud
        .handle(
            key(),
            &payload(OP_EDIT, Some("n1")),
            &BTreeMap::new(),
            Some("om_2".into()),
        )
        .await;
    let Out::UpdateCardByMsgId { card, .. } = out else {
        panic!("expected UpdateCardByMsgId");
    };
    let s = card.to_string();
    assert!(s.contains("旧标题"), "edit form must prefill: {s}");
    assert!(
        s.contains("\"id\":\"n1\""),
        "submit payload carries id: {s}"
    );

    // 提交 → 更新同一条记录。
    let out = crud
        .handle(
            key(),
            &payload(OP_SUBMIT, Some("n1")),
            &form_value("新标题"),
            None,
        )
        .await;
    assert!(matches!(out, Out::SendCard { .. }), "got {out:?}");

    let item = store.get("n1").await.expect("record still exists");
    assert_eq!(item.get("title").and_then(Value::as_str), Some("新标题"));
    assert_eq!(store.list().await.len(), 1, "update must not duplicate");
}

#[tokio::test]
async fn delete_removes_item_and_returns_list() {
    let store = InMemoryStore::with_items("id", vec![item("n1", "旧标题")]);
    let crud = CrudForm::new(spec(), "id", store.clone());

    let out = crud
        .handle(
            key(),
            &payload(OP_DELETE, Some("n1")),
            &BTreeMap::new(),
            Some("om_3".into()),
        )
        .await;
    match out {
        Out::UpdateCardByMsgId { card, .. } => {
            assert!(card.to_string().contains("暂无记录"));
        }
        other => panic!("expected UpdateCardByMsgId, got {other:?}"),
    }
    assert!(store.get("n1").await.is_none());
    assert!(store.list().await.is_empty());
}

#[tokio::test]
async fn cancel_returns_to_list_without_touching_store() {
    let store = InMemoryStore::with_items("id", vec![item("n1", "旧标题")]);
    let crud = CrudForm::new(spec(), "id", store.clone());

    let out = crud
        .handle(
            key(),
            &payload(OP_CANCEL, None),
            &BTreeMap::new(),
            Some("om_4".into()),
        )
        .await;
    match out {
        Out::UpdateCardByMsgId { card, .. } => {
            assert!(card.to_string().contains("旧标题"));
        }
        other => panic!("expected UpdateCardByMsgId, got {other:?}"),
    }
    assert_eq!(store.list().await.len(), 1);
}

#[tokio::test]
async fn foreign_or_unknown_payload_falls_back_to_list_safely() {
    let store = InMemoryStore::with_items("id", vec![item("n1", "旧标题")]);
    let crud = CrudForm::new(spec(), "id", store.clone());

    // 其它表单的负载（如权限卡）不能误伤本实体。
    let out = crud
        .handle(
            key(),
            &json!({ "form": "other", "op": "create" }),
            &BTreeMap::new(),
            None,
        )
        .await;
    assert!(matches!(out, Out::SendCard { .. }));

    // 未知 op 兜底回列表。
    let out = crud
        .handle(
            key(),
            &payload("bogus", None),
            &BTreeMap::new(),
            Some("om_5".into()),
        )
        .await;
    assert!(matches!(out, Out::UpdateCardByMsgId { .. }));
    assert_eq!(
        store.list().await.len(),
        1,
        "no write on foreign/unknown op"
    );
}

#[tokio::test]
async fn create_button_opens_empty_form() {
    let crud = CrudForm::new(spec(), "id", InMemoryStore::new("id"));
    let out = crud
        .handle(
            key(),
            &payload(OP_CREATE, None),
            &BTreeMap::new(),
            Some("om_6".into()),
        )
        .await;
    match out {
        Out::UpdateCardByMsgId { card, .. } => {
            let s = card.to_string();
            assert!(s.contains("\"tag\":\"form\""), "{s}");
            // 取消/返回列表按钮在表单容器外（普通回调按钮，带 cancel 负载）。
            assert!(s.contains("\"op\":\"cancel\""), "{s}");
            // 新增表单的提交负载不带 id。
            assert!(!s.contains("\"id\":\"n"), "{s}");
        }
        other => panic!("expected UpdateCardByMsgId, got {other:?}"),
    }
}
