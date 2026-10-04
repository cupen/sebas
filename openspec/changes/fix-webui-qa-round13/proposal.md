# fix-webui-qa-round13 Proposal

## Why

第十三轮 GUI 全功能验收（QA-A/QA-B 双簇真实浏览器黑盒，54 项 PASS、console 零
pageerror，证据在 `verification/REPORT.md` 与 `findings-a.md` / `findings-b.md`）给出
收口清单：两个稳定复现的呈现缺陷（B-2 thinking 面板留白、观察-2 usage 刻度重复）、
一个失实引导文案（native 不可用提示指向设置页，真实启用条件是 core env，配置了也不
点亮）、一项未钉死的口径（usage 页只见 router 流量），以及验收记录侧的一处口径修正
（B-1：allow 与 auto 同为放行档系 round2 M-C6 既有裁决，AGENTS.md 过时句误导了本轮
验收预期，B-1 据此改判非缺陷）。

## What Changes

- thinking 过程展开面板压缩「💭 thinking」标签与内容间的留白（B-2，两轮采样稳定复现）
- usage 折线图 y 轴刻度去重（观察-2：相邻刻度重复呈现 1/1/1/0/0）
- 会话创建对话框对 native 不可用的提示如实化：改述真实启用条件（core 进程 env
  `SEBAS_AGENT_PROVIDER_API_KEY` / `SEBAS_AGENT_ROUTER_URL`），SHALL NOT 再引导到
  「设置 → 模型」（该页配置不能点亮 native，QA 实证）
- usage 页口径钉死：usage 页呈现 router 用量记录（ACP 直连回合不经 router、不产生
  用量记录，其 token 计数呈现于会话头部，不入本视图）；spec 场景化 + 页面加数据源
  说明（观察-1）
- AGENTS.md 过时句修正：「审批路径（ask/edit/allow 档）才有环后正文」→ allow 与
  auto 同为放行档、无审批卡无环后正文（B-1 收口 = 口径修正，无代码变更）

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `usage-statistics`：WebUI usage view 增加坐标刻度唯一性要求与数据源口径（仅 router
  用量记录；纯 ACP 流量窗口如实呈现无数据空态）
- `webui`：会话创建对话框对不可用 agent 的禁用提示 SHALL 如实反映启用条件（native =
  core env；不得引导到对启用无效的设置页）
- `agent-workbench`：「Execution-body availability is stated, not discovered」的 cause
  口径反转——native 的启用条件是 core env，可见文案 SHALL 点名 env 标识符且不得引导
  到无效设置面；WebUI 内可补救的 cause 保留操作者语言原则（3c review 发现的主 spec
  矛盾，随本 change 一并收口避免归档后语料冲突）

## Impact

- `sebas-webui/frontend`：thinking 面板样式、usage 图表刻度生成、new-session-dialog
  不可用提示、usage 页数据源说明
- `AGENTS.md`：一句过时口径修正
- 测试面：上述各点的既有前端单测翻新 + 新增断言；webui 浏览器旅程不回归
- 非 BREAKING；无 API/wire 形状变化

## Non-goals

- round12 尾巴（fix-webui-qa-round12 tasks 4–6：模型切换回执收口、e2e 7 红回归修复、
  深链首载去重）——保留在 round12 change 内，由 /spec-go 与本 change 一并编排
- N-1（workbench 直建会话不进 session_map、core 重启蒸发）——功能级工程（native 后端
  接入映射持久化 + dormant 恢复 + 检查点转录），建议独立立项，不再随 QA 轮滚动
- 三个每日 QA 自动化（00:10/03:04/06:00）的合并——运维建议，已记录于 REPORT.md，
  不属代码变更
- allow 档门控行为——round2 M-C6 已裁决 allow 与 auto 同档（bypass tier、留审计），
  前端词汇已互相点名等价，不动
