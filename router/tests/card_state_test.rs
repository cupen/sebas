//! CardStateMap 存储语义单测（FSM/累积在 card_state_test 的后续测试 + Task 5 覆盖）。

use feishu::cards::CardElement;
use router::card_state::{CardState, CardStateMap};

#[tokio::test]
async fn seed_is_idempotent_keeps_accumulated_prompt() {
    let m = CardStateMap::default();
    m.seed("s1".into(), "original".into()).await;
    m.apply("s1", |st| {
        st.body.push(CardElement::Markdown {
            content: "accumulated".into(),
        })
    })
    .await;
    // 重入 seed：保留原 prompt 与 body，不冲掉。
    m.seed("s1".into(), "SHOULD_NOT_WIN".into()).await;
    let snap = m.snapshot("s1").await.expect("seeded");
    assert_eq!(snap.user_prompt, "original");
    assert_eq!(snap.status_emoji, "👀");
    assert_eq!(snap.body.len(), 1);
}

#[tokio::test]
async fn apply_lazy_seeds_with_empty_prompt() {
    let m = CardStateMap::default();
    // 未 seed 直接 apply：lazy 兜底，prompt=""。
    m.apply("s2", |st| {
        st.body.push(CardElement::Markdown {
            content: "early".into(),
        })
    })
    .await;
    let snap = m.snapshot("s2").await.expect("lazy seeded");
    assert_eq!(snap.user_prompt, "");
    assert_eq!(snap.status_emoji, "👀");
    assert_eq!(snap.body.len(), 1);
}

#[tokio::test]
async fn drop_removes_entry() {
    let m = CardStateMap::default();
    m.seed("s3".into(), "hi".into()).await;
    assert!(m.snapshot("s3").await.is_some());
    m.drop("s3").await;
    assert!(m.snapshot("s3").await.is_none());
    // 幂等：drop 不存在的 entry 不 panic。
    m.drop("s3").await;
}

#[tokio::test]
async fn new_and_lazy_constructors() {
    let a = CardState::new("prompt");
    assert_eq!(a.user_prompt, "prompt");
    assert_eq!(a.status_emoji, "👀");
    assert!(a.body.is_empty());
    let b = CardState::lazy();
    assert_eq!(b.user_prompt, "");
    assert_eq!(b.status_emoji, "👀");
}
