use feishu::events::SessionKey;
use router::state::Mapping;
use router::state::SessionMap;

#[tokio::test]
async fn insert_and_lookup() {
    let m = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    m.insert(
        k.clone(),
        Mapping {
            session_id: "s1".into(),
            last_active_unix: 1,
        },
    )
    .await;
    let got = m.get(&k).await;
    assert_eq!(got.unwrap().session_id, "s1");
}

#[tokio::test]
async fn dump_and_restore_round_trip() {
    let m = SessionMap::new();
    let k = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    m.insert(
        k.clone(),
        Mapping {
            session_id: "s1".into(),
            last_active_unix: 1,
        },
    )
    .await;

    let json = m.dump_json().await.unwrap();
    let m2 = SessionMap::restore_json(&json).unwrap();
    let got = m2.get(&k).await;
    assert_eq!(got.unwrap().session_id, "s1");
}

#[tokio::test]
async fn overflow_rejects() {
    let m = SessionMap::with_capacity(2);
    for i in 0..2 {
        m.insert(
            SessionKey {
                chat_id: format!("oc_{i}"),
                thread_id: None,
            },
            Mapping {
                session_id: format!("s_{i}"),
                last_active_unix: 0,
            },
        )
        .await;
    }
    let r = m
        .insert(
            SessionKey {
                chat_id: "oc_3".into(),
                thread_id: None,
            },
            Mapping {
                session_id: "s_3".into(),
                last_active_unix: 0,
            },
        )
        .await;
    assert!(r.is_err());
}
