//! Signal survival for the persisted session map (persist-session-map 4.2).
//!
//! persist-session-map 把映射持久化改为**按变更落库**（生命周期事件处一次
//! upsert，经单写 actor 提交），关停快照已退休——因此「SIGTERM 后状态文件
//! 被 dump」的旧断言不再成立，取而代之的是两条更强的语义：
//!
//! - **SIGTERM（优雅退出）**：每个已提交映射在事件返回时已 durable 于状态库
//!   （state-store「Mutation durability」）；关停路径不写任何文件，重启读回
//!   全部映射（含 session_id 与 desired_mode）。
//! - **SIGKILL（非优雅退出）**：落库不依赖任何关停动作——进程直接消失，
//!   已提交映射同样在库中；重启后映射完整。旧实现下同一用例失败（dump 只
//!   发生在关停），这是本 change 的核心收益。
//!
//! 两个用例都是单元级模拟（工程纪律：不要求真 kill 子进程的进程级测试）：
//! 经生产写入路径（SessionMap 生命周期钩子 + 全局 `DbStateEngine` →
//! projects.db）落库后，**不经任何关停路径**另开裸连接重开同一库读回断言；
//! 进程级旅程由 `invoke testsuite-e2e` / `testsuite-acceptance` 的重启旅程
//! 覆盖。全局引擎是进程级 OnceLock：两个用例经串行锁共享同一沙箱库，
//! 各用独立会话键互不串扰。

mod support;

use sebas::sebas_state::writer::StateWriter;
use sebas_channels::ChannelKey;
use sebas_dispatch::state::{Mapping, SessionMap};
use sebas_models::session_map::SessionMapRow;
use std::path::{Path, PathBuf};
use support::TestDir;

/// 全局引擎初始化（进程级 OnceLock 只能一次）+ 沙箱目录：写者线程随进程
/// 退出；TestDir 有意 forget（目录活在测试进程全程，归 `cargo clean` 清）。
fn shared() -> &'static PathBuf {
    use std::sync::OnceLock;
    static STATE_DIR: OnceLock<PathBuf> = OnceLock::new();
    static INIT: std::sync::Once = std::sync::Once::new();
    let dir = STATE_DIR.get_or_init(|| {
        let d = TestDir::new("sigterm_cleanup", "state");
        let path = d.path().to_path_buf();
        std::mem::forget(d);
        path
    });
    INIT.call_once(|| {
        let settings = StateWriter::start_settings(dir.join("settings.db")).expect("settings");
        let projects = StateWriter::start_projects(dir.join("projects.db")).expect("projects");
        sebas_dispatch::state_store::init_engine(Box::new(
            sebas::sebas_state::engine::DbStateEngine::with_projects(
                settings.handle().clone(),
                projects.handle().clone(),
            ),
        ));
        // 引擎持有 handle 克隆，写者线程随全局引擎续命；writer 壳保活与否
        // 无关紧要——与 run.rs 的生产装配同一形态。
        std::mem::forget((settings, projects));
    });
    dir
}

static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 「重启后」的读回面：另开裸 SQLite 连接读 session_map 全表——独立于
/// 全局引擎的写者句柄，等价于新进程重新打开库。
fn read_rows(db: &PathBuf) -> Vec<SessionMapRow> {
    let mut conn = sebas_db::conn::open(db).expect("reopen projects.db");
    sebas_models::session_map::load_session_map(&mut conn).expect("load session_map")
}

/// 沙箱内 web 会话键（带项目归属，满足「会话必须从属于项目」写库不变量）。
fn web_key(reference: &str) -> ChannelKey {
    ChannelKey::new("web", reference)
}

