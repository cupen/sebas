//! 控制面侧的远程会话视图与对账（add-remote-execution-node group 5）。
//!
//! 权威分域（设计 D1）：节点持有执行事实，控制面持有**可重建的副本**。本模块就是
//! 那份副本的维护逻辑，对账只用两种原语（设计 D5）：
//!
//! - **有序追加**：turn 批与回拉结果都按 `seq` 追加；重复应用同一段结果不变（幂等）；
//! - **快照**：状态（相位 / 纪元 / 水位线）整体替换，不做增量合并。
//!
//! 三条必须守住的语义：
//!
//! 1. **缺口可回答**：批与批之间的空洞要么是「尚未送达」（pending），要么是
//!    「节点已回收」（unavailable）——由节点上报的回收水位线区分，绝不静默填平。
//! 2. **纪元断裂**：节点日志重置会换纪元；此时**开启新段**而不是把两条时间线接成
//!    一条（`segments` 的存在理由）。
//! 3. **重复不重放**：`from_seq <= cursor` 的批只取其中的新条目，已入册的不再追加。
//!
//! 合并（传输层节流）只影响**到达节奏**，不影响本模块的正确性：任何被合并掉的片段
//! 都能按 `seq` 回拉（[`RemoteSession::reconcile`]）。

use crate::node_link::client::{NodeConnection, NodeLinkError};
use sebas_node_link::{
    ApprovalDecision, LogEntry, ParkedApproval, SessionEvent, SessionOp, SessionResult,
};

/// 一段时间线（同一纪元内的连续条目）。
///
/// 只派生 `PartialEq`：条目里带 `serde_json::Value`（工具参数/用量），它不实现 `Eq`。
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// 该段的日志纪元。
    pub epoch: u64,
    /// 段内条目（按 seq 升序）。
    pub entries: Vec<LogEntry>,
}

/// 一段不可得的缺口（节点已回收）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unavailable {
    /// 起点（含）。
    pub from_seq: u64,
    /// 终点（含）。
    pub through_seq: u64,
}

/// 一次批入账的结果——**缺口与重复都必须被看见**，不能悄悄吞掉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchOutcome {
    /// 顺序衔接：全部条目入册。
    Applied {
        /// 本批新增条目数。
        added: usize,
    },
    /// 与游标有重叠：只入册了其中未见过的部分。
    Overlapped {
        /// 实际新增条目数。
        added: usize,
        /// 被跳过的重复条目数。
        skipped: usize,
    },
    /// 与游标之间有空洞：条目已入册，但空洞被记为 pending 或缺损。
    Gapped {
        /// 实际新增条目数。
        added: usize,
        /// 空洞起点（含）。
        gap_from: u64,
        /// 空洞终点（含）。
        gap_through: u64,
        /// 该空洞是否已被节点的回收水位线覆盖（覆盖即永久缺损）。
        unavailable: bool,
    },
    /// 空批（节点可能只是没话说）。
    Empty,
}

/// 控制面持有的远程会话副本。
#[derive(Debug, Clone)]
pub struct RemoteSession {
    session_id: String,
    /// 按纪元分段的本地副本。
    segments: Vec<Segment>,
    /// 当前纪元。
    epoch: u64,
    /// 已同步到的 seq（游标；以当前纪元为准）。
    cursor: u64,
    /// 节点已回收到（含）的 seq。
    reclaimed_through: u64,
    /// 尚未补齐的空洞（pending：等回拉）。
    gaps: Vec<Unavailable>,
    /// 永久缺损（节点已回收）。
    unavailable: Vec<Unavailable>,
    /// 相位（来自事件或快照）。
    phase: String,
    /// 结束成因（`Some` 即已终止）。
    terminated: Option<String>,
    /// **悬空的审批请求**（控制面视图）。
    ///
    /// 悬空的事实由**节点**持有（它是唯一知道 agent 是否还停在那里等的人），
    /// 这里是一份可重建的副本：事件流增量维护，重连后用 [`Self::reconcile_approvals`]
    /// 以节点为准整体校正。
    parked: Vec<ParkedApproval>,
    /// 节点丢弃过的决定（会话已终止等）。留痕而不是悄悄消失——操作者点过一次
    /// 「允许」，就该能查到它为什么没生效。
    discarded_decisions: Vec<String>,
    /// 该会话**钉住**的操作者级材料版本（来自快照；未使用 → `None`）。
    materials_version: Option<String>,
}

