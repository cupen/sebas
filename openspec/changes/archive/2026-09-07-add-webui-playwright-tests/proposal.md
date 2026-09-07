# add-webui-playwright-tests

## Why

webui 的浏览器级 UI 渲染是验收账本里显式的豁免面：vitest 只做组件单元（happy-dom，非真实浏览器），Rust 验收套件只打 HTTP 面，真实浏览器里的旅程——登录、workbench 首屏、流式渲染、审批卡片点击、会话管理——没有任何自动化覆盖，回归全靠人肉点。Playwright 预写用例把这层钉死，同一套用例即可反复做集成与回归。

## What Changes

- 新增 `tests/testsuite-webui/` 独立 pnpm 包：`@playwright/test` + TypeScript，页面对象（登录、workbench、审批卡）与 API helper（seed / 断言 / 会话 key 取用）。
- 新增沙箱装配脚本 `scripts/webui_e2e_server.sh`：照 AGENTS.md 调试菜谱起 `sebas core --router --debug --webui`（fake-claude 桩、throwaway 目录、端口 9899/9898 ≠ 9797），支持 auth 关/开两形态（开 = admin/admin），退出 SIGTERM + 删沙箱目录。平台定位：Linux 本机/CI/云端 headless 为第一目标；Windows（Git Bash/WSL）best-effort，含 cygpath 路径、`.exe` 后缀、CRLF 三项脚本级适配。
- 首批旅程用例（9 个 spec）：auth 双形态；workbench 首屏（reachability 如实显示）；项目增删；会话往返与持久化；流式渲染（`stream` 触发词）；审批卡 allow-once / deny / allow-session；错误呈现（`refuse` / `crash`）；close / archive / 深链 / 退役路径重定向；模型面 D4 诚实缺省（fake-claude 无模型选项时不显示下拉、set_model 如实报错）。
- 新增 `invoke testsuite-webui` 任务：cargo build（build.rs 自动重建 dist）→ 起沙箱 → 跑 playwright → 清理。

## Capabilities

### New Capabilities

- `testsuite-webui-browser`: 浏览器级 e2e 套件的行为要求——沙箱装配边界（绝不触碰真实实例与端口 9797）、旅程覆盖面、断言稳定性、退出清理、invoke 入口。

### Modified Capabilities

（无——testsuite-acceptance 矩阵按既有"账本同步"规则补一行，是执行其现有 requirement，不改其行为）

## Impact

- 新增 `tests/testsuite-webui/`、`scripts/webui_e2e_server.sh`、`tasks.py` 新任务；`tests/acceptance/COVERAGE.md` 增加 testsuite-webui-browser 行。
- 依赖：`@playwright/test` + chromium（本地 devDependency，不进 Rust 构建链）。
- 零生产代码改动、零 API 变更。

## Non-goals

- watchdog 两进程形态的旅程（等 wire-webui-sebas-agent-e2e 落地后再扩）
- CI 接入（先本地 `invoke testsuite-webui`，稳定后另立变更）
- 非 chromium 浏览器、视觉快照、a11y 审计
- 模型选择正向路径：claude driver `model: None`、native 需真实凭据，均不可沙箱验证——只测 D4 诚实缺省
- IM / 飞书 UI、图片上传、admin 深旅程
