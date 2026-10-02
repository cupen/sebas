//! 会话面 checkpoint（fix-webui-qa-round9 1.2/1.3，session-transcript-durability）。
//!
//! 引擎是唯一同时看得到转录（`turn_log`）、泊车审批（`stall` 的 parked
//! 登记）与会话级 usage（`card_states.usage_total`）的位置；本模块把三面
//! 状态序列化成**单行快照**（projects.db 的 `session_checkpoints` 表，经
//! 状态库单写 actor 提交——persistence-runtime 准入），并在 core 启动时
//! 反向回填。
//!
//! - **触发**：审批泊车进入/解除事件驱动即时 checkpoint（审批丢失代价高、
//!   频度低）+ 周期兜底扫描（`[dispatch] checkpoint_interval_secs`，默认
//!   30s，`0` = 关，装配在 run.rs）。事件触发点经
//!   [`DispatchHandle::checkpoint_spawn`] 起后台任务——写与回合事件流异步，
//!   绝不阻塞转发（spec「checkpoint 不阻塞回合」）。
//! - **未变跳过**：快照内容指纹与上次成功写入一致的会话不重写（spec
//!   「内容未变时不重写」）；指纹只在写入成功后登记——写失败下轮自然重试。
//! - **回放**：[`DispatchHandle::restore_session_checkpoints`] 在
//!   session_map 载入后调用；注册表里有而 checkpoint 缺失的会话保持空转录
//!   （现状），checkpoint 有而注册表没有的孤儿行删除（注册表与回放内容
//!   一致）。
//! - **close**：[`DispatchHandle::checkpoint_drop`] 随会话终结删行 + 忘
//!   指纹——归档语义不受影响（close 归档的快照落盘在 webui 侧照旧，回放
//!   不复活已归档会话）。

use super::{now_unix, DispatchHandle};
use crate::engine::stall::ParkedRequest;
use sebas_channels::card::AppUsage;
use sebas_domain::session::TurnEntry;
use sebas_models::checkpoint::SessionCheckpointRow;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 一个会话的 checkpoint 快照（内存态 ↔ 行的中间形状；serde 双向）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct SessionCheckpointSnapshot {
    /// 转录条目（position 保序；回放时重排 0..n）。
    #[serde(default)]
    pub transcript: Vec<TurnEntry>,
    /// 泊车审批登记（空泊车 = 空表）。
    #[serde(default)]
    pub parked: Vec<ParkedRequest>,
    /// 会话累计输入 token。
    #[serde(default)]
    pub usage_in: u64,
    /// 会话累计输出 token。
    #[serde(default)]
    pub usage_out: u64,
    /// 会话是否上报过任何 token 计数（区分「未上报」与「已上报 0」）。
    #[serde(default)]
    pub usage_reported: bool,
}

impl SessionCheckpointSnapshot {
    /// 折成状态库行（JSON 序列化失败按空表降级——快照不因单面序列化失败
    /// 整体丢弃，与「截断诚实」同口径）。
    fn to_row(&self, session_id: &str) -> SessionCheckpointRow {
        SessionCheckpointRow {
            session_id: session_id.to_string(),
            updated_at: now_unix(),
            transcript_json: serde_json::to_string(&self.transcript)
                .unwrap_or_else(|_| "[]".into()),
            parked_json: serde_json::to_string(&self.parked).unwrap_or_else(|_| "[]".into()),
            usage_in: self.usage_in as i64,
            usage_out: self.usage_out as i64,
            usage_reported: self.usage_reported,
        }
    }