impl RemoteSession {
    /// 建立一个空副本。`epoch` 未知时先记 0，首次对账会校正。
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            segments: vec![Segment {
                epoch: 0,
                entries: Vec::new(),
            }],
            epoch: 0,
            cursor: 0,
            reclaimed_through: 0,
            gaps: Vec::new(),
            unavailable: Vec::new(),
            phase: "spawning".into(),
            terminated: None,
            parked: Vec::new(),
            discarded_decisions: Vec::new(),
            materials_version: None,
        }
    }

    /// 会话 id。
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 当前游标（已同步到的 seq）。
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// 当前纪元。
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// 相位。
    pub fn phase(&self) -> &str {
        &self.phase
    }

    /// 结束成因（未终止为 `None`）。
    pub fn terminated(&self) -> Option<&str> {
        self.terminated.as_deref()
    }

    /// 悬空的审批请求（按 request_id 升序）。
    pub fn parked_approvals(&self) -> &[ParkedApproval] {
        &self.parked
    }

    /// 被节点丢弃过的决定（会话已终止等）。
    pub fn discarded_decisions(&self) -> &[String] {
        &self.discarded_decisions
    }

    /// 该会话钉住的材料版本（`None` = 不使用操作者级材料）。
    pub fn materials_version(&self) -> Option<&str> {
        self.materials_version.as_deref()
    }

    /// 本地副本的段（纪元断裂会新增段）。
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// 拍平后的条目（跨段按段序）。
    pub fn entries(&self) -> Vec<LogEntry> {
        self.segments
            .iter()
            .flat_map(|s| s.entries.iter().cloned())
            .collect()
    }

    /// 当前段里某个 seq 的条目。
    pub fn entry(&self, seq: u64) -> Option<&LogEntry> {
        self.current_segment().entries.iter().find(|e| e.seq == seq)
    }

    /// 尚未补齐的空洞（pending）。
    pub fn gaps(&self) -> &[Unavailable] {
        &self.gaps
    }

    /// 永久缺损（节点已回收）。
    pub fn unavailable(&self) -> &[Unavailable] {
        &self.unavailable
    }

    fn current_segment(&self) -> &Segment {
        self.segments.last().expect("至少一个段")
    }

    fn current_segment_mut(&mut self) -> &mut Segment {
        self.segments.last_mut().expect("至少一个段")
    }

    /// 入账一个 turn 批（事件流路径）。
    pub fn note_batch(
        &mut self,
        from_seq: u64,
        entries: &[LogEntry],
        coalesced_overflow: bool,
    ) -> BatchOutcome {
        if entries.is_empty() {
            return BatchOutcome::Empty;
        }
        let last_seq = entries.last().map(|e| e.seq).unwrap_or(from_seq);

        // 空洞：批起点越过游标 +1。
        let gap = if from_seq > self.cursor + 1 {
            Some((self.cursor + 1, from_seq - 1))
        } else {
            None
        };
        // 缺口是否已被回收水位线覆盖 → 永久缺损而不是 pending。
        let gap_unavailable = gap
            .map(|(_, through)| through <= self.reclaimed_through)
            .unwrap_or(false);

        // 入册：跳过已见过的（幂等）。
        let before = self.current_segment().entries.len();
        let mut skipped = 0usize;
        for entry in entries {
            if self.entry(entry.seq).is_some() {
                skipped += 1;
                continue;
            }
            // 只收当前纪元里 seq 大于游标的条目（跨纪元由 note_epoch 处理）。
            if entry.seq <= self.cursor && self.entry(entry.seq).is_none() {
                // 游标之前但本地没有：可能是回收掉的历史，记缺损而不是插入错位。
                self.record_unavailable(entry.seq, entry.seq);
                skipped += 1;
                continue;
            }
            self.current_segment_mut().entries.push(entry.clone());
        }
        let added = self.current_segment().entries.len() - before;
        if last_seq > self.cursor {
            self.cursor = last_seq;
        }

        if let Some((gap_from, gap_through)) = gap {
            if gap_unavailable {
                self.record_unavailable(gap_from, gap_through);
                return BatchOutcome::Gapped {
                    added,
                    gap_from,
                    gap_through,
                    unavailable: true,
                };
            }
            self.gaps.push(Unavailable {
                from_seq: gap_from,
                through_seq: gap_through,
            });
            return BatchOutcome::Gapped {
                added,
                gap_from,
                gap_through,
                unavailable: false,
            };
        }

        if skipped > 0 || coalesced_overflow {
            BatchOutcome::Overlapped { added, skipped }
        } else {
            BatchOutcome::Applied { added }
        }
    }

    /// 收到回收水位线：把被覆盖的空洞从 pending 迁到永久缺损。
    pub fn note_reclaimed(&mut self, through_seq: u64) {
        self.reclaimed_through = self.reclaimed_through.max(through_seq);
        let mut still_pending = Vec::new();
        for gap in self.gaps.drain(..) {
            if gap.through_seq <= self.reclaimed_through {
                self.unavailable.push(gap);
            } else if gap.from_seq <= self.reclaimed_through {
                // 部分被覆盖：切成两段，已覆盖的部分记缺损，其余留 pending。
                self.unavailable.push(Unavailable {
                    from_seq: gap.from_seq,
                    through_seq: self.reclaimed_through,
                });
                still_pending.push(Unavailable {
                    from_seq: self.reclaimed_through + 1,
                    through_seq: gap.through_seq,
                });
            } else {
                still_pending.push(gap);
            }
        }
        self.gaps = still_pending;
    }

    /// 纪元变更：**开新段**，不把两条时间线接起来。
    ///
    /// 返回是否确实发生了断裂（同纪元重复上报 → `false`，幂等）。
    pub fn note_epoch(&mut self, epoch: u64) -> bool {
        if epoch == self.epoch {
            return false;
        }
        // 本地纪元为 0 = **未知**。第一次学到真实纪元是**校正**，不是断裂：
        // turn 批本身不携带纪元（只有事件与回拉才带），所以视图完全可能已经持有
        // 若干条目却仍不知道纪元——按「段为空」判断会在此时凭空多切一段。
        if self.epoch == 0 {
            self.epoch = epoch;
            self.current_segment_mut().epoch = epoch;
            return false;
        }
        self.epoch = epoch;
        self.cursor = 0;
        self.reclaimed_through = 0;
        self.gaps.clear();
        self.segments.push(Segment {
            epoch,
            entries: Vec::new(),
        });
        true
    }

    /// 相位变化。
    pub fn note_phase(&mut self, phase: impl Into<String>) {
        self.phase = phase.into();
    }

    /// 会话结束。
    pub fn note_exited(&mut self, cause: impl Into<String>) {
        self.phase = "exited".into();
        self.terminated = Some(cause.into());
    }

    /// 消费一个节点事件。返回是否改变了本地视图。
    pub fn note_event(&mut self, event: &SessionEvent) -> bool {
        match event {
            SessionEvent::TurnBatch {
                epoch,
                from_seq,
                entries,
                coalesced_overflow,
                ..
            } => {
                // 批携带纪元：先校正/断裂，再入账——顺序反了会把断裂后的条目
                // 追加进旧段。
                let epoch_changed = *epoch != 0 && self.note_epoch(*epoch);
                let outcome = self.note_batch(*from_seq, entries, *coalesced_overflow);
                epoch_changed || !matches!(outcome, BatchOutcome::Empty)
            }
            SessionEvent::State { phase, .. } => {
                self.note_phase(phase.clone());
                true
            }
            SessionEvent::Exited { cause, .. } => {
                self.note_exited(cause.clone());
                true
            }
            SessionEvent::Reclaimed {
                reclaimed_through_seq,
                ..
            } => {
                self.note_reclaimed(*reclaimed_through_seq);
                true
            }
            SessionEvent::EpochChanged { epoch, .. } => self.note_epoch(*epoch),
            SessionEvent::ApprovalRequested {
                session_id,
                request_id,
                tool,
                category,
                mode,
            } => {
                self.note_approval_requested(ParkedApproval {
                    session_id: session_id.clone(),
                    request_id: request_id.clone(),
                    tool: tool.clone(),
                    category: *category,
                    mode: *mode,
                });
                true
            }
            SessionEvent::GateResolved { request_id, .. } => {
                self.note_gate_resolved(request_id);
                true
            }
            SessionEvent::MaterialsChanged { version } => {
                // 这个事件的方向是**控制面 → 节点**（通知有新版本、不带内容）。
                // 控制面侧本不该收到它；如实记录而不是假装处理了。
                eprintln!(
                    "node-link: 控制面收到了 MaterialsChanged({version}) —— 方向不对，已忽略"
                );
                false
            }
        }
    }

    /// 入账一个悬空审批请求（按 `request_id` 去重：重连重放不应产生两条）。
    fn note_approval_requested(&mut self, approval: ParkedApproval) {
        if self
            .parked
            .iter()
            .any(|p| p.request_id == approval.request_id)
        {
            return;
        }
        self.parked.push(approval);
        self.parked.sort_by(|a, b| a.request_id.cmp(&b.request_id));
    }

    /// 门控已有结论：从悬空集合移除。
    fn note_gate_resolved(&mut self, request_id: &str) {
        self.parked.retain(|p| p.request_id != request_id);
    }

    /// 对账悬空审批：**以节点为准**整体替换本地集合。
    ///
    /// 「已决议者不重现」由这一步保证：本地以为还悬着、节点已经不认的，会被丢掉；
    /// 主控缺席期间节点 park 的，会在这里全部出现。
    pub async fn reconcile_approvals(
        &mut self,
        connection: &NodeConnection,
    ) -> Result<usize, NodeLinkError> {
        let result = connection.request(SessionOp::ParkedApprovals).await?;
        match result {
            SessionResult::ParkedApprovals { approvals } => {
                self.parked = approvals
                    .into_iter()
                    .filter(|p| p.session_id == self.session_id)
                    .collect();
                self.parked.sort_by(|a, b| a.request_id.cmp(&b.request_id));
                Ok(self.parked.len())
            }
            SessionResult::Rejected { code, cause } => Err(NodeLinkError::Transport {
                cause: format!("查询悬空审批被拒绝（{}）：{cause}", code.as_str()),
            }),
            other => Err(NodeLinkError::Transport {
                cause: format!("查询悬空审批得到非预期应答：{other:?}"),
            }),
        }
    }

    /// 回填一个决定。
    ///
    /// 返回 `Ok(true)` = 节点已生效；`Ok(false)` = 节点**丢弃**了它（会话已终止等），
    /// 本地同样移除并留痕；`Err` = 链路层失败（决定是否送达未知）。
    pub async fn answer_approval(
        &mut self,
        connection: &NodeConnection,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<bool, NodeLinkError> {
        let Some(approval) = self
            .parked
            .iter()
            .find(|p| p.request_id == request_id)
            .cloned()
        else {
            return Err(NodeLinkError::Transport {
                cause: format!("本视图里没有悬空请求 {request_id}（先 reconcile_approvals 校正）"),
            });
        };

        let result = connection
            .request(SessionOp::ApprovalAnswer {
                session_id: approval.session_id.clone(),
                request_id: request_id.to_string(),
                decision,
            })
            .await?;
        match result {
            SessionResult::ApprovalApplied { applied } => {
                self.note_gate_resolved(request_id);
                if !applied {
                    self.discarded_decisions.push(format!(
                        "{request_id}:{} 被节点丢弃（会话已终止）",
                        decision.as_str()
                    ));
                }
                Ok(applied)
            }
            SessionResult::Rejected { code, cause } => Err(NodeLinkError::Transport {
                cause: format!("决定被拒绝（{}）：{cause}", code.as_str()),
            }),
            other => Err(NodeLinkError::Transport {
                cause: format!("回填决定得到非预期应答：{other:?}"),
            }),
        }
    }

    /// 从节点回拉游标之后的精确序列并合入（幂等：重复调用结果不变）。
    pub async fn reconcile(
        &mut self,
        connection: &NodeConnection,
    ) -> Result<BatchOutcome, NodeLinkError> {
        let from_seq = self.cursor + 1;
        let result = connection
            .request(SessionOp::LogFrom {
                session_id: self.session_id.clone(),
                from_seq,
            })
            .await?;
        match result {
            SessionResult::Log {
                epoch,
                entries,
                last_seq,
            } => {
                // 纪元对不上：先记录断裂，再按新纪元的段入账。
                let _ = self.note_epoch(epoch);
                let outcome = self.note_batch(from_seq, &entries, false);
                if entries.is_empty() && last_seq < from_seq {
                    // 节点已经没有更多内容：游标与节点一致即无待补。
                    return Ok(BatchOutcome::Empty);
                }
                Ok(outcome)
            }
            SessionResult::Rejected { code, cause } => Err(NodeLinkError::Transport {
                cause: format!("回拉被拒绝（{}）：{cause}", code.as_str()),
            }),
            other => Err(NodeLinkError::Transport {
                cause: format!("回拉得到非预期应答：{other:?}"),
            }),
        }
    }

    /// 用快照校正**状态**：纪元、相位、材料版本、回收水位线（幂等，不做增量合并）。
    ///
    /// **不动游标**，这是刻意的：游标回答的是「我手上确实有哪些条目」，而快照回答
    /// 的是「节点那边现在是什么样」——两件事。混为一谈会在**重建出来的空视图**上
    /// 宣称"我已经有到 `last_seq` 了"，于是紧接着的增量回拉从末尾开始，一条也拉不
    /// 回来：控制面重启后看到的会是一个**没有任何转写**的会话（而它其实好好躺在
    /// 节点磁盘上）。用 [`Self::note_snapshot`] 当"我有全部内容"的确认才推进游标。
    pub fn note_state(&mut self, summary: &sebas_node_link::SessionSummary, reclaimed_through: u64) {
        self.note_epoch(summary.epoch);
        self.phase = summary.phase.clone();
        self.materials_version = summary.materials_version.clone();
        self.note_reclaimed(reclaimed_through);
    }

    /// 用快照校正状态**并**把游标推到节点报告的末序号。
    ///
    /// 只在"手上的条目确实已经覆盖到 `last_seq`"时用（例如回拉之后再校正）。
    pub fn note_snapshot(&mut self, summary: &sebas_node_link::SessionSummary, reclaimed_through: u64) {
        self.note_state(summary, reclaimed_through);
        if self.cursor < summary.last_seq {
            self.cursor = summary.last_seq;
        }
    }

    fn record_unavailable(&mut self, from_seq: u64, through_seq: u64) {
        // 合并相邻/重叠的缺损区间，避免同一段被记多次。
        let mut merged = Unavailable {
            from_seq,
            through_seq,
        };
        let mut rest = Vec::new();
        for span in self.unavailable.drain(..) {
            if span.through_seq + 1 >= merged.from_seq && span.from_seq <= merged.through_seq + 1 {
                merged.from_seq = merged.from_seq.min(span.from_seq);
                merged.through_seq = merged.through_seq.max(span.through_seq);
            } else {
                rest.push(span);
            }
        }
        rest.push(merged);
        rest.sort_by_key(|s| s.from_seq);
        self.unavailable = rest;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_link::server::NodeLinkServer;
    use futures_util::{SinkExt, StreamExt};
    use sebas_node_link::{
        CapabilityManifest, Frame, Hello, HelloOutcome, NodeAuth, PROTOCOL_VERSION,
    };
    use tokio_tungstenite::tungstenite::Message as ClientMessage;

    fn entry(seq: u64, text: &str) -> LogEntry {
        LogEntry {
            seq,
            kind: "output".into(),
            text: text.into(),
            data: None,
            at_unix: 0,
        }
    }

    #[test]
    fn sequential_batches_advance_the_cursor() {
        let mut view = RemoteSession::new("s-1");
        assert_eq!(
            view.note_batch(1, &[entry(1, "a"), entry(2, "b")], false),
            BatchOutcome::Applied { added: 2 }
        );
        assert_eq!(view.cursor(), 2);
        assert_eq!(
            view.note_batch(3, &[entry(3, "c")], false),
            BatchOutcome::Applied { added: 1 }
        );
        assert_eq!(view.cursor(), 3);
        assert!(view.gaps().is_empty());
        assert_eq!(view.entries().len(), 3);
    }

    #[test]
    fn duplicate_batch_is_idempotent() {
        let mut view = RemoteSession::new("s-1");
        let batch = [entry(1, "a"), entry(2, "b")];
        assert_eq!(
            view.note_batch(1, &batch, false),
            BatchOutcome::Applied { added: 2 }
        );
        // 重连后节点可能重发同一段：结果必须与只应用一次相同。
        let outcome = view.note_batch(1, &batch, false);
        assert_eq!(outcome, BatchOutcome::Overlapped { added: 0, skipped: 2 });
        assert_eq!(view.cursor(), 2);
        assert_eq!(view.entries().len(), 2, "重复不应追加");
    }

    #[test]
    fn partial_overlap_only_adds_the_new_part() {
        let mut view = RemoteSession::new("s-1");
        view.note_batch(1, &[entry(1, "a"), entry(2, "b")], false);
        let outcome = view.note_batch(2, &[entry(2, "b"), entry(3, "c")], false);
        assert_eq!(outcome, BatchOutcome::Overlapped { added: 1, skipped: 1 });
        assert_eq!(view.cursor(), 3);
        assert_eq!(view.entries().len(), 3);
    }

    #[test]
    fn a_gap_is_recorded_not_papered_over() {
        let mut view = RemoteSession::new("s-1");
        view.note_batch(1, &[entry(1, "a")], false);
        // 跳过了 2..=4。
        let outcome = view.note_batch(5, &[entry(5, "e")], false);
        assert_eq!(
            outcome,
            BatchOutcome::Gapped {
                added: 1,
                gap_from: 2,
                gap_through: 4,
                unavailable: false
            }
        );
        assert_eq!(
            view.gaps(),
            &[Unavailable {
                from_seq: 2,
                through_seq: 4
            }]
        );
        assert!(view.unavailable().is_empty());
    }

    /// 快照**不许**替我们宣称"历史已经在手上了"：否则重建出来的空视图会跳过
    /// 回拉，控制面重启后转写凭空消失（进程级 e2e 抓到过）。
    #[test]
    fn a_snapshot_corrects_state_without_claiming_we_hold_the_entries() {
        let summary = sebas_node_link::SessionSummary {
            session_id: "s-1".into(),
            phase: "active".into(),
            agent_kind: Some("echo".into()),
            model: None,
            mode: Some("ask".into()),
            desired_mode: Some("ask".into()),
            provider: None,
            desired_provider: None,
            provider_cause: None,
            materials_version: Some("v2".into()),
            last_seq: 42,
            epoch: 3,
        };

        // 空视图：校正状态，但游标仍是 0 —— 于是回拉会从 1 开始，历史拉得回来。
        let mut view = RemoteSession::new("s-1");
        view.note_state(&summary, 0);
        assert_eq!(view.cursor(), 0, "状态快照不得推进游标");
        assert_eq!(view.phase(), "active");
        assert_eq!(view.epoch(), 3);
        assert_eq!(view.materials_version(), Some("v2"));

        // 回拉之后再校正：这时推进游标才是对的（条目确实在手上）。
        view.note_batch(1, &[entry(1, "a")], false);
        view.note_snapshot(&summary, 0);
        assert_eq!(view.cursor(), 42, "确认持有后才推进游标");
    }

    #[test]
    fn reclaimed_watermark_turns_a_gap_into_a_permanent_loss() {
        let mut view = RemoteSession::new("s-1");
        view.note_batch(1, &[entry(1, "a")], false);
        view.note_reclaimed(3); // 节点已回收 1..=3
        let outcome = view.note_batch(5, &[entry(5, "e")], false);
        assert_eq!(
            outcome,
            BatchOutcome::Gapped {
                added: 1,
                gap_from: 2,
                gap_through: 4,
                unavailable: false, // 4 不在水位线内 → 仍是 pending
            }
        );
        // 水位线再推进到 4：该空洞转为永久缺损。
        view.note_reclaimed(4);
        assert!(view.gaps().is_empty());
        assert_eq!(
            view.unavailable(),
            &[Unavailable {
                from_seq: 2,
                through_seq: 4
            }]
        );
    }

    #[test]
    fn partial_reclaim_splits_the_gap() {
        let mut view = RemoteSession::new("s-1");
        view.note_batch(1, &[entry(1, "a")], false);
        view.note_batch(11, &[entry(11, "k")], false); // 空洞 2..=10
        view.note_reclaimed(6);
        assert_eq!(
            view.unavailable(),
            &[Unavailable {
                from_seq: 2,
                through_seq: 6
            }]
        );
        assert_eq!(
            view.gaps(),
            &[Unavailable {
                from_seq: 7,
                through_seq: 10
            }]
        );
    }

    #[test]
    fn epoch_change_opens_a_new_segment_instead_of_appending() {
        let mut view = RemoteSession::new("s-1");
        view.note_batch(1, &[entry(1, "old-a")], false);
        assert!(!view.note_epoch(1), "同纪元重复上报是幂等的");
        assert!(view.note_epoch(2), "纪元变化即断裂");

        assert_eq!(view.epoch(), 2);
        assert_eq!(view.cursor(), 0, "新纪元从零开始");
        assert_eq!(view.segments().len(), 2, "两条时间线不接成一条");
        assert_eq!(view.segments()[0].epoch, 1);
        assert_eq!(view.segments()[0].entries.len(), 1);
        assert_eq!(view.segments()[1].epoch, 2);
        assert!(view.segments()[1].entries.is_empty());

        view.note_batch(1, &[entry(1, "new-a")], false);
        assert_eq!(view.entries().len(), 2, "拍平后两段都在");
        assert_eq!(view.entry(1).unwrap().text, "new-a", "当前段的 1 是新内容");
    }

    #[test]
    fn events_drive_the_view() {
        let mut view = RemoteSession::new("s-1");
        assert!(view.note_event(&SessionEvent::State {
            session_id: "s-1".into(),
            phase: "active".into(),
            detail: None,
        }));
        assert_eq!(view.phase(), "active");

        assert!(view.note_event(&SessionEvent::TurnBatch {
            session_id: "s-1".into(),
            epoch: 1,
            from_seq: 1,
            entries: vec![entry(1, "hi")],
            coalesced_overflow: false,
        }));
        assert_eq!(view.cursor(), 1);

        assert!(view.note_event(&SessionEvent::Reclaimed {
            session_id: "s-1".into(),
            reclaimed_through_seq: 1,
        }));
        assert!(
            !view.note_event(&SessionEvent::EpochChanged {
                session_id: "s-1".into(),
                epoch: 1
            }),
            "同纪元事件不改变视图"
        );

        assert!(view.note_event(&SessionEvent::Exited {
            session_id: "s-1".into(),
            cause: "节点重启".into(),
        }));
        assert_eq!(view.terminated(), Some("节点重启"));
        assert_eq!(view.phase(), "exited");
    }

    // ── 回拉（对账）对真链路 ────────────────────────────────────────────────

    async fn connected() -> (
        tempfile::TempDir,
        std::sync::Arc<NodeLinkServer>,
        std::sync::Arc<NodeConnection>,
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ) {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let server = Arc::new(
            NodeLinkServer::bind("127.0.0.1:0", dir.path().join("nodes.json"))
                .await
                .unwrap(),
        );
        let secret = server
            .registry()
            .lock()
            .await
            .pair("dev-box", crate::node_link::server::now_unix())
            .unwrap();
        let url = format!("ws://{}", server.local_addr().unwrap());
        let srv = Arc::clone(&server);
        tokio::spawn(async move {
            let _ = srv.accept_one().await;
        });

        let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        ws.send(ClientMessage::Text(
            serde_json::to_string(&Hello {
                protocol_version: PROTOCOL_VERSION,
                node_id: "dev-box".into(),
                auth: NodeAuth::Credential { secret },
                manifest: CapabilityManifest::default(),
            })
            .unwrap()
            .into(),
        ))
        .await
        .unwrap();
        match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => {
                let ack: sebas_node_link::HelloAck = serde_json::from_str(&t).unwrap();
                assert!(matches!(ack.outcome, HelloOutcome::Accepted { .. }));
            }
            other => panic!("{other:?}"),
        }
        let conn = loop {
            if let Some(c) = server.live_connection("dev-box").await {
                break c;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        (dir, server, conn, ws)
    }

    #[tokio::test]
    async fn reconcile_pulls_from_the_cursor_and_is_idempotent() {
        let (_d, _s, conn, mut ws) = connected().await;
        let mut view = RemoteSession::new("s-1");

        let c = std::sync::Arc::clone(&conn);
        let driver = tokio::spawn(async move {
            let first = view.reconcile(&c).await;
            let second = view.reconcile(&c).await;
            (first, second, view)
        });

        // 假节点：两次回拉都返回同一段 1..=2（模拟重连后的重复投递）。
        for _ in 0..2 {
            match ws.next().await {
                Some(Ok(ClientMessage::Text(t))) => match serde_json::from_str::<Frame>(&t).unwrap()
                {
                    Frame::Request { id, op } => {
                        assert!(matches!(op, SessionOp::LogFrom { .. }));
                        let ack = Frame::Response {
                            id,
                            result: SessionResult::Log {
                                epoch: 1,
                                entries: vec![entry(1, "a"), entry(2, "b")],
                                last_seq: 2,
                            },
                        };
                        ws.send(ClientMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                            .await
                            .unwrap();
                    }
                    other => panic!("{other:?}"),
                },
                other => panic!("{other:?}"),
            }
        }

        let (first, second, view) = driver.await.unwrap();
        assert_eq!(first.unwrap(), BatchOutcome::Applied { added: 2 });
        // 第二次回拉 from_seq=3，节点仍回 1..=2：全部视为重复，视图不变。
        assert_eq!(
            second.unwrap(),
            BatchOutcome::Overlapped {
                added: 0,
                skipped: 2
            }
        );
        assert_eq!(view.cursor(), 2);
        assert_eq!(view.entries().len(), 2);
    }

    #[tokio::test]
    async fn reconcile_records_an_epoch_break() {
        let (_d, _s, conn, mut ws) = connected().await;
        let mut view = RemoteSession::new("s-1");
        // 先用**带纪元**的批把视图的纪元钉在 1（真实链路里批都带纪元）。
        view.note_event(&SessionEvent::TurnBatch {
            session_id: "s-1".into(),
            epoch: 1,
            from_seq: 1,
            entries: vec![entry(1, "old")],
            coalesced_overflow: false,
        });
        assert_eq!(view.epoch(), 1);

        let c = std::sync::Arc::clone(&conn);
        let driver = tokio::spawn(async move {
            let out = view.reconcile(&c).await;
            (out, view)
        });

        match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => match serde_json::from_str::<Frame>(&t).unwrap() {
                Frame::Request { id, .. } => {
                    let ack = Frame::Response {
                        id,
                        result: SessionResult::Log {
                            epoch: 7, // 节点日志被重置：新纪元
                            entries: vec![entry(1, "new")],
                            last_seq: 1,
                        },
                    };
                    ws.send(ClientMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                        .await
                        .unwrap();
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }

        let (_out, view) = driver.await.unwrap();
        assert_eq!(view.epoch(), 7);
        assert_eq!(view.segments().len(), 2, "断裂即开新段");
    }

    #[tokio::test]
    async fn reconcile_reports_a_rejection_as_a_transport_error() {
        let (_d, _s, conn, mut ws) = connected().await;
        let mut view = RemoteSession::new("ghost");

        let c = std::sync::Arc::clone(&conn);
        let driver = tokio::spawn(async move { view.reconcile(&c).await });

        match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => match serde_json::from_str::<Frame>(&t).unwrap() {
                Frame::Request { id, .. } => {
                    let ack = Frame::Response {
                        id,
                        result: SessionResult::Rejected {
                            code: sebas_node_link::SessionRejectCode::UnknownSession,
                            cause: "会话 ghost 不存在".into(),
                        },
                    };
                    ws.send(ClientMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                        .await
                        .unwrap();
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }

        match driver.await.unwrap() {
            Err(NodeLinkError::Transport { cause }) => {
                assert!(cause.contains("unknown_session"), "{cause}");
            }
            other => panic!("应如实报错，实际 {other:?}"),
        }
    }

    // ── 悬空审批（6.4 / 5.5）────────────────────────────────────────────────

    fn approval(request_id: &str, mode: sebas_node_link::SessionMode) -> ParkedApproval {
        ParkedApproval {
            session_id: "s-1".into(),
            request_id: request_id.into(),
            tool: "bash".into(),
            category: sebas_node_link::GateCategory::Execute,
            mode,
        }
    }

    #[test]
    fn approval_events_track_the_parked_set_without_duplicates() {
        let mut view = RemoteSession::new("s-1");
        let event = SessionEvent::ApprovalRequested {
            session_id: "s-1".into(),
            request_id: "s-1:req-1".into(),
            tool: "bash".into(),
            category: sebas_node_link::GateCategory::Execute,
            mode: sebas_node_link::SessionMode::Ask,
        };
        assert!(view.note_event(&event));
        assert!(view.note_event(&event), "同一 id 重放不应产生两条");
        assert_eq!(view.parked_approvals().len(), 1);

        assert!(view.note_event(&SessionEvent::ApprovalRequested {
            session_id: "s-1".into(),
            request_id: "s-1:req-2".into(),
            tool: "edit".into(),
            category: sebas_node_link::GateCategory::Edit,
            mode: sebas_node_link::SessionMode::Ask,
        }));
        assert_eq!(view.parked_approvals().len(), 2);
        assert_eq!(view.parked_approvals()[0].request_id, "s-1:req-1", "按 id 有序");

        assert!(view.note_event(&SessionEvent::GateResolved {
            session_id: "s-1".into(),
            request_id: "s-1:req-1".into(),
            decision: "allow_once".into(),
            source: "control-plane".into(),
        }));
        assert_eq!(view.parked_approvals().len(), 1);
        assert_eq!(view.parked_approvals()[0].request_id, "s-1:req-2");
    }

    #[tokio::test]
    async fn reconcile_approvals_replaces_the_local_set_with_the_nodes_view() {
        let (_d, _s, conn, mut ws) = connected().await;
        let mut view = RemoteSession::new("s-1");
        // 本地先"以为"悬着两条（模拟主控停机前的陈旧视图）。
        view.note_approval_requested(approval("s-1:req-1", sebas_node_link::SessionMode::Ask));
        view.note_approval_requested(approval("s-1:req-2", sebas_node_link::SessionMode::Ask));

        let c = std::sync::Arc::clone(&conn);
        let driver = tokio::spawn(async move {
            let first = view.reconcile_approvals(&c).await;
            let second = view.reconcile_approvals(&c).await;
            (first, second, view)
        });

        // 第一次：节点说只有 req-2 还悬着（req-1 已被决议）→ 以节点为准。
        match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => match serde_json::from_str::<Frame>(&t).unwrap() {
                Frame::Request { id, op } => {
                    assert!(matches!(op, SessionOp::ParkedApprovals));
                    let ack = Frame::Response {
                        id,
                        result: SessionResult::ParkedApprovals {
                            approvals: vec![approval(
                                "s-1:req-2",
                                sebas_node_link::SessionMode::Ask,
                            )],
                        },
                    };
                    ws.send(ClientMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                        .await
                        .unwrap();
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
        // 第二次：节点说一条都不悬了。
        match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => match serde_json::from_str::<Frame>(&t).unwrap() {
                Frame::Request { id, .. } => {
                    let ack = Frame::Response {
                        id,
                        result: SessionResult::ParkedApprovals { approvals: vec![] },
                    };
                    ws.send(ClientMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                        .await
                        .unwrap();
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }

        let (first, second, view) = driver.await.unwrap();
        assert_eq!(first.unwrap(), 1, "以节点为准：只剩 req-2");
        assert_eq!(second.unwrap(), 0, "已决议者不重现");
        assert!(view.parked_approvals().is_empty());
    }

    #[tokio::test]
    async fn answering_a_parked_request_routes_the_decision_and_records_discards() {
        let (_d, _s, conn, mut ws) = connected().await;
        let mut view = RemoteSession::new("s-1");
        view.note_approval_requested(approval("s-1:req-1", sebas_node_link::SessionMode::Ask));
        view.note_approval_requested(approval("s-1:req-2", sebas_node_link::SessionMode::Ask));

        let c = std::sync::Arc::clone(&conn);
        let driver = tokio::spawn(async move {
            let first = view
                .answer_approval(&c, "s-1:req-1", ApprovalDecision::AllowOnce)
                .await;
            let second = view
                .answer_approval(&c, "s-1:req-2", ApprovalDecision::Deny)
                .await;
            (first, second, view)
        });

        // req-1：生效。
        match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => match serde_json::from_str::<Frame>(&t).unwrap() {
                Frame::Request { id, op } => {
                    match op {
                        SessionOp::ApprovalAnswer {
                            session_id,
                            request_id,
                            decision,
                        } => {
                            assert_eq!(session_id, "s-1");
                            assert_eq!(request_id, "s-1:req-1");
                            assert_eq!(decision, ApprovalDecision::AllowOnce);
                        }
                        other => panic!("{other:?}"),
                    }
                    let ack = Frame::Response {
                        id,
                        result: SessionResult::ApprovalApplied { applied: true },
                    };
                    ws.send(ClientMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                        .await
                        .unwrap();
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
        // req-2：节点说会话已终止 → 决定被丢弃。
        match ws.next().await {
            Some(Ok(ClientMessage::Text(t))) => match serde_json::from_str::<Frame>(&t).unwrap() {
                Frame::Request { id, .. } => {
                    let ack = Frame::Response {
                        id,
                        result: SessionResult::ApprovalApplied { applied: false },
                    };
                    ws.send(ClientMessage::Text(serde_json::to_string(&ack).unwrap().into()))
                        .await
                        .unwrap();
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }

        let (first, second, view) = driver.await.unwrap();
        assert!(first.unwrap(), "req-1 已生效");
        assert!(!second.unwrap(), "req-2 被节点丢弃");
        assert!(view.parked_approvals().is_empty(), "两者都离开悬空集合");
        assert_eq!(view.discarded_decisions().len(), 1, "丢弃必须留痕");
        assert!(view.discarded_decisions()[0].contains("req-2"));
    }

    #[tokio::test]
    async fn answering_an_unknown_request_fails_locally_before_touching_the_link() {
        let (_d, _s, conn, _ws) = connected().await;
        let mut view = RemoteSession::new("s-1");
        match view
            .answer_approval(&conn, "s-1:req-404", ApprovalDecision::AllowOnce)
            .await
        {
            Err(NodeLinkError::Transport { cause }) => {
                assert!(cause.contains("req-404"), "{cause}");
            }
            other => panic!("未知请求应本地如实失败，实际 {other:?}"),
        }
    }
}
