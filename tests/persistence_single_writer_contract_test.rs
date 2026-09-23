//! extract-sebas-db 集成补测：单写 actor 的跨调用方契约（review 补齐）。
//!
//! 对照 `openspec/changes/extract-sebas-db/specs/persistence-runtime/spec.md`
//! 里此前只有单元级夹具覆盖、缺跨 crate 行为级覆盖的场景：
//!
//! - R3「Commands serialize through one owner」的行为面：多个调用方经**克隆的
//!   `StateHandle`** 并发提交命令——命令逐条执行（任一时刻在途命令数 = 1），
//!   且每个调用方都拿到自己的结果；
//! - 同场景的「without observing another caller's partial state」：一个调用方
//!   的多语句写入单元对并发读者**整体可见**——读者永远看不到单元写了一半的
//!   中间态（两条同代行不会一新一旧）；
//! - R3/R4 的端到端组合面：根 crate 的**域接线**（`sebas_state::writer::
//!   StateWriter` + 域注册表）驱动 sebas-db actor，`StateHandle` 类型化门面
//!   对 `sebas-models` 的 ActiveRecord 行做 save/find/all/delete 全往返——
//!   单元测试用的是中性夹具表，这里补真域形状（projects 行走 projects 库
//!   写者，single-state-dir 分层）；
//! - R1「A second database does not re-implement the recipe」的行为面：auth.db
//!   （第二个库）在**多连接并发写**下靠共享配方的 busy_timeout=5s + WAL +
//!   Immediate 事务入口全部成功——pragma 读数断言（user_store 单测）之外的
//!   竞争级证明。
//!
//! 全部用例只走进程内 SQLite，不起任何网络/浏览器设施。

use sebas::sebas_state::writer::StateWriter;
use sebas_models::project::ProjectRow;
use sebas_webui::rbac::Role;
use sebas_webui::user_store::{StoreError, UserStore};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

// ---- R3：命令经单一所有者逐条执行 ----

/// 并发 40 个调用方经同一 `StateHandle` 提交命令：任一时刻在途命令数必须
/// 恰为 1（单写线程逐条执行），且每个调用方都收到**自己的**返回值。
#[tokio::test]
async fn commands_apply_one_at_a_time_and_each_caller_gets_its_own_result() {
    let dir = tempdir().unwrap();
    let writer = StateWriter::start(dir.path().join("mx.db")).unwrap();
    let handle = writer.handle().clone();

    let in_flight = Arc::new(AtomicUsize::new(0));
    let overlaps = Arc::new(Mutex::new(Vec::<u32>::new()));

    let mut tasks = Vec::new();
    for i in 0..40u32 {
        let h = handle.clone();
        let in_flight = in_flight.clone();
        let overlaps = overlaps.clone();
        tasks.push(tokio::spawn(async move {
            h.exec::<u32>(move |_conn| {
                // 第二个命令在第一个还在途时进入 = 串行化被破坏。
                if in_flight.fetch_add(1, Ordering::SeqCst) != 0 {
                    overlaps.lock().unwrap().push(i);
                }
                // 停留一下，放大与其他命令重叠的窗口（若串行化破损必然踩中）。
                std::thread::sleep(std::time::Duration::from_millis(2));
                in_flight.fetch_sub(1, Ordering::SeqCst);
                Ok(i)
            })
            .await
            .expect("命令必须成功执行")
        }));
    }

    let mut results = Vec::new();
    for t in tasks {
        results.push(t.await.unwrap());
    }
    results.sort_unstable();
    assert_eq!(
        results,
        (0..40).collect::<Vec<_>>(),
        "每个调用方都必须收到自己的结果，一条不落"
    );
    assert!(
        overlaps.lock().unwrap().is_empty(),
        "观察到命令重叠执行（串行化破损）: {:?}",
        overlaps.lock().unwrap()
    );
}

// ---- R3：调用方看不到另一调用方的半成品单元 ----

/// 写者把「两条同代行」作为一个单元提交（一个 exec 闭包内先写 A 再写 B），
/// 并发读者在单元之间轮询：两行必须始终同代——若命令中途被打断/交错，读者
/// 会看到 A 是新代、B 是旧代的撕裂中间态。
#[tokio::test]
async fn reader_never_observes_a_torn_multi_statement_unit() {
    let dir = tempdir().unwrap();
    let writer = StateWriter::start_projects(dir.path().join("projects.db")).unwrap();
    let handle = writer.handle().clone();

    let row = |path: &str, generation: i64| ProjectRow {
        id: Some(format!("proj-{path}")),
        path: path.to_string(),
        name: path.to_string(),
        default_agent: None,
        branch: None,
        branch_at: generation,
        added_at: generation,
        sort_order: 0,
    };

    // 先落一个完整单元（第 1 代），之后读者的不变量恒可判定：恰两行、同代。
    handle
        .exec_void(move |conn| {
            sebas_db::record::save(conn, &row("/gen/a", 1)).map_err(|e| e.to_string())?;
            sebas_db::record::save(conn, &row("/gen/b", 1)).map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
        .unwrap();

    // 写者：第 2..=30 代，每代一个闭包内先 A 后 B。
    let writer_task = tokio::spawn({
        let h = handle.clone();
        async move {
            for generation in 2..=30i64 {
                h.exec_void(move |conn| {
                    sebas_db::record::save(conn, &row("/gen/a", generation)).map_err(|e| e.to_string())?;
                    sebas_db::record::save(conn, &row("/gen/b", generation)).map_err(|e| e.to_string())?;
                    Ok(())
                })
                .await
                .unwrap();
            }
        }
    });

    // 读者：3 个任务 × 40 轮，在写者单元之间穿插执行。
    let torn = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut readers = Vec::new();
    for _ in 0..3 {
        let h = handle.clone();
        let torn = torn.clone();
        readers.push(tokio::spawn(async move {
            for _ in 0..40 {
                let rows = h.all::<ProjectRow>().await.unwrap();
                if rows.len() != 2 {
                    torn.lock().unwrap().push(format!("行数 {} != 2", rows.len()));
                    continue;
                }
                let ga = rows[0].branch_at;
                let gb = rows[1].branch_at;
                if ga != gb {
                    torn.lock()
                        .unwrap()
                        .push(format!("撕裂读: /gen/a 在第 {ga} 代而 /gen/b 在第 {gb} 代"));
                }
            }
        }));
    }

    writer_task.await.unwrap();
    for r in readers {
        r.await.unwrap();
    }
    assert!(
        torn.lock().unwrap().is_empty(),
        "读者观察到单元的中间态（串行化/整体性破损）: {:?}",
        torn.lock().unwrap()
    );

    // 终态：两行都是最后一代。
    let rows = handle.all::<ProjectRow>().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.branch_at == 30));
}