/// SIGTERM 用例：会话创建 → 模式/模型/label 变更（每次变更返回即已提交）
/// → 进程优雅退出（不写任何文件）→ 重开库读回：映射完整（session_id +
/// desired_mode + label + current_model），且状态目录里**从未出现**
/// sessions.json（state-store「No separate session-map file exists」）。
#[tokio::test]
async fn sigterm_committed_mappings_survive_graceful_shutdown() {
    let _guard = TEST_SERIAL.lock().unwrap();
    let dir = shared();
    let projects_db = dir.join("projects.db");
    let map = SessionMap::new();
    let key = web_key("web-sigterm");

    // 会话创建且对客户端可见（activate 完成 = 快照已暴露 Active 映射）。
    map.begin_spawn_with(key.clone(), Some("claude".into()), None, None, false)
        .await
        .unwrap();
    map.set_project_dir(&key, Some("/tmp/sigterm-proj".into()))
        .await;
    map.activate(&key, "sess-term-1".into(), None, None).await;

    // 中途变更（每次返回即已提交——state-store「Mutation durability」）。
    map.set_desired_mode(&key, "edit".into()).await;
    map.set_current_model(&key, "m-term".into()).await;
    map.set_label(&key, Some("优雅退出".into())).await;

    // 响应返回前库中已可见（2.4：未提交的变更不会落库，提交的立即落库）。
    {
        let engine = sebas_dispatch::state_store::engine().expect("engine");
        let rows = engine.load_session_map().await.unwrap();
        let row = rows
            .iter()
            .find(|r| r.thread_id.as_deref() == Some("web-sigterm"));
        let row = row.expect("mapping committed before shutdown");
        assert_eq!(row.session_id, "sess-term-1");
        assert_eq!(row.desired_mode, "edit");
    }

    // 优雅退出 = 直接结束进程；关停路径没有任何写动作（快照已退休）。
    drop(map);

    // 重开库读回：全部已提交字段完整。
    let rows = read_rows(&projects_db);
    let row = rows
        .iter()
        .find(|r| r.thread_id.as_deref() == Some("web-sigterm"))
        .expect("mapping survives graceful shutdown");
    assert_eq!(row.session_id, "sess-term-1");
    assert_eq!(row.desired_mode, "edit");
    assert_eq!(row.label.as_deref(), Some("优雅退出"));
    assert_eq!(row.current_model.as_deref(), Some("m-term"));
    assert_eq!(row.project_dir.as_deref(), Some("/tmp/sigterm-proj"));

    // 全程无独立会话映射文件。
    assert!(
        !dir.join("sessions.json").exists(),
        "no separate session-map file may exist"
    );
}

/// 生命周期逐事件落库（persist-session-map 2.2）：创建/退役/归档/移除
/// 各触发一次提交，库中行与内存映射一致；非持久形态不落库。
#[tokio::test]
async fn lifecycle_mutations_are_each_committed() {
    let _guard = TEST_SERIAL.lock().unwrap();
    let dir = shared();
    let projects_db = dir.join("projects.db");
    let map = SessionMap::new();
    let key = web_key("web-lifecycle");

    // 创建（0-turn 占位，带项目归属）→ 占位行。
    map.begin_spawn_with(
        key.clone(),
        Some("claude".into()),
        None,
        Some("ask".into()),
        true,
    )
    .await
    .unwrap();
    map.set_project_dir(&key, Some("/tmp/lifecycle-proj".into()))
        .await;
    let find = |rows: &[SessionMapRow], reference: &str| {
        rows.iter()
            .find(|r| r.thread_id.as_deref() == Some(reference))
            .cloned()
    };
    let rows = read_rows(&projects_db);
    let placeholder = find(&rows, "web-lifecycle").expect("placeholder row committed at create");
    assert_eq!(placeholder.session_id, "");
    assert!(placeholder.awaiting_first_prompt);

    // spawn 成功激活 → 同键覆盖为 Active 行。
    map.activate(&key, "sess-lc-1".into(), None, None).await;
    let rows = read_rows(&projects_db);
    let active = find(&rows, "web-lifecycle").expect("active row overwrites the placeholder");
    assert_eq!(active.session_id, "sess-lc-1");
    assert!(!active.awaiting_first_prompt);

    // 优雅退役（terminal teardown）→ 行翻 Dormant，命名来源随行迁移。
    let retired_key = map
        .retire_to_record("sess-lc-1", Some("首条预览".into()))
        .await
        .expect("active mapping retires");
    assert_eq!(retired_key, key);
    let rows = read_rows(&projects_db);
    let dormant = find(&rows, "web-lifecycle").expect("retired row stays as dormant");
    assert_eq!(dormant.session_id, "sess-lc-1");
    assert_eq!(dormant.prompt_preview.as_deref(), Some("首条预览"));

    // fallback 归档（resume load 被拒）→ closed-* 归档行落库。
    map.preserve_closed_mapping(&key, "sess-lc-1", Some("acp-lc".into()))
        .await;
    let rows = read_rows(&projects_db);
    let archive = rows
        .iter()
        .find(|r| r.thread_id.as_deref().is_some_and(|t| t.starts_with("closed-")))
        .expect("archive record row committed")
        .clone();
    assert_eq!(archive.session_id, "sess-lc-1");
    assert_eq!(archive.acp_session_id.as_deref(), Some("acp-lc"));

    // 移除（按 key 关闭；按 session 拆除）→ 行删除，重启不复活。
    map.remove_by_key(&key).await;
    let rows = read_rows(&projects_db);
    assert!(
        find(&rows, "web-lifecycle").is_none(),
        "removed mapping must not survive a restart"
    );
    let archive_key = ChannelKey::new(
        archive.chat_id.as_str(),
        archive.thread_id.clone().expect("archive reference"),
    );
    map.remove_by_key(&archive_key).await;
    let rows = read_rows(&projects_db);
    assert!(
        !rows
            .iter()
            .any(|r| r.thread_id.as_deref().is_some_and(|t| t.starts_with("closed-"))),
        "archived row is removable too"
    );
    // 活跃映射经 remove_by_session 拆除（terminal 事件路径）→ 删行。
    map.insert(
        ChannelKey::new("web", "web-lc-2"),
        {
            let mut m = Mapping::active("sess-lc-2");
            m.project_dir = Some("/tmp/lifecycle-proj".into());
            m
        },
    )
    .await
    .unwrap();
    let rows = read_rows(&projects_db);
    assert!(find(&rows, "web-lc-2").is_some(), "inserted active row");
    map.remove_by_session("sess-lc-2").await;
    let rows = read_rows(&projects_db);
    assert!(find(&rows, "web-lc-2").is_none(), "torn-down mapping deleted");
}