    fn from_row(row: SessionCheckpointRow) -> Result<Self, String> {
        let transcript = if row.transcript_json.is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&row.transcript_json)
                .map_err(|e| format!("checkpoint {} 转录反序列化失败: {e}", row.session_id))?
        };
        let parked = if row.parked_json.is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&row.parked_json)
                .map_err(|e| format!("checkpoint {} 泊车反序列化失败: {e}", row.session_id))?
        };
        Ok(Self {
            transcript,
            parked,
            usage_in: row.usage_in.max(0) as u64,
            usage_out: row.usage_out.max(0) as u64,
            usage_reported: row.usage_reported,
        })
    }

    /// 内容指纹（序列化后的转录/泊车 JSON + usage 标量的哈希）：未变跳过
    /// 写入的依据。指纹只做内存内比对，碰撞的后果是「少写一次同内容快照」
    /// ——幂等 upsert 兜底，无正确性影响。
    fn fingerprint(&self, transcript_json: &str, parked_json: &str) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        transcript_json.hash(&mut h);
        parked_json.hash(&mut h);
        self.usage_in.hash(&mut h);
        self.usage_out.hash(&mut h);
        self.usage_reported.hash(&mut h);
        h.finish()
    }
}

/// 单次 checkpoint 的结果（测试与日志用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointOutcome {
    /// 快照已写入状态库（内容有变化）。
    Written,
    /// 内容与上次成功写入一致，跳过（spec「不重复写」）。
    Unchanged,
    /// 无可快照的会话（无映射 / Spawning 占位），或转录与泊车皆空且从未
    /// 上报 usage——无可恢复内容，不落空壳行。
    NothingToPersist,
    /// 状态库引擎未接（独立 router / 测试夹具）——如实静默（现状语义：
    /// 无状态库的进程本来就没有会话面持久化）。
    EngineUnavailable,
    /// 写入被状态库拒绝（下轮扫描/事件重试）。
    Failed,
}

/// 快照内容指纹登记表（session_id → 上次成功写入的指纹）。纯内存：重启后
/// 登记为空，首轮周期扫描全量重写一遍（幂等 upsert，无正确性影响）。
#[derive(Default, Clone)]
pub(crate) struct CheckpointRegistry {
    fingerprints: Arc<RwLock<HashMap<String, u64>>>,
}

impl CheckpointRegistry {
    async fn get(&self, session_id: &str) -> Option<u64> {
        self.fingerprints.read().await.get(session_id).copied()
    }

    async fn record(&self, session_id: &str, fp: u64) {
        self.fingerprints
            .write()
            .await
            .insert(session_id.to_string(), fp);
    }

    async fn forget(&self, session_id: &str) {
        self.fingerprints.write().await.remove(session_id);
    }
}

impl DispatchHandle {
    /// 序列化一个会话的三面状态（转录 + 泊车 + usage）。无可寻址转录的会话
    /// （无映射 / Spawning 占位）返回 `None`。usage 从卡态投影（`usage_total`
    /// 是会话累计；`usage_reported` 是「未上报」门控位）——无卡态 = 从未开过
    /// 回合 = 未上报。
    pub async fn session_checkpoint_snapshot(
        &self,
        session_id: &str,
    ) -> Option<SessionCheckpointSnapshot> {
        let transcript = self.turn_log.read().await.get(session_id).cloned();
        let parked = self.stall.parked_requests(session_id).await;
        let (usage_in, usage_out, usage_reported) = match self.card_states.snapshot(session_id).await
        {
            Some(st) => (
                st.usage_total.total_input,
                st.usage_total.total_output,
                st.usage_reported,
            ),
            None => (0, 0, false),
        };
        Some(SessionCheckpointSnapshot {
            transcript: transcript.unwrap_or_default(),
            parked,
            usage_in,
            usage_out,
            usage_reported,
        })
    }

    /// 立即 checkpoint 一个会话（事件驱动入口的执行半边；生产触发经
    /// [`Self::checkpoint_spawn`] 起后台任务，本方法自身可 await——单测与
    /// 周期扫描用）。内容未变 → [`CheckpointOutcome::Unchanged`]。
    pub async fn checkpoint_session_now(&self, session_id: &str) -> CheckpointOutcome {
        let Some(engine) = crate::state_store::engine() else {
            return CheckpointOutcome::EngineUnavailable;
        };
        let Some(snapshot) = self.session_checkpoint_snapshot(session_id).await else {
            return CheckpointOutcome::NothingToPersist;
        };
        // 空壳不落行：无转录、无泊车、从未上报 usage 的会话（0-turn 占位 /
        // 刚 spawn 的空会话）没有可恢复内容。
        if snapshot.transcript.is_empty() && snapshot.parked.is_empty() && !snapshot.usage_reported
        {
            return CheckpointOutcome::NothingToPersist;
        }
        let row = snapshot.to_row(session_id);
        let fp = snapshot.fingerprint(&row.transcript_json, &row.parked_json);
        if self.checkpoints.get(session_id).await == Some(fp) {
            return CheckpointOutcome::Unchanged;
        }
        match engine.save_session_checkpoint(row).await {
            Ok(()) => {
                self.checkpoints.record(session_id, fp).await;
                CheckpointOutcome::Written
            }
            Err(e) => {
                tracing::warn!(session_id = %session_id, error = %e, "session checkpoint 写入失败（下轮重试）");
                CheckpointOutcome::Failed
            }
        }
    }

