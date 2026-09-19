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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

/// 一条泊车审批的完整登记（fix-webui-approval-restore-and-session-identity
/// 1.1）：request_id 之外带上工具名与参数——「按会话枚举当前待批请求」的
/// 读模型直接从这里投影，刷新/重连后的审批面重建不再依赖一次性 WS 推送。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParkedRequest {
    /// 权限请求 id（agent-driver 命名空间，如 `claude:tc-N`）。
    pub request_id: String,
    /// 被门控的工具名。
    pub tool_name: String,
    /// 工具调用参数（原样 JSON）。
    pub args: Value,
}

#[derive(Default, Clone)]
pub struct StallRegistry {
    /// session_id → 最近一次事件到达的 unix 秒。
    clocks: Arc<RwLock<HashMap<String, i64>>>,
    /// session_id → 在等批复的权限请求登记（request_id → 工具/参数）。
    parked: Arc<RwLock<HashMap<String, HashMap<String, ParkedRequest>>>>,
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

    /// 权限请求泊车：登记 request_id + 工具/参数（该会话豁免计时，design D2）。
    /// 同 id 重登（重放的 PermissionRequest）幂等覆盖为最新登记。
    pub async fn note_permission_parked(
        &self,
        session_id: &str,
        request_id: &str,
        tool_name: &str,
        args: Value,
    ) {
        self.parked
            .write()
            .await
            .entry(session_id.to_string())
            .or_default()
            .insert(
                request_id.to_string(),
                ParkedRequest {
                    request_id: request_id.to_string(),
                    tool_name: tool_name.to_string(),
                    args,
                },
            );
    }

    /// 权限批复出站：从**任意**会话的泊车集合解除该 request_id（emit 单点，
    /// 调用方不必知道归属会话）。返回解除所在的 session_id（`None` = 该
    /// request_id 本就未泊车，no-op）——调用方（engine `emit`）据此对解除
    /// 的会话发布 Updated（waiting → 原 phase 的 flip 即刻广播）。
    pub async fn note_permission_resolved(&self, request_id: &str) -> Option<String> {
        let mut parked = self.parked.write().await;
        // remove 返回 true = 该集合曾有此请求；集合因此变空时连同会话条目
        // 一起清掉（防空壳积累）。未泊车的 id 解除是无害 no-op。
        let mut resolved_session = None;
        parked.retain(|sid, ids| {
            let had = ids.remove(request_id).is_some();
            if had {
                resolved_session = Some(sid.clone());
            }
            !had || !ids.is_empty()
        });
        resolved_session
    }

    /// 该会话当前泊车的权限请求全量（读模型投影源，落库序不保证——按
    /// request_id 字典序稳定输出，方便测试与呈现）。
    pub async fn parked_requests(&self, session_id: &str) -> Vec<ParkedRequest> {
        let g = self.parked.read().await;
        match g.get(session_id) {
            Some(ids) => {
                let mut out: Vec<ParkedRequest> = ids.values().cloned().collect();
                out.sort_by(|a, b| a.request_id.cmp(&b.request_id));
                out
            }
            None => Vec::new(),
        }
    }

    /// 该 request_id 当前泊车在哪个会话（`None` = 未泊车）。批复路由的
    /// fail-closed 校验源：未泊车的 id 不可批复。
    pub async fn parked_owner(&self, request_id: &str) -> Option<String> {
        self.parked
            .read()
            .await
            .iter()
            .find(|(_, ids)| ids.contains_key(request_id))
            .map(|(sid, _)| sid.clone())
    }

