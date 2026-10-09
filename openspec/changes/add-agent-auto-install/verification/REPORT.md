# add-agent-auto-install — GUI 验收记录（task 4.2）

沙箱：`invoke testsuite-webui-sandbox`（端口 9879，auth 关，两进程 core --webui + 独立 router --debug）。
PATH 垫 fake npm（`/tmp/sebas-gui-verify/fake-npm/npm`，经 shell export 透传进沙箱 webui 进程的 env）——**真实 npm 全程未被触发**。
浏览器：headless Chromium（CDP :9333）经 browser-harness 驱动，1440×1000。

## 验收清单

| # | 检查项 | 结果 | 证据 |
|---|---|---|---|
| 1 | Settings → Agents 出现「可安装 agent」区，claude / opencode 两配方行 | ✅ | `agents-section-before.png`：claude 显示「已安装」，opencode 显示「未安装 + 安装」 |
| 2 | 未装配方显示安装按钮（仅写权限角色） | ✅ | 同上；按钮文案「安装」 |
| 3 | 点击安装 → 成功通知 + 目录刷新出现新行且可达 | ✅ | `agents-section-after.png`：绿色通知「已安装 opencode 到 …/agent-tools/opencode/bin/opencode，版本 9.9.9，已添加目录行」；目录列表新增 `opencode 可达` |
| 4 | 状态判定复用探测口径（已装不再显示按钮） | ✅ | 安装后 opencode 行状态翻「已安装」，按钮消失 |
| 5 | 真实 npm 未被触发（fake npm argv 为证） | ✅ | fake npm argv 日志：`--version` / `install` / `--global` / `--prefix /tmp/sbtestsuite.*/agent-tools/opencode` / `opencode-ai` |

## 未在本轮 GUI 覆盖（诚实标注）

- **npm 缺失的失败通知**：进程级 e2e `install_without_npm_is_a_typed_400_and_leaves_no_trace` 与集成测试已覆盖（400 + Node.js 指引 + 零落盘）；GUI 层文案分支由前端 vitest 用例覆盖，未另跑浏览器形态。
- **busy 态**：fake npm 秒回，GUI 窗口内难稳定观测「安装中…」；该分支由前端 vitest 组件测试覆盖。
- **agent_created=false 提示**：进程级 e2e 已覆盖（重装 + config 种子两段）；GUI 层同理由 vitest 覆盖。
- **真实 npm 安装**：不在自动化验收面内（design D7）；真实链路需 operator 手跑（本机有 npm 时点一次即可，落 `<SEBAS_HOME>/agent-tools/<recipe>`）。

## 结论

GUI 主链路（打开设置 → Agent 分区 → 可安装区渲染 → 点击安装 → 通知 + 目录刷新）逐项通过，且以 fake npm argv 日志证明真实 npm 未被触发。task 4.2 完成。
