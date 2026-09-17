//! 回合停滞看门狗的事实登记表（fix-pending-queue-liveness，design D1/D2）。
//!
//! 引擎是唯一同时看得到「卡片态、泊车态、事件到达」的位置（design D1）：
//!
//! - **事件时钟**（`clocks`）：每个会话记录最近一次**任何事件**到达的 unix
//!   秒。单点写入——ACP 事件漏斗（`apply_event` 流式臂 / `apply_event_to_out`
//!   即时臂）与回合开轮点（`emit_turn_card` / `seed_card`）各调一次
//!   [`StallRegistry::touch`]，不动事件热路径的其他逻辑。
//! - **泊车豁免**（`parked`）：会话 → 在等批复的权限 request_id 集合。
//!   `PermissionRequest` 事件到达即登记（design D2：泊车中不计时）；
//!   `PermissionReply` 出站（`emit` 单点）即解除——解除后若回合继续，后续
//!   事件自然刷新时钟（重计）。
//! - **阈值**（`timeout_secs`）：`[dispatch] turn_stall_timeout`，`0` = 关闭
//!   （扫描短路）。运行期可调（装配点 set 一次）。
//!
//! 纯事实 + 纯内存：扫描与强制收尾在 [`super::DispatchHandle`]（它才能摸到
//! 卡片态与队列），本类型只回答「谁停滞了」。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

#[derive(Default, Clone)]
pub struct StallRegistry {
    /// session_id → 最近一次事件到达的 unix 秒。
    clocks: Arc<RwLock<HashMap<String, i64>>>,
    /// session_id → 在等批复的权限 request_id 集合（泊车豁免事实）。
    parked: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    /// 看门狗阈值（秒）；0 = 关闭。
    timeout_secs: Arc<AtomicU64>,
}

/// 一个停滞会话的归因快照（扫描结果；收尾由调用方执行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StallFacts {
    pub session_id: String,
    /// 最近一次事件到达的 unix 秒（收尾日志的归因锚点，design Risks）。
    pub last_event_unix: i64,
    /// 距最近一次事件的秒数。
    pub silent_for_secs: i64,
}

impl StallRegistry {
    /// 设置阈值（秒）；`0` = 关闭。装配点调用；后到覆盖先到。
    pub fn set_timeout_secs(&self, secs: u64) {
        self.timeout_secs.store(secs, Ordering::Relaxed);
    }

