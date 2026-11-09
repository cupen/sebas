# 现有前端基线报告（L1）

- 采集日期：2026-11-08
- 分支：`feat/react-webui-redesign`，基线 commit `40d8ce3`（`git rev-parse --short HEAD`）
- 范围：既有 **Lit 3.3.3 + Web Awesome 3.12.0** 前端的单元测试、类型检查、生产构建、浏览器套件；外加本阶段蓝图的独立静态/截图验证。
- **不含**：React/AntD 生产迁移的实现或验收；未启动、未修改 operator 的 dev server；未触碰真实实例。

## 1. 隔离方式

为避免改动工作区里 27 个与本 change 无关的未提交文件，基线在 `/tmp/sebas-react-antd-baseline/worktree` 的 detached worktree 中执行：

```bash
git worktree add --detach /tmp/sebas-react-antd-baseline/worktree 40d8ce3
```

- `sebas-webui/frontend/node_modules` 与 `tests/testsuite-webui/node_modules` 是指向主工作区已安装依赖的符号链接；`diff -rq` 确认 worktree 与主工作区的前端源码完全一致（排除 `node_modules`、`dist`），且主工作区 `sebas-webui/frontend` 无未提交改动。
- 浏览器套件在沙箱端口 9894–9899 与临时目录内运行；未连接 operator 的 Vite 5273 端口。
- 所有日志位于 `/tmp/sebas-react-antd-baseline/`。

## 2. 执行结果总览

| 命令（cwd: `worktree/sebas-webui/frontend` 或 worktree 根） | 退出码 | 结果 | 日志 |
|---|---|---|---|
| `pnpm test` | 0 | 40 个测试文件、1023 个测试全部通过；耗时 7.31s | `frontend-test.log` |
| `pnpm exec tsc --noEmit` | 2 | **45 个类型错误**（41 个在 `*.test.ts`，4 个在 `src`） | `frontend-tsc.log` |
| `pnpm build` | 0 | 构建成功；JS 1,767.57 kB / gzip 533.81 kB，CSS 111.05 kB / gzip 16.69 kB；有单 chunk >500 kB 警告 | `frontend-build.log` |
| `invoke testsuite-webui`（cwd: worktree 根） | 1 | 主 config：**148 通过 / 10 失败 / 3 跳过**；失败后 `&&` 链中止，其余 5 个 config 未运行 | `testsuite-webui.log` |

`pnpm test` 的 Vitest 日志把 root 打印为主工作区路径，这是 `node_modules` 符号链接解析的结果；上表的源码一致性核对表明它测的是同一份 `40d8ce3` 前端源码。

## 3. 类型检查基线（已红，非本阶段引入）

`tsc --noEmit` 在干净 HEAD 上即失败，**未修复**（本阶段不改生产前端）。45 个错误的分布：

| 错误码 | 数量 | 含义 |
|---|---|---|
| TS2554 | 34 | 调用参数个数不匹配（多数为测试替身/`Element` 断言） |
| TS6133 | 5 | 未使用变量/导入 |
| TS2341 | 4 | 访问 private 成员（`SebasApp.railWidth`） |
| TS2339 | 2 | 属性不存在（`SessionDetail.active_session`、`Element.value`） |

非测试文件的 4 个错误：

- `src/components/line-chart.ts(368,28) TS6133: 's' is declared but its value is never read.`
- `src/components/line-chart.ts(403,25) TS6133: 'geo' is declared but its value is never read.`
- `src/views/dashboard.ts(2030,76) TS2339: Property 'active_session' does not exist on type 'SessionDetail'.`
- `src/views/project-rail.ts(230,10) TS6133: 'decodeSessionKeyTail' is declared but its value is never read.`

这 4 处是既有债务，可直接作为 React 迁移的清理项，但**不计入本阶段回归或修复**。

## 4. 浏览器套件基线（已红，非本阶段引入）

`invoke testsuite-webui` 先执行主 config（`playwright.config.ts`，端口 9899）即失败，命令链因此停止：

- 汇总：148 passed，10 failed，3 skipped；耗时 573.3s。
- 3 skipped：`agents.spec.ts` 的「保留 id 与重复 id」与「删除即时收场」未运行；`tiered-notices.spec.ts` 的 fatal 锁定用例跳过。
- 10 failed 按现象分组：
  - `agents.spec.ts`：新增 claude agent 后 `provider-key` 未变为「可达」。
  - `conversation.spec.ts`：上次模型对预选读回 `null`。
  - `first-paint.spec.ts`：创建对话框里 `native` 选项未被标记 disabled。
  - `pending-stack.spec.ts`：忙碌期提交未出现 `queued while busy` 文案。
  - `qa-round8-transcript-scroll.spec.ts`、`unread-badge.spec.ts`（2 条）、`unread-seam.spec.ts`：`session-unread` 徽标计数与预期 1 不符/超时。
  - `tool-call-fold.spec.ts`（2 条）：native 场景模型切换返回 HTTP 404（`test/tool-use`、`test/tools-parallel`）。
- 因首个 config 失败，`auth`、`auth-setup`、`detached`、`dead-core`、`native` 五个 config 未执行；**不能据此宣称其余 config 绿**。
- 失败沙箱保留在 `/tmp/sbtestsuite.0tw35ntp`（`core.log` 同目录），供复用排查。

结论：当前 `40d8ce3` 的浏览器套件基线不是全绿。上述红项在本阶段未修改任何前端源码的情况下出现，按**预存失败**记录；迁移矩阵（`journey-migration-matrix.md`）已逐条保留对应旅程，不因基线红而删除或弱化断言。

## 5. 蓝图验证（非生产 GUI 验收）

对 `react-antd-workbench-blueprint.html` 的独立验证：

- 静态检查：无 `http(s)://` 外链、无外部 `src`/`href`、无 CSS `url()`/`@import`；唯一根 ID `sebas-blueprint`；内嵌「设计稿，非生产实现」标记；脚本通过 `node --check`。
- 浏览器检查：Playwright 34 项断言全部通过，0 个 console/page error；覆盖工作台、会话、设置、用量、登录、首启、状态样本；发送/排队、流式、停止、审批、重置、viewer 只读、RBAC（admin/root/Users 分区）、断线、抽屉 Esc、reduced-motion。
- 视口：1440/1024/768/375 均无横向溢出（`scrollWidth <= clientWidth`）；375 抽屉展开后显示完整导航标签；浅色/深色主题文本对比可读。
- 产物：`/tmp/sebas-react-antd-baseline/screens/*.png`、`blueprint-check.json`、`blueprint-check.log`、独立预览 `blueprint-standalone.html`。
- 此验证只证明**设计稿可评审**，不代表 React 生产实现的视觉或行为验收。

## 6. 本阶段结论

1. 任务 1.1–1.5 的事实材料已齐：迁移清单、旅程矩阵、交互蓝图、性能预算、基线报告。
2. 基线红项为既有 `tsc` 错误与主 config 浏览器失败；本阶段未修复、未隐藏，也未把它们算作迁移成果。
3. 未进入第 2 阶段，未勾选 1.6，未修改生产代码/`package.json`，未提交。