    /// 事件驱动的即时 checkpoint 触发（fire-and-forget）：起后台任务在
    /// **执行时刻**重读当前状态并写入——任务排队间隙的后续变更自然并入，
    /// 不存在陈旧快照覆盖新态的窗口（同一单写 actor 串行化）。调用方
    /// （审批泊车进入/解除的漏斗臂）零阻塞。`DispatchHandle` 的克隆是廉价
    /// 的（字段全部 Arc 背书），任务与原句柄共享同一份状态。
    pub(crate) fn checkpoint_spawn(&self, session_id: String) {
        let handle = self.clone();
        tokio::spawn(async move {
            let outcome = handle.checkpoint_session_now(&session_id).await;
            if outcome == CheckpointOutcome::Written {
                tracing::debug!(session_id = %session_id, "session checkpoint 已即时落盘");
            }
        });
    }

    /// 周期兜底扫描：对全部有转录寻址 id 的映射逐会话 checkpoint（内容
    /// 未变的在 [`Self::checkpoint_session_now`] 内跳过）。返回实际写入数。
    pub async fn checkpoint_dirty_sessions(&self) -> usize {
        // 先收集 transcript 寻址 id，再逐个写——不持映射锁跨写路径。
        let ids: Vec<String> = self
            .map
            .snapshot_all()
            .await
            .into_iter()
            .filter_map(|(_, m)| m.transcript_id().map(str::to_string))
            .collect();
        let mut written = 0;
        for sid in ids {
            if self.checkpoint_session_now(&sid).await == CheckpointOutcome::Written {
                written += 1;
            }
        }
        written
    }

    /// 会话终结路径的 checkpoint 半边（close 调用）：忘指纹 + 删行。幂等
    /// （无行 = no-op）。归档快照落盘语义不受影响——webui 侧的归档条目在
    /// close 之前落 archive，这里只负责 checkpoint 不复活、不重复计数。
    pub async fn checkpoint_drop(&self, session_id: &str) {
        self.checkpoints.forget(session_id).await;
        if let Some(engine) = crate::state_store::engine()
            && let Err(e) = engine.delete_session_checkpoint(session_id).await
        {
            tracing::warn!(session_id = %session_id, error = %e, "session checkpoint 删除失败");
        }
    }