/// SIGKILL 用例：会话创建后**不调用任何关停/退役路径**，进程直接消失——
/// 已提交映射不依赖快照，重开库读回仍在（含 session_id 与 desired_mode），
/// 恢复侧按行原样重建映射。
#[tokio::test]
async fn sigkill_committed_mappings_survive_unclean_exit() {
    let _guard = TEST_SERIAL.lock().unwrap();
    let dir = shared();
    let projects_db = dir.join("projects.db");
    let map = SessionMap::new();
    let key = web_key("web-sigkill");

    map.begin_spawn_with(
        key.clone(),
        Some("claude".into()),
        Some("sonnet-x".into()),
        Some("auto".into()),
        true,
    )
    .await
    .unwrap();
    map.set_project_dir(&key, Some("/tmp/sigkill-proj".into()))
        .await;
    // 首条消息触发真实 spawn → activate：0-turn 占位行被 Active 行覆盖。
    map.activate(&key, "sess-kill-1".into(), None, None).await;

    // SIGKILL：无 retire、无 remove、无关停——直接丢弃整个进程内状态。
    drop(map);

    // 重启（重开库读回）：0-turn 占位消失，Active 行（覆盖后的最新已提交
    // 状态）保留全部身份字段。
    let rows = read_rows(&projects_db);
    let row = rows
        .iter()
        .find(|r| r.thread_id.as_deref() == Some("web-sigkill"))
        .expect("mapping survives an unclean exit")
        .clone();
    assert_eq!(row.session_id, "sess-kill-1");
    assert_eq!(row.desired_mode, "auto");
    assert_eq!(row.pending_model.as_deref(), Some("sonnet-x"));
    assert!(!row.awaiting_first_prompt, "激活后占位身份已被消费");
    // 恢复侧读回同一行 → 映射（含身份）原样重建。
    let (back_key, back) =
        sebas_dispatch::state::mapping_from_row(row).expect("addressable row");
    assert_eq!(back_key, key);
    assert_eq!(
        back.transcript_id(),
        Some("sess-kill-1"),
        "恢复的映射仍以原会话 id 寻址转录"
    );
}
