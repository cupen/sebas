## Context

第八轮验收缺陷均有源码级根因定位（QA 证据 + 核查报告，`D:/workbench/tmp/sebas-qa/`）。修复横跨 `sebas-webui/frontend`（Lit SPA）、`sebas-webui/src`（session/agent backend）、`sebas-dispatch`、`sebas-agent` 五处，但无 wire 形状变更、无新依赖。关键现状：权限决策在 `sebas-domain` 词汇表里把 `Escalate` 有意降级为 `AllowOnce`（ACP 边界无升级等价物），native 路径经 ApproverHub 直投支持升级；native 内核不上报 prompt 条目与 pending 集合（`agent_backend.rs` 注释自认）；转录 sticky 滚动有「未读缝定位置 false」「距底 80px 翻 false」两处门控；零输出 notice 在 summary 事件时点求值而 error 条目晚于它落地。

## Goals / Non-Goals

**Goals:**
- 审批降级可见、native 转录三面（气泡/待执行栈/未读）补齐、滚动可达、布局遮挡清零、关键操作留痕对齐
- 全部修复可用现有沙箱装配（fake-claude + test/* 场景模型）做浏览器级复测

**Non-Goals:**
- 不改会话 key 编码、不给 claude 驱动造升级协议、不做 admin 动作 UI 恢复（见 proposal Non-goals）

## Decisions

1. **escalate 可见化走「呈现层」而非「协议层」**：保留 `vocabulary.rs` 的降级映射（它是 ACP 边界的现实约束），在 webui 应答路径感知降级事实后落转录系统条目（对齐 `permission_mode_result` 先例），条目携带操作者原因。被否：给 claude driver 实现 escalate——需要 agent 侧协议支持，超本轮范围。
2. **native 用户气泡由内核宿主补条目**：`NativeAgentBackend::message` 在投递内核前向转录 push `TurnEntry::prompt`（与 ACP seed_card 等价），前端零改动。被否：前端从请求记录乐观补气泡——刷新/重放会双份或不一致。
3. **native 待执行栈用 webui 影子队列**：提交即记、`session.updated` 相位帧推进即对账清理；不改内核 wire。被否：内核上报 pending 集合——wire 变更超范围，且排队本质是内核内部 VecDeque，core 不该建模它。
4. **滚动跟随：浮标 + 门控收敛**：新增「跳到最新」浮标（sticky=false 且有新条目时出现）；未读缝定位改为「定位完成即恢复 sticky」，仅操作者主动上滚才脱离跟随。被否：无条件强制贴底——会打断回看历史。
5. **未读锚竞速：锚推进只由「已见」驱动**：定稿覆盖写锚与切会话路径统一为一处「以可见高度/全部已读为前提」的推进逻辑；rail 徽标以 `/api/sessions` 的 `msg_count` 为真值对账，localStorage 锚仅作本地缓存。
6. **零输出判定后移**：调整 `sebas-agent` 事件顺序——终态（含 Error）落地后再发 summary；判据表本身不动（`land()` 已把 error 算可见输出）。
7. **连接徽标让位**：工作台头部右端预留徽标宽度（布局占位），徽标不再绝对定位悬浮；归档视图同策略。
8. **模型切换条目**：`apply_model_changed`（dispatch）与 native override 路径各 push 系统条目，格式对齐权限模式条目。
9. **key 友好化**：新增纯前端工具函数解码 `%00` 编码串为「渠道 · 本地段」标签，仅用于展示位（审批面板、聚焦链接），wire 与路由不变。
10. **别名管理 UI**：设置弹窗新增分区，复用 `/api/model-aliases` 既有 CRUD；不新增 API。

## Risks / Trade-offs

- [内核事件顺序调整可能影响既有收尾语义] → 只后移 summary 相对 error 的发射点，进程级 e2e（`tests/testsuite_e2e_test.rs`）与验收套件全量回归把关
- [影子队列与内核真实队列可能短暂失真] → 以 `session.updated` 相位帧为对账锚点，失真窗口最多一个刷新周期；会话终结时整栈清空并提示
- [设置弹窗已 4300+ 行] → 别名分区独立成模块文件挂入，不继续膨胀单文件
- [滚动门控收敛可能改变回看习惯] → 保留「手动上滚即脱离跟随」的原语义，只修「未上滚也脱离」的误伤面

## Migration Plan

纯增量修复，无数据迁移；灰度面为零（单二进制内嵌前端）。回滚 = 回退本轮提交。

## Open Questions

（无）