    /// fail-closed 释放：清空该会话的**全部**泊车登记（cancel / 会话终结
    /// 路径调用）。返回被释放的 request_id 列表（日志/断言用）；幂等——
    /// 重复释放返回空表、无副作用。孤儿泊车自此不再豁免计时，也绝不可再
    /// 批复（[`Self::note_permission_resolved`] 对已释放 id 本就是 no-op）。
    pub async fn release_session(&self, session_id: &str) -> Vec<String> {
        let mut parked = self.parked.write().await;
        match parked.remove(session_id) {
            Some(ids) => {
                let mut out: Vec<String> = ids.into_keys().collect();
                out.sort();
                out
            }
            None => Vec::new(),
        }
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
            .filter(|(sid, last)| now - *last > timeout && !parked.contains_key(sid.as_str()))
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
        reg.note_permission_parked("s1", "req-1", "Bash", serde_json::json!({"command": "ls"}))
            .await;
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
        reg.note_permission_parked("s1", "r1", "Bash", serde_json::json!({})).await;
        reg.note_permission_parked("s1", "r2", "Read", serde_json::json!({})).await;
        reg.note_permission_parked("s2", "r3", "Grep", serde_json::json!({})).await;
        reg.note_permission_resolved("does-not-exist").await;
        assert_eq!(reg.parked_count("s1").await, 2);
        assert_eq!(reg.parked_count("s2").await, 1);
        reg.note_permission_resolved("r1").await;
        assert_eq!(reg.parked_count("s1").await, 1, "only r1 leaves s1");
        assert_eq!(reg.parked_count("s2").await, 1, "s2 untouched");
    }

    /// fix-webui-approval-restore-and-session-identity 1.1：泊车登记携带
    /// 工具/参数——读模型枚举逐一可见、批复后消失；同 id 重登幂等覆盖。
    #[tokio::test]
    async fn parked_requests_enumerate_tool_and_args_and_clear_on_resolve() {
        let reg = StallRegistry::default();
        let args = serde_json::json!({"command": "rm -rf build", "dir": "/x"});
        reg.note_permission_parked("s1", "claude:tc-1", "Bash", args.clone()).await;
        // 同 id 重登 = 覆盖（幂等），不重复计数。
        reg.note_permission_parked("s1", "claude:tc-1", "Bash", args.clone()).await;
        reg.note_permission_parked("s1", "claude:tc-2", "Read", serde_json::json!({"path": "a.rs"})).await;

        let listed = reg.parked_requests("s1").await;
        assert_eq!(listed.len(), 2, "one card per request id");
        assert_eq!(listed[0].request_id, "claude:tc-1", "stable request_id order");
        assert_eq!(listed[0].tool_name, "Bash");
        assert_eq!(listed[0].args, args);
        assert_eq!(listed[1].tool_name, "Read");

        // 批复 → 该请求从读模型消失；另一条不受影响。
        assert_eq!(reg.note_permission_resolved("claude:tc-1").await.as_deref(), Some("s1"));
        let listed = reg.parked_requests("s1").await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].request_id, "claude:tc-2");

        // 未知会话 → 空表。
        assert!(reg.parked_requests("nope").await.is_empty());
    }

    /// fix-webui-approval-restore-and-session-identity 2.1：release_session
    /// 一次性释放全部泊车（fail-closed）、幂等；释放后 parked_owner 查无此 id。
    #[tokio::test]
    async fn release_session_clears_every_parked_request_idempotently() {
        let reg = StallRegistry::default();
        reg.note_permission_parked("s1", "r1", "Bash", serde_json::json!({})).await;
        reg.note_permission_parked("s1", "r2", "Read", serde_json::json!({})).await;
        reg.note_permission_parked("s2", "r3", "Grep", serde_json::json!({})).await;

        assert_eq!(reg.parked_owner("r1").await.as_deref(), Some("s1"));
        let released = reg.release_session("s1").await;
        assert_eq!(released, vec!["r1".to_string(), "r2".to_string()]);
        assert_eq!(reg.parked_count("s1").await, 0, "s1 fully released");
        assert_eq!(reg.parked_owner("r1").await, None, "released id is no longer answerable");
        assert_eq!(reg.parked_count("s2").await, 1, "other sessions untouched");

        // 幂等：重复释放返回空表。
        assert!(reg.release_session("s1").await.is_empty());
        // 未知会话释放 = no-op。
        assert!(reg.release_session("nope").await.is_empty());
    }
}
