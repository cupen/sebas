//! 会话终态 → 任务 `in-review` 的**端到端**联动（agent-task-center 5.1~5.3）.
//!
//! `sebas-dispatch/tests/task_terminal_hook_test.rs` 用记录型假钩子钉住了
//! 引擎的裁决面（何时调用钩子）；本文件补另一半：把**生产钩子**
//! （`task_hook::make_hook` → 真实 `BeadsClient` spawn `bd`）装进真实
//! `DispatchHandle`，断言 `bd` 命令行形状，以及反例一律**不**发命令。
//!
//! 终态来源覆盖面（design D7）：
//! - `closed`：操作者显式关闭（`web_close_session`）；
//! - `exited`：子进程死亡确认（`finalize_dead_child`）；
//! - `failed` / 在飞回合 / 非任务会话 / 老会话：全部不触发。

use sebas_acp::AcpEvent;
use sebas_channels::ChannelKey;
use sebas_dispatch::engine::{CancelOutcome, CloseOutcome};
use sebas_dispatch::{DispatchHandle, Mapping, SessionMap};
use sebas_webui::beads_client::BeadsClient;
use sebas_webui::task_hook::make_hook;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn web_key(id: &str) -> ChannelKey {
    ChannelKey::new("web", format!("web-{id}"))
}

/// 假 `bd`：把每次调用的 argv 逐行写进 `calls.log`，并对 `update` 回一份
/// 合法 issue 数组（钩子只关心命令形状，不解析这里的内容）。
fn fake_bd(dir: &Path) -> (PathBuf, PathBuf) {
    let bin = dir.join("bd");
    let calls = dir.join("calls.log");
    let mut file = std::fs::File::create(&bin).unwrap();
    write!(
        file,
        "#!/bin/sh\necho \"$@\" >> {}\necho '[{{\"id\":\"bd-42\",\"title\":\"t\",\"status\":\"in_progress\",\"labels\":[\"in-review\"]}}]'\n",
        calls.display()
    )
    .unwrap();
    file.sync_all().unwrap();
    drop(file);
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, calls)
}

async fn task_mapping(session_id: &str, task_id: Option<&str>, active: i64) -> Mapping {
    let mut mapping = Mapping::active(session_id);
    mapping.task_id = task_id.map(str::to_string);
    mapping.project_dir = Some("/tmp/project".into());
    mapping.pending_kind = Some("claude".into());
    mapping.last_active_unix = active;
    mapping
}

/// 轮询等生产钩子的异步 `bd` 落盘（fire-and-forget：调用方同步返回）。
async fn wait_for_calls(calls: &Path) -> String {
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(calls)
            && !text.trim().is_empty()
        {
            return text;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    String::new()
}

async fn router_with_hook(
    map: SessionMap,
    bin: PathBuf,
) -> (DispatchHandle, tokio::sync::mpsc::Receiver<sebas_dispatch::Out>) {
    let (router, rx) = DispatchHandle::new(map);
    router.set_session_terminal_hook(make_hook(BeadsClient::with_bin(bin)));
    (router, rx)
}

/// 5.1/5.2 正例（`Closed`）：正常完成的回合一关 → 生产钩子真的调
/// `bd update <task_id> --add-label in-review`，且带上会话绑定的 agent kind
/// 作为审计 actor。
#[tokio::test]
async fn normal_turn_then_close_runs_bd_update_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, calls) = fake_bd(tmp.path());
    let map = SessionMap::new();
    let key = web_key("a");
    map.insert(key.clone(), task_mapping("sid-a", Some("bd-42"), 100).await)
        .await
        .unwrap();
    let (router, _rx) = router_with_hook(map, bin).await;

    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-a".into(),
        })
        .await;
    assert_eq!(
        router.web_close_session(key).await,
        CloseOutcome::Closed {
            discarded_pending: 0
        }
    );

    let seen = wait_for_calls(&calls).await;
    assert!(
        seen.contains("update bd-42 --add-label in-review"),
        "钩子必须落 bd update 的完整参数: {seen:?}"
    );
    assert!(
        seen.contains("--actor claude"),
        "审计 actor = 会话绑定的 agent kind: {seen:?}"
    );
}

/// 5.1 正例（`Exited`）：子进程死亡确认走 `finalize_dead_child`，同样在
/// 最后一轮正常完成时给任务加标签。
#[tokio::test]
async fn child_death_after_normal_turn_runs_bd_update() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, calls) = fake_bd(tmp.path());
    let map = SessionMap::new();
    let key = web_key("b");
    map.insert(key.clone(), task_mapping("sid-b", Some("bd-42"), 100).await)
        .await
        .unwrap();
    let (router, _rx) = router_with_hook(map, bin).await;

    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-b".into(),
        })
        .await;
    router.finalize_dead_child("sid-b").await;

    let seen = wait_for_calls(&calls).await;
    assert!(
        seen.contains("update bd-42 --add-label in-review"),
        "Exited 终态必须推进任务: {seen:?}"
    );
}