    /// 当前阈值（秒）；`0` = 关闭。
    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs.load(Ordering::Relaxed)
    }

    /// 看门狗是否关闭（`timeout == 0`）：扫描短路依据。
    pub fn is_disabled(&self) -> bool {
        self.timeout_secs() == 0
    }

    /// 事件到达 / 回合开轮：刷新会话时钟。幂等覆盖——两个漏斗臂（流式
    /// `apply_event`、即时 `apply_event_to_out`）与开轮点都会调，重复触碰
    /// 无害（同一时刻多写一次同值邻域）。
    pub async fn touch(&self, session_id: &str) {
        self.clocks
            .write()
            .await
            .insert(session_id.to_string(), super::now_unix());
    }

    /// 权限请求泊车：登记 request_id（该会话豁免计时，design D2）。
    pub async fn note_permission_parked(&self, session_id: &str, request_id: &str) {
        self.parked
            .write()
            .await
            .entry(session_id.to_string())
            .or_default()
            .insert(request_id.to_string());
    }

    /// 权限批复出站：从**任意**会话的泊车集合解除该 request_id（emit 单点，
    /// 调用方不必知道归属会话）。
    pub async fn note_permission_resolved(&self, request_id: &str) {
        let mut parked = self.parked.write().await;
        // remove 返回 true = 该集合曾有此请求；集合因此变空时连同会话条目
        // 一起清掉（防空壳积累）。未泊车的 id 解除是无害 no-op。
        parked.retain(|_, ids| {
            let had = ids.remove(request_id);
            !had || !ids.is_empty()
        });
    }

    /// 该会话当前泊车中的权限请求数。
    pub async fn parked_count(&self, session_id: &str) -> usize {
        self.parked
            .read()
            .await
            .get(session_id)
            .map(|ids| ids.len())
            .unwrap_or(0)
    }

    /// 会话终结：清掉时钟与泊车登记（映射移除路径调用，防无界积累）。
    pub async fn drop_session(&self, session_id: &str) {
        self.clocks.write().await.remove(session_id);
        self.parked.write().await.remove(session_id);
    }

    /// 扫描：找出「时钟缺失按未停滞处理」的停滞会话——阈值 > 0、距最近
    /// 事件超过阈值、**无泊车审批**（design D2）。调用方再对结果逐会话核对
    /// 卡片 WORKING 态后收尾（这里不知道卡片）。
    pub async fn stalled_sessions(&self) -> Vec<StallFacts> {
        if self.is_disabled() {
            return Vec::new();
        }
        let timeout = self.timeout_secs() as i64;
        let now = super::now_unix();
        let clocks = self.clocks.read().await;
        let parked = self.parked.read().await;
        clocks
            .iter()
            .filter(|(sid, last)| {
                now - *last > timeout && !parked.contains_key(sid.as_str())
            })
            .map(|(sid, last)| StallFacts {
                session_id: sid.clone(),
                last_event_unix: *last,
                silent_for_secs: now - *last,
            })
            .collect()
    }

    /// 把会话时钟回拨 `secs` 秒（仅测试：秒级阈值下模拟「长时间无事件」，
    /// 免去真实等待）。
    #[doc(hidden)]
    pub async fn rewind_last_event_for_test(&self, session_id: &str, secs: i64) {
        if let Some(last) = self.clocks.write().await.get_mut(session_id) {
            *last -= secs;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fix-pending-queue-liveness 2.1：泊车期间不计时——登记泊车后扫描不再
    /// 报该会话；解除（批复出站）后重新计时（回到扫描视野）。
    #[tokio::test]
    async fn parked_permission_suspends_and_resolution_restarts_the_clock() {
        let reg = StallRegistry::default();
        reg.set_timeout_secs(10);
        // 时钟拨到远超阈值的过去。
        {
            let mut g = reg.clocks.write().await;
            g.insert("s1".into(), super::super::now_unix() - 3600);
        }
        assert_eq!(reg.stalled_sessions().await.len(), 1);

        // 泊车 → 豁免：不进停滞名单。
        reg.note_permission_parked("s1", "req-1").await;
        assert_eq!(reg.parked_count("s1").await, 1);
        assert!(
            reg.stalled_sessions().await.is_empty(),
            "parked approval must exempt the session from the stall guard"
        );

        // 批复出站 → 解除、重计：回到停滞名单（时钟仍是旧值）。
        reg.note_permission_resolved("req-1").await;
        assert_eq!(reg.parked_count("s1").await, 0);
        assert_eq!(
            reg.stalled_sessions().await.len(),
            1,
            "after the permission resolves the guard must see the session again"
        );
    }

    /// fix-pending-queue-liveness 2.2：`timeout = 0` 扫描短路（guard 关闭）。
    #[tokio::test]
    async fn zero_timeout_short_circuits_the_scan() {
        let reg = StallRegistry::default();
        reg.set_timeout_secs(0);
        {
            let mut g = reg.clocks.write().await;
            g.insert("s1".into(), 0);
        }
        assert!(reg.is_disabled());
        assert!(
            reg.stalled_sessions().await.is_empty(),
            "timeout=0 must disable the stall guard entirely"
        );
    }

    /// touch 幂等：两个漏斗臂 + 开轮点重复触碰不炸、不出多份事实。
    #[tokio::test]
    async fn touch_is_idempotent_per_session() {
        let reg = StallRegistry::default();
        reg.set_timeout_secs(600);
        reg.touch("s1").await;
        reg.touch("s1").await;
        reg.touch("s1").await;
        let stalled = reg.stalled_sessions().await;
        assert_eq!(stalled.len(), 0, "fresh clock must not be stalled");
        reg.drop_session("s1").await;
        assert!(reg.clocks.read().await.is_empty());
        assert!(reg.parked.read().await.is_empty());
    }

    /// 解除未泊车的 request_id 是无害 no-op；多会话/多请求互不串扰。
    #[tokio::test]
    async fn resolve_is_targeted_and_tolerates_unknown_ids() {
        let reg = StallRegistry::default();
        reg.note_permission_parked("s1", "r1").await;
        reg.note_permission_parked("s1", "r2").await;
        reg.note_permission_parked("s2", "r3").await;
        reg.note_permission_resolved("does-not-exist").await;
        assert_eq!(reg.parked_count("s1").await, 2);
        assert_eq!(reg.parked_count("s2").await, 1);
        reg.note_permission_resolved("r1").await;
        assert_eq!(reg.parked_count("s1").await, 1, "only r1 leaves s1");
        assert_eq!(reg.parked_count("s2").await, 1, "s2 untouched");
    }
}
