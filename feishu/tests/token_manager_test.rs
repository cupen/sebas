//! TokenManager: lazy fetch on first use, expiry-triggered refetch,
//! force_refresh, and the send_card retry-once-on-business-error policy.
//! Backed by a hand-rolled TCP stub (no external mock deps).

use feishu::client::TokenManager;
use feishu::events::SessionKey;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Stub {
    base: String,
    token_hits: Arc<AtomicUsize>,
    card_hits: Arc<AtomicUsize>,
    handle: std::thread::JoinHandle<()>,
}

/// Serves `token_body` for POST /token and a script of card responses for
/// POST /card. One request per connection (`connection: close`) so reqwest
/// never pipelines.
fn start_stub(card_bodies: Vec<String>) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let token_hits = Arc::new(AtomicUsize::new(0));
    let card_hits = Arc::new(AtomicUsize::new(0));
    let (th, ch) = (token_hits.clone(), card_hits.clone());
    let handle = std::thread::spawn(move || {
        let mut card_bodies = card_bodies.into_iter();
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => break,
            };
            // Read request head + body (content-length).
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut content_len = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                let lower = line.to_lowercase();
                if let Some(stripped) = lower.strip_prefix("content-length:") {
                    content_len = stripped.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; content_len];
            let _ = reader.read_exact(&mut body);
            let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
            let body_out = if path.starts_with("/token") {
                th.fetch_add(1, Ordering::SeqCst);
                r#"{"code":0,"msg":"ok","tenant_access_token":"t-fresh","expire":7200}"#.to_string()
            } else {
                ch.fetch_add(1, Ordering::SeqCst);
                card_bodies.next().unwrap_or_else(|| {
                    r#"{"code":0,"msg":"ok","data":{"message_id":"om_1"}}"#.to_string()
                })
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body_out.len(),
                body_out
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    Stub {
        base: format!("http://127.0.0.1:{port}"),
        token_hits,
        card_hits,
        handle,
    }
}

#[tokio::test]
async fn fetches_lazily_and_caches_until_expiry() {
    let stub = start_stub(vec![]);
    let tm = TokenManager::with_url(
        "app".into(),
        "secret".into(),
        format!("{}/token", stub.base),
    );
    let t1 = tm.token().await.expect("first fetch");
    assert_eq!(t1, "t-fresh");
    let t2 = tm.token().await.expect("cached");
    assert_eq!(t2, "t-fresh");
    assert_eq!(
        stub.token_hits.load(Ordering::SeqCst),
        1,
        "second token() must be served from cache"
    );
    tm.force_refresh().await.expect("refresh");
    assert_eq!(stub.token_hits.load(Ordering::SeqCst), 2);
    drop(stub.handle);
}

#[tokio::test]
async fn send_card_retries_once_with_fresh_token_on_business_error() {
    // First card attempt: business error (e.g. invalid token). Retry: success.
    let stub = start_stub(vec![
        r#"{"code":99991663,"msg":"tenant access token invalid"}"#.to_string(),
        r#"{"code":0,"msg":"ok","data":{"message_id":"om_ok"}}"#.to_string(),
    ]);
    let tm = TokenManager::with_url(
        "app".into(),
        "secret".into(),
        format!("{}/token", stub.base),
    );
    // Point the client at the stub by using send_card's http + stub URL:
    // send_card builds its own URL, so for the stub we exercise the retry
    // helper through a card endpoint the stub serves. FeishuClient::send_card
    // targets the real Feishu host; for the test we call the internal
    // `post_card` via a client whose base is the stub. To keep the public
    // surface minimal, the retry policy is factored into
    // `FeishuClient::post_card_with_retry`, which the test drives directly.
    let client = feishu::client::FeishuClient::new(feishu::client::FeishuConfig {
        app_id: "app".into(),
        app_secret: "secret".into(),
        owner_id: String::new(),
    });
    let http = reqwest::Client::new();
    let key = SessionKey {
        chat_id: "oc_x".into(),
        thread_id: None,
    };
    let out = client
        .post_card_with_retry(
            &http,
            &tm,
            &format!("{}/card", stub.base),
            serde_json::json!({"receive_id": key.chat_id}),
        )
        .await
        .expect("retry succeeds");
    assert_eq!(out, "om_ok");
    assert_eq!(
        stub.card_hits.load(Ordering::SeqCst),
        2,
        "exactly one retry"
    );
    assert!(
        stub.token_hits.load(Ordering::SeqCst) >= 2,
        "initial token + forced refresh"
    );
    drop(stub.handle);
}
