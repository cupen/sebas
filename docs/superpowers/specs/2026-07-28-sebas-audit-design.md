# sebas 走查（audit）设计

> 日期：2026-07-28
> 状态：已确认
> 作者：Claude（与 cupen 协作）

## 1. 背景与目标

sebas 骨架已完整（feishu / acp-claude / router 三 crate，全部测试通过），但 README 与 spec 中存在多条"未验证 / 待定"声明，且 git log 显示部分声明已过时（真实使用中修过的 bug 与"WS 未端到端验证"矛盾）。

本次走查的目标：把项目从"声明状态"推进到"核实状态"——

- README / spec 里每一条"未验证""待定"都有明确结论
- 发现的每个真实问题都进 beads，带优先级和依赖关系
- 形成一份有依据的 backlog，后续修复按优先级排期

## 2. 原则

1. **走查不修代码 bug**：发现的问题只记录为 beads；唯一顺手修改的是文档（README 声明刷新、spec §7 标注结论），这属于走查产出本身。
2. **静态走查为主**：本轮只做代码 / 文档 / 测试的静态核查，不需要真实飞书环境。
3. **动态 smoke test 延期**：真实飞书冒烟测试（README 6 步 + slash 全表 + 群聊 @ + 媒体消息）不在本轮范围，作为 follow-up beads 记录，待静态问题修完后用户再跑。

## 3. 走查范围（静态）

### A. 文档声明核实

- README `Status` + `Known limitations` 共 6 条，逐条对照代码与 git log 核实
- spec §7「待定 / 后续」3 条核实（群聊 @ 消息格式、ACP 子命令协议、record 子命令）

### B. 代码实现 vs spec 差距

- spec §4.1 错误处理矩阵 9 行逐行核对：Feishu 重试 3 次、child hang 5min 检测、idle_kill_secs、max_concurrent_sessions 上限等是否实现
- spec §5 的 11 个 slash 命令（`/new` `/sessions` `/switch` `/resume` `/cancel` `/status` `/compact` `/cost` `/model` `/cd` `/help`）逐一核实实现状态
- spec §3.2 emoji 状态机（👀→🚧→✅）、§3.3(e) 重启恢复、§3.3(f) 媒体消息流是否落地
- spec §6.2 配置表 vs `src/config.rs` 实际字段（spec 有但未实现的配置项）

### C. 测试盘点

- 现有测试 vs 覆盖率目标（router/cards ≥90%，整体 ≥80%）的差距
- CI 缺失、cargo-llvm-cov 未配置、真实 ACP fixture harness 缺失——确认并记录

## 4. 产出

1. **beads issues**：所有确认的问题，带 P0–P4 优先级与依赖关系。优先级口径：
   - P0：daemon 不可用 / 数据丢失
   - P1：核心链路不工作（消息收发、权限审批、session 管理）
   - P2：功能缺失或体验受损但有 workaround
   - P3：文档 / 测试 / 工具缺失
   - P4：改进项
2. **README 刷新**：声明更新为核实后的真实状态
3. **spec §7 标注**：待定项更新结论
4. **走查总结**：会话内汇报发现清单与建议修复顺序，不单独写报告文档

## 5. 执行方式

- 静态走查按维度（A / B / C）派并行 subagent 核查，汇总后去重、定优先级、批量建 beads
- 建 beads 前先查重（现有 9 条已关闭 issue，避免重复立项）
- 文档刷新（README / spec §7）在 beads 建完后一次性提交修改，遵循 conservative profile：只改文件，不 commit，最后报告建议命令

## 6. 非目标

- 不修复任何代码 bug
- 不跑真实飞书 smoke test（见 §2 原则 3）
- 不做新功能设计（多 agent backend 等留待 backlog 排期时再议）
