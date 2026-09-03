use sebas_acp::claude::manager::SessionManager;
use std::path::PathBuf;
use std::time::Duration;

fn fake_claude_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("target/debug/fake-claude-cli")
}

#[tokio::test]
async fn create_and_kill() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    let fake = fake_claude_path();
    let id = mgr
        .create_claude_session(fake.to_str().unwrap(), vec![], None, vec![], "hello".into())
        .await
        .expect("spawn fake-claude");
    tokio::time::sleep(Duration::from_millis(100)).await;
    mgr.kill(&id).await;
}

#[tokio::test]
async fn kill_unknown_is_noop() {
    let mgr = SessionManager::claude_only(Duration::from_secs(30));
    mgr.kill("nope").await; // must not panic
}
