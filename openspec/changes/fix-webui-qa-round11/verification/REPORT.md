# fix-webui-qa-round11 修复验证报告

- 日期：2026-10-04
- 验证人：主 agent（spec-go 3b 验收 + 3c 后 GUI 复核，Playwright 真实点击/输入，截图证据在 `shots/`）
- 被测：http://127.0.0.1:9877/（沙箱：`target/qa-r11-sandbox/`，fake-claude + native `test/*` 场景，auth 开，admin/member/viewer 三角色）
- 脚本：`v1.mjs`（核心链路 13 项全 PASS）、`v2.mjs`（角色门禁/多行/深链/别名/Esc/usage 12 项 PASS + W7 见下）
- 最终构建：feat/webui-qa-round11（含 3a 实现 + 3c review 修复 + 本轮 GUI 复核追加修复）

## 11 项验收缺陷销账

| 缺陷 | 结论 | 证据 |
|---|---|---|
| A-1 viewer 可删技能 | 修复：viewer 技能页无删除控件；直发 DELETE 403；仓目录原样 | w1 ×3 |
| B-1 native 状态恒 Queued | 修复：rail working→done/failed 推进、计数如实、首条消息自动命名 | v04/v05 |
| B-2 native 流式无停止钮 | 修复：流式中「停止回复」出现，点击即取消（取消提示落转录），跟发回合正常 | v04 ×3 |
| B-3 后台完成无通知 | 修复：历史页停留时回合完成弹「会话「…」的回合已完成。」；第 2+ 回合（re-arm）同样弹出 | v06 ×2 |
| A-2 别名空下拉 | 修复：零 store provider 时禁用态 + 指引文案 | w4（截图） |
| B-4 多行气泡渲染单行 | 修复：三行消息渲染三行，刷新后保持 | w2 ×2 |
| B-5 深链 404 轮询刷屏 | 修复：404 有界（首 3.5s 后零增长）、无重试环、不可得呈现清晰 | w3 |
| A-3+B-6 WS 未认证噪音 | 修复：登录页零 console 错误（原 50+ 条）；登出后静默 | v01/v07 |
| A-4 设置弹窗失焦 Esc | 修复：不点击弹窗内部直接 Esc 即关闭 | w5 |
| A-5 usage 刻度截断 | 修复：右缘 inset 12→32，两种粒度最右刻度完整 | w6（截图） |
| A-6 删除 agent 文案歧义 | 修复：单表达式完整四短句（代码+单测钉死「中立」绝迹）；GUI 表单流自动化受阻，以单测+代码核验结案 | w7（受限）+ 单测 |

## 3c review 产出与追加修复

- e2e ×2 新增（native 生命周期推进 + 第 2+ 回合 re-arm 武装帧），后者揪出**真缺陷**：
  `message()` 仅在排队分支发相位帧 → 已修（无条件发帧，空闲直投帧带 working 真值）。
- 既有 e2e `test_model_tool_loop…` 的 FS 探针在 Windows 假红（bash 工具 unix-only，round11
  proposal Non-goal）→ FS 断言加 `#[cfg(unix)]`，Windows 全量套件不再永远红。
- **GUI 复核追加修复（N-2，P2）**：侧栏「历史」「用量」链接点击触发**整页刷新**而非 SPA 路由——
  组头链接的 `stopPropagation` 把事件挡死在 rail 内，document 级 SPA 拦截器收不到，浏览器回落
  原生 href。后果：丢 SPA 状态（分栏/未读游标/turn-notify 迁移锚）并连带吞掉历史页的回合终点
  通知（B-3 修复在历史页不可达）。已修（锚不断传播、按钮折叠判定改 composedPath）+ 单测回归。

## 新发现（不在本轮 11 项内，建议另立 change）

- **N-1（P1，未修）**：workbench 直建的会话（native 与 web 渠道）不进 `projects.db` 的
  `session_map` 持久化——`session_map` 表恒 0 行，core 重启（含 /F kill）后全部会话蒸发。
  违反 session-persistence 的 per-mutation 落库语义（「逐变更落库、无关停 dump」）。修复是
  功能级工程（native 后端接入映射持久化 + dormant 恢复 + 检查点转录），超出本轮范围。
  复现：GUI 建会话 → `sqlite3 target/qa-r11-sandbox/projects.db 'SELECT COUNT(*) FROM session_map'`
  → 0；重启 core → 会话列表空。

## 环境备注

- 验收早期曾把沙箱放在 `%TEMP%/sebas-qa-r11`，该目录在验证中途被外部因素整目录清除
  （无本流程删除操作；嫌疑指向 Windows Temp 自动清理），后迁至 `target/qa-r11-sandbox/`。
  QA-A/QA-B 的原始 findings 已从会话记录恢复至 `findings-a.md` / `findings-b.md`
  （截图证据不可再生，已在文件头注明）。
- bd（beads）在本机不可用（嵌入 Dolt 需 CGO、bootstrap 同败、无 dolt server）——本轮
  任务跟踪由 tasks.md 承载。