    /// 启动回放（fix-webui-qa-round9 1.3）：session_map 载入后把 checkpoint
    /// 行回填进引擎三面状态。逐行：
    ///
    /// - 注册表里**没有**该 session_id 的映射 → 孤儿行，删除（close 删行是
    ///   常态路径，孤儿只在「映射删除与 checkpoint 删除之间崩溃」时出现）；
    /// - 转录：按行重排 position（0..n 单调）写入 turn_log（与归档恢复同
    ///   一口径；启动期直接插入、不发流式广播——回放不是回合事件）；
    /// - 泊车：逐条登记回 stall 泊车表——待批审批经既有读模型/restore 链路
    ///   恢复呈现并可决定；
    /// - usage：已上报的会话回填累计 token（`usage_reported` 门控——未上报
    ///   的保持「未上报」，不以 0 冒充）。
    ///
    /// 返回 `(回放的会话数, 删除的孤儿行数)`。反序列化失败的行跳过并告警
    /// （截断诚实：坏行按丢失处理，不阻塞其余回放）。
    pub async fn restore_session_checkpoints(
        &self,
        rows: Vec<SessionCheckpointRow>,
    ) -> (usize, usize) {
        let mut restored = 0;
        let mut pruned = 0;
        for row in rows {
            let sid = row.session_id.clone();
            // 无映射 → 孤儿行：注册表说了算（按 transcript 寻址 id 匹配——
            // 启动恢复的会话是 Dormant，`session_id()` 只认 Active）。
            if self.map.lookup_key_by_transcript(&sid).await.is_none() {
                self.checkpoint_drop(&sid).await;
                pruned += 1;
                continue;
            }
            let snapshot = match SessionCheckpointSnapshot::from_row(row) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "checkpoint 行反序列化失败，按丢失处理");
                    continue;
                }
            };
            // 转录回放：position 重排 0..n。
            if !snapshot.transcript.is_empty() {
                let mut g = self.turn_log.write().await;
                let log = g.entry(sid.clone()).or_default();
                log.reserve(snapshot.transcript.len());
                for (i, mut e) in snapshot.transcript.into_iter().enumerate() {
                    e.position = i as u64;
                    log.push(e);
                }
            }
            // 泊车回放：逐条登记（同 id 幂等覆盖）。
            for pr in snapshot.parked {
                self.stall
                    .note_permission_parked(&sid, &pr.request_id, &pr.tool_name, pr.args)
                    .await;
            }
            // usage 回放：门控位在场才回填（未上报保持未上报，不以 0 冒充）。
            if snapshot.usage_reported {
                self.card_states
                    .apply(&sid, |st| {
                        st.usage_total = AppUsage {
                            model: None,
                            total_input: snapshot.usage_in,
                            total_output: snapshot.usage_out,
                        };
                        st.usage_reported = true;
                    })
                    .await;
            }
            restored += 1;
        }
        (restored, pruned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::SessionMap;
    use crate::test_engine::install_fresh;
    use sebas_channels::ChannelKey;

    /// 夹具：全新内存引擎 + 带一个 Dormant 映射（session_id = "sess-ckpt"）
    /// 的句柄。库内行观察走全局槽的 `load_session_checkpoints`（引擎经
    /// [`install_fresh`] 装入槽，trait 对象读取即同一份状态）。
    async fn fixture() -> (std::sync::Arc<DispatchHandle>, crate::test_engine::EngineGuard) {
        let guard = install_fresh();
        let (h, _rx) = DispatchHandle::new(SessionMap::default());
        let key = ChannelKey::web_new();
        h.map
            .insert(key, crate::state::Mapping::dormant("sess-ckpt", 42))
            .await
            .unwrap();
        (std::sync::Arc::new(h), guard)
    }

    async fn stored_rows() -> Vec<sebas_models::checkpoint::SessionCheckpointRow> {
        crate::state_store::engine()
            .expect("测试夹具已装引擎")
            .load_session_checkpoints()
            .await
            .unwrap()
    }

    async fn seed_transcript(h: &DispatchHandle) {
        h.push_transcript_entry(
            "sess-ckpt",
            TurnEntry::prompt(0, "帮我看看这个 bug"),
        )
        .await;
        h.push_transcript_entry(
            "sess-ckpt",
            TurnEntry::markdown(1, "看过了，是越界。"),
        )
        .await;
    }

    /// 1.2 主张 1：写出行内容与内存态一致——转录条目、泊车登记、usage 全部
    /// 进同一行快照。
    #[tokio::test]
    async fn checkpoint_row_matches_in_memory_state() {
        let (_h_arc, _guard) = fixture().await;
        let h = _h_arc.clone();
        seed_transcript(&h).await;
        h.stall
            .note_permission_parked("sess-ckpt", "claude:tc-1", "Bash", serde_json::json!({"command": "ls"}))
            .await;
        h.card_states
            .apply("sess-ckpt", |st| {
                st.usage_total = AppUsage { model: None, total_input: 120, total_output: 45 };
                st.usage_reported = true;
            })
            .await;

        let outcome = h.checkpoint_session_now("sess-ckpt").await;
        assert_eq!(outcome, CheckpointOutcome::Written);

        let rows = stored_rows().await;
        assert_eq!(rows.len(), 1, "一会话一行");
        let snap = SessionCheckpointSnapshot::from_row(rows[0].clone()).unwrap();
        assert_eq!(snap.transcript.len(), 2);
        assert_eq!(snap.transcript[0].content, "帮我看看这个 bug");
        assert_eq!(snap.parked.len(), 1);
        assert_eq!(snap.parked[0].request_id, "claude:tc-1");
        assert_eq!(snap.usage_in, 120);
        assert_eq!(snap.usage_out, 45);
        assert!(snap.usage_reported);
    }

    /// （fix-webui-qa-round9 review 缺口 B）开轮 reseed 保留会话累计 usage：
    /// `emit_turn_card` 的 drop+seed 复位的是「本回合」缓冲（body/usage），
    /// `usage_total`/`usage_reported` 是会话累计（checkpoint 的持久面），
    /// 必须随新卡态带回——否则第二轮开轮起，已上报历史从持久面消失。
    #[tokio::test]
    async fn turn_reseed_preserves_session_usage_totals() {
        let guard = install_fresh();
        let (h, _rx) = DispatchHandle::new(SessionMap::default());
        let key = ChannelKey::web_new();
        h.map
            .insert(key.clone(), crate::state::Mapping::dormant("sess-ckpt", 42))
            .await
            .unwrap();
        let _ = guard;
        h.card_states
            .apply("sess-ckpt", |st| {
                st.usage_total = AppUsage { model: None, total_input: 120, total_output: 45 };
                st.usage_reported = true;
            })
            .await;

        h.emit_turn_card(key, "sess-ckpt", "第二轮".into(), None).await;

        let st = h.card_states.snapshot("sess-ckpt").await.expect("开轮后卡态在");
        assert_eq!(st.usage_total.total_input, 120, "会话累计跨轮保留");
        assert_eq!(st.usage_total.total_output, 45);
        assert!(st.usage_reported, "上报门控位跨轮保留");
        // 本回合缓冲确实复位（新卡 SEED、空 body），保留的只是累计。
        assert_eq!(st.status_emoji, crate::card_state::phase::SEED);
        assert!(st.body.is_empty());
        assert_eq!(st.usage.total_input, 0);
    }

    /// 1.2 主张 2：内容未变不重写（spec「不重复写」）；内容变化后再次写入。
    #[tokio::test]
    async fn unchanged_content_skips_the_rewrite() {
        let (_h_arc, _guard) = fixture().await;
        let h = _h_arc.clone();
        seed_transcript(&h).await;

        assert_eq!(h.checkpoint_session_now("sess-ckpt").await, CheckpointOutcome::Written);
        let first = stored_rows().await;
        // 同内容再扫：Unchanged，行不动。
        assert_eq!(h.checkpoint_session_now("sess-ckpt").await, CheckpointOutcome::Unchanged);
        assert_eq!(stored_rows().await.len(), 1);
        assert_eq!(stored_rows().await[0].transcript_json, first[0].transcript_json);

        // 追加一条转录 → 内容变 → 再写。
        h.push_transcript_entry("sess-ckpt", TurnEntry::markdown(2, "补充一点。")).await;
        assert_eq!(h.checkpoint_session_now("sess-ckpt").await, CheckpointOutcome::Written);
        let rows = stored_rows().await;
        assert_eq!(rows.len(), 1, "仍是一会话一行");
        let snap = SessionCheckpointSnapshot::from_row(rows[0].clone()).unwrap();
        assert_eq!(snap.transcript.len(), 3);
    }

    /// 1.2 主张 3：无转录/无泊车/未上报 usage 的空壳会话不落行（0-turn 占位
    /// 不制造孤儿）；无映射的 session_id 无可快照。
    #[tokio::test]
    async fn empty_sessions_do_not_persist_placeholder_rows() {
        let (_h_arc, _guard) = fixture().await;
        let h = _h_arc.clone();
        // sess-ckpt 存在但什么都没有。
        assert_eq!(
            h.checkpoint_session_now("sess-ckpt").await,
            CheckpointOutcome::NothingToPersist
        );
        assert!(stored_rows().await.is_empty());
        // 无映射的 id：快照 None。
        assert_eq!(
            h.checkpoint_session_now("no-such-session").await,
            CheckpointOutcome::NothingToPersist
        );
    }

    /// 1.2 主张 4：周期扫描只写脏会话——两个会话其一变更时返回写入数 1。
    #[tokio::test]
    async fn periodic_scan_writes_only_dirty_sessions() {
        let guard = install_fresh();
        let _ = &guard;
        let (h, _rx) = DispatchHandle::new(SessionMap::default());
        let h = std::sync::Arc::new(h);
        for sid in ["sess-a", "sess-b"] {
            h.map
                .insert(ChannelKey::web_new(), crate::state::Mapping::dormant(sid, 1))
                .await
                .unwrap();
            h.push_transcript_entry(sid, TurnEntry::prompt(0, "hi")).await;
        }
        // 首轮：两个都脏 → 写 2。
        assert_eq!(h.checkpoint_dirty_sessions().await, 2);
        // 第二轮：全净 → 写 0。
        assert_eq!(h.checkpoint_dirty_sessions().await, 0);
        // 只动 sess-b → 写 1。
        h.push_transcript_entry("sess-b", TurnEntry::markdown(1, "more")).await;
        assert_eq!(h.checkpoint_dirty_sessions().await, 1);
    }

    /// 1.3 主张 1：回放后 turns() 输出与 checkpoint 前一致（position 重排
    /// 单调）；泊车登记与 usage 一并回填（已上报门控）。
    #[tokio::test]
    async fn replay_restores_turns_parked_and_usage() {
        let (_h_arc, _guard) = fixture().await;
        let h = _h_arc.clone();

        let rows = vec![SessionCheckpointRow {
            session_id: "sess-ckpt".into(),
            updated_at: 42,
            transcript_json: serde_json::to_string(&vec![
                TurnEntry::prompt(7, "第一条"),
                TurnEntry::markdown(9, "回复正文"),
            ])
            .unwrap(),
            parked_json: serde_json::to_string(&vec![ParkedRequest {
                request_id: "claude:tc-9".into(),
                tool_name: "Write".into(),
                args: serde_json::json!({"file_path": "/x"}),
            }])
            .unwrap(),
            usage_in: 33,
            usage_out: 7,
            usage_reported: true,
        }];

        let (restored, pruned) = h.restore_session_checkpoints(rows).await;
        assert_eq!((restored, pruned), (1, 0));

        // 转录经公开读模型回读：position 重排 0..n、内容原样。
        let key = h.map.lookup_key_by_transcript("sess-ckpt").await.unwrap();
        let turns = h.session_turns(&key, 0).await.unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].position, 0);
        assert_eq!(turns[0].content, "第一条");
        assert_eq!(turns[1].position, 1);
        assert_eq!(turns[1].content, "回复正文");

        // 泊车登记回填：读模型可枚举、fail-closed 校验可批。
        let parked = h.stall.parked_requests("sess-ckpt").await;
        assert_eq!(parked.len(), 1);
        assert_eq!(parked[0].request_id, "claude:tc-9");
        assert_eq!(h.stall.parked_owner("claude:tc-9").await.as_deref(), Some("sess-ckpt"));

        // usage 回填：已上报会话的累计可见（快照投影非「未上报」）。
        let st = h.card_states.snapshot("sess-ckpt").await.unwrap();
        assert!(st.usage_reported);
        assert_eq!(st.usage_total.total_input, 33);
        assert_eq!(st.usage_total.total_output, 7);
    }

    /// 1.3 主张 2：usage 未上报的 checkpoint 不冒充「已上报 0」——回放后
    /// 投影仍是未上报。
    #[tokio::test]
    async fn replay_keeps_unreported_usage_unreported() {
        let (_h_arc, _guard) = fixture().await;
        let h = _h_arc.clone();
        let rows = vec![SessionCheckpointRow {
            session_id: "sess-ckpt".into(),
            updated_at: 42,
            transcript_json: "[]".into(),
            parked_json: "[]".into(),
            usage_in: 0,
            usage_out: 0,
            usage_reported: false,
        }];
        h.restore_session_checkpoints(rows).await;
        // 无卡态 → 投影未上报（apply 未被触发）。
        assert!(h.card_states.snapshot("sess-ckpt").await.is_none());
    }

    /// 1.3 主张 3：注册表里没有的会话 → 孤儿行删除（返回 pruned=1，库里
    /// 也不再有其行）；注册表里有而 checkpoint 缺失 → 保持现状（不报错、
    /// 不伪造）。
    #[tokio::test]
    async fn replay_prunes_orphan_rows_and_tolerates_missing_ones() {
        let (_h_arc, _guard) = fixture().await;
        let h = _h_arc.clone();
        let rows = vec![
            SessionCheckpointRow {
                session_id: "sess-ckpt".into(), // 注册表里有
                updated_at: 1,
                transcript_json: serde_json::to_string(&vec![TurnEntry::prompt(0, "x")]).unwrap(),
                parked_json: "[]".into(),
                usage_in: 0,
                usage_out: 0,
                usage_reported: false,
            },
            SessionCheckpointRow {
                session_id: "sess-orphan".into(), // 注册表里没有
                updated_at: 1,
                transcript_json: serde_json::to_string(&vec![TurnEntry::prompt(0, "y")]).unwrap(),
                parked_json: "[]".into(),
                usage_in: 0,
                usage_out: 0,
                usage_reported: false,
            },
        ];
        let (restored, pruned) = h.restore_session_checkpoints(rows).await;
        assert_eq!((restored, pruned), (1, 1));
        // 孤儿行已删。
        let left = stored_rows().await;
        assert!(left.iter().all(|r| r.session_id != "sess-orphan"));
    }

    /// 1.3 主张 4：close 归档删除对应 checkpoint 行（checkpoint_drop）——
    /// 归档会话重启后不复活（回放不重新挂回）。
    #[tokio::test]
    async fn close_deletes_the_checkpoint_row() {
        let (_h_arc, _guard) = fixture().await;
        let h = _h_arc.clone();
        seed_transcript(&h).await;
        assert_eq!(h.checkpoint_session_now("sess-ckpt").await, CheckpointOutcome::Written);
        assert_eq!(stored_rows().await.len(), 1);

        let key = h.map.lookup_key_by_transcript("sess-ckpt").await.unwrap();
        assert_eq!(
            h.web_close_session(key).await,
            crate::engine::CloseOutcome::Closed { discarded_pending: 0 }
        );
        let rows = stored_rows().await;
        assert!(
            rows.iter().all(|r| r.session_id != "sess-ckpt"),
            "close 后 checkpoint 行必须删除: {rows:?}"
        );
    }

    /// 无引擎（独立 router / 裸夹具）：checkpoint 如实报 EngineUnavailable，
    /// 回放把全部行当孤儿删不了也绝不 panic（engine() None 时删除是 no-op）。
    #[tokio::test]
    async fn missing_engine_degrades_to_unavailable_outcome() {
        let _guard = crate::test_engine::install_none();
        let (h, _rx) = DispatchHandle::new(SessionMap::default());
        h.map
            .insert(ChannelKey::web_new(), crate::state::Mapping::dormant("sess-x", 1))
            .await
            .unwrap();
        h.push_transcript_entry("sess-x", TurnEntry::prompt(0, "hi")).await;
        assert_eq!(
            h.checkpoint_session_now("sess-x").await,
            CheckpointOutcome::EngineUnavailable
        );
        // 回放：无引擎 → 删除 no-op，孤儿行计数仍如实。
        let (restored, pruned) = h
            .restore_session_checkpoints(vec![SessionCheckpointRow {
                session_id: "no-mapping".into(),
                updated_at: 0,
                transcript_json: "[]".into(),
                parked_json: "[]".into(),
                usage_in: 0,
                usage_out: 0,
                usage_reported: false,
            }])
            .await;
        assert_eq!((restored, pruned), (0, 1));
    }
}