/// 5.1 反例（`Failed`）：terminal Error 是失败终态——关闭与子进程死亡都
/// 不得推进任务（最后一轮结算为 500）。
#[tokio::test]
async fn failed_terminal_never_runs_bd() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, calls) = fake_bd(tmp.path());
    let map = SessionMap::new();
    let key = web_key("c");
    map.insert(key.clone(), task_mapping("sid-c", Some("bd-42"), 100).await)
        .await
        .unwrap();
    let (router, _rx) = router_with_hook(map, bin).await;

    router
        .dispatch_acp_event(AcpEvent::Error {
            session_id: "sid-c".into(),
            message: "agent crashed".into(),
            terminal: true,
        })
        .await;
    router.finalize_dead_child("sid-c").await;
    router.web_close_session(key).await;

    // 给假 `bd` 一个可观测的窗口：若被调用，文件会立刻非空。
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        std::fs::read_to_string(&calls).map(|s| s.trim().is_empty()).unwrap_or(true),
        "Failed 终态不得调用 bd"
    );
}

/// 5.2 反例：在飞回合（SEED/WORKING）被关闭——本回合没有正常收尾，不推进。
#[tokio::test]
async fn close_while_turn_in_flight_never_runs_bd() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, calls) = fake_bd(tmp.path());
    let map = SessionMap::new();
    let key = web_key("d");
    map.insert(key.clone(), task_mapping("sid-d", Some("bd-42"), 100).await)
        .await
        .unwrap();
    let (router, _rx) = router_with_hook(map, bin).await;

    router.seed_card("sid-d".to_string(), "go".into()).await;
    router.web_close_session(key).await;

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        std::fs::read_to_string(&calls).map(|s| s.trim().is_empty()).unwrap_or(true),
        "在飞回合的关闭不得推进任务"
    );
}

/// 5.2 反例（spec「Older session terminating does not affect task state」）：
/// 任务下还有更新的会话时，老会话终结不得回改任务状态。
#[tokio::test]
async fn older_session_terminal_never_runs_bd() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, calls) = fake_bd(tmp.path());
    let map = SessionMap::new();
    let older = web_key("old");
    let newer = web_key("new");
    map.insert(older.clone(), task_mapping("sid-old", Some("bd-42"), 100).await)
        .await
        .unwrap();
    map.insert(newer.clone(), task_mapping("sid-new", Some("bd-42"), 300).await)
        .await
        .unwrap();
    let (router, _rx) = router_with_hook(map, bin).await;

    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-old".into(),
        })
        .await;
    router.web_close_session(older).await;

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        std::fs::read_to_string(&calls).map(|s| s.trim().is_empty()).unwrap_or(true),
        "老会话终结不得推进任务（最新会话仍在）"
    );
}

/// 5.2 反例：非任务会话（`task_id` 为空/未设）终态不触发任何 `bd`。
#[tokio::test]
async fn session_without_task_id_never_runs_bd() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, calls) = fake_bd(tmp.path());
    let map = SessionMap::new();
    let key = web_key("e");
    map.insert(key.clone(), task_mapping("sid-e", None, 100).await)
        .await
        .unwrap();
    let (router, _rx) = router_with_hook(map, bin).await;

    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-e".into(),
        })
        .await;
    router.web_close_session(key).await;

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        std::fs::read_to_string(&calls).map(|s| s.trim().is_empty()).unwrap_or(true),
        "非任务会话不得调用 bd"
    );
}

/// 5.2 反例：被取消的最后一轮（499）不是正常完成——关闭不推进任务。
#[tokio::test]
async fn cancelled_last_turn_never_runs_bd() {
    let tmp = tempfile::tempdir().unwrap();
    let (bin, calls) = fake_bd(tmp.path());
    let map = SessionMap::new();
    let key = web_key("f");
    map.insert(key.clone(), task_mapping("sid-f", Some("bd-42"), 100).await)
        .await
        .unwrap();
    let (router, _rx) = router_with_hook(map, bin).await;

    router.seed_card("sid-f".to_string(), "go".into()).await;
    assert!(matches!(
        router.web_cancel_session(&key).await,
        CancelOutcome::Dispatched
    ));
    router
        .dispatch_acp_event(AcpEvent::Finished {
            session_id: "sid-f".into(),
        })
        .await;
    router.web_close_session(key).await;

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(
        std::fs::read_to_string(&calls).map(|s| s.trim().is_empty()).unwrap_or(true),
        "被取消的最后一轮不得推进任务"
    );
}
