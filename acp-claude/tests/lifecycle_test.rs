use acp_claude::manager::SessionManager;
use std::time::Duration;

#[tokio::test]
async fn create_and_kill() {
    let mgr = SessionManager::new();
    let id = mgr
        .create_session("/bin/cat", vec![], None, "hello".into())
        .await
        .expect("spawn cat");
    tokio::time::sleep(Duration::from_millis(100)).await;
    mgr.kill(&id).await;
}

#[tokio::test]
async fn kill_unknown_is_noop() {
    let mgr = SessionManager::new();
    mgr.kill("nope").await;  // must not panic
}