// ---- R3/R4：类型化门面经域接线全往返（真域注册表 + sebas-models 行）----

/// 根 crate 的 `StateWriter::start_projects(db_path)`（域接线）→ sebas-db
/// actor → `sebas-models` 的 `ProjectRow`：save（插入/覆盖两分支）、find
/// （命中/未命中）、all、delete（命中/再删）全走单写线程。
#[tokio::test]
async fn typed_facade_round_trips_domain_rows_through_domain_wiring() {
    let dir = tempdir().unwrap();
    let writer = StateWriter::start_projects(dir.path().join("facade-projects.db")).unwrap();
    let handle = writer.handle().clone();

    let row = ProjectRow {
        id: Some("proj-facade".into()),
        path: "/facade/p".into(),
        name: "facade".into(),
        default_agent: Some("claude".into()),
        branch: Some("main".into()),
        branch_at: 10,
        added_at: 7,
        sort_order: 3,
    };

    // save：插入分支。
    handle.save(&row).await.unwrap();
    let found = handle
        .find::<ProjectRow, _>("/facade/p".to_string())
        .await
        .unwrap()
        .expect("主键命中");
    assert_eq!(found, row);
    assert!(
        handle
            .find::<ProjectRow, _>("/facade/none".to_string())
            .await
            .unwrap()
            .is_none(),
        "未命中的主键应返回 None"
    );

    // save：覆盖分支（同主键 upsert）。
    let mut updated = row.clone();
    updated.sort_order = 9;
    updated.branch = Some("feat/x".into());
    handle.save(&updated).await.unwrap();
    assert_eq!(
        handle.all::<ProjectRow>().await.unwrap(),
        vec![updated.clone()],
        "全表恰好一行且为最新值"
    );

    // delete：首删命中、再删空。
    assert!(handle.delete::<ProjectRow, _>("/facade/p".to_string()).await.unwrap());
    assert!(!handle.delete::<ProjectRow, _>("/facade/p".to_string()).await.unwrap());
    assert!(handle.all::<ProjectRow>().await.unwrap().is_empty());
}

// ---- R1：第二库（auth.db）靠共享配方吸收跨连接写竞争 ----

/// 四个独立连接（各自 `Mutex` 串行、跨连接无锁）在同一 auth.db 上并发建
/// 40 个不同用户：全部成功 = 共享配方的 busy_timeout=5s + WAL + 经共享入口
/// 的 `transaction_immediate` 在真实写竞争下成立。若哪天有人把配方改回本地
/// 拼装（比如丢了 busy_timeout），跨连接写锁竞争会以 `DatabaseBusy` 在这里
/// 显形。随后跨连接重名创建被拒且不留半行。
#[test]
fn second_database_absorbs_concurrent_writers_via_shared_recipe() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("busy-auth.db");

    // 主线程串行打开四个连接（建库/建 schema 不是本测的竞争点）。
    let stores: Vec<UserStore> = (0..4)
        .map(|_| UserStore::open_with_iterations(&path, 1000).expect("open store"))
        .collect();

    let outcomes: Vec<Vec<Result<(), StoreError>>> = std::thread::scope(|scope| {
        let handles: Vec<_> = stores
            .iter()
            .enumerate()
            .map(|(si, store)| {
                scope.spawn(move || {
                    (0..10)
                        .map(|u| {
                            store
                                .create(&format!("user-{si}-{u}"), "password8", Role::Member)
                                .map(|_| ())
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    let failures: Vec<&StoreError> = outcomes
        .iter()
        .flatten()
        .filter_map(|r| r.as_ref().err())
        .collect();
    assert!(
        failures.is_empty(),
        "共享配方下跨连接并发建户必须全部成功，实际失败: {failures:?}"
    );

    let check = UserStore::open_with_iterations(&path, 1000).unwrap();
    assert_eq!(check.count().unwrap(), 40, "40 个用户一个不少");
    assert_eq!(check.list().unwrap().len(), 40);

    // 跨连接重名（大小写不敏感）仍被唯一约束拒绝，且失败侧不留半行。
    assert!(matches!(
        stores[1].create("USER-0-0", "password8", Role::Viewer),
        Err(StoreError::UsernameTaken)
    ));
    assert_eq!(check.count().unwrap(), 40, "失败创建不得改行数");
}
