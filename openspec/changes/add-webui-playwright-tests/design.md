# design — add-webui-playwright-tests

## Context

测试金字塔现状：vitest 只覆盖组件单元（happy-dom，无真实浏览器）；Rust 进程级套件（`core_flow_e2e_test` / `acceptance_suite_test`）只打 HTTP 面；浏览器级 UI 渲染在验收账本中显式豁免。前端构建由 `sebas-webui/build.rs` 自动保证：cargo build 时 dist 过期即重跑 pnpm build，二进制内嵌最新 bundle。fake-claude 桩（`tests/bin/fake-claude.rs`）具备全套确定性触发：默认回复 "hello world"、`stream`（5 chunk + 800ms 停顿，为 debounce 泵预留瞬态窗口）、`perm` / bash / deny 场景（tool_use + hook_callback 审批，等待决策、fail-closed）、`refuse`（非终态拒绝）、`crash`（子进程死亡）。auth 装配已核实：bare-core `--webui` 形态读取 `[watchdog.webui].auth`，凭据经 `SEBAS_WEBUI_AUTH_FILE`（`src/run.rs:273`，与独立 webui 同套）。

## Goals / Non-Goals

Goals：真实浏览器（chromium）里的旅程级回归，一条命令可跑、失败留现场、绝不触碰真实实例。

Non-goals：见 proposal（watchdog 两进程形态、CI、非 chromium、模型正向路径等）。

## Decisions

### D1. 被测形态：core --webui 单进程调试形态

`sebas core -c <SB>/config.toml --router --debug --webui --webui-port <port>`，fake-claude 作 agent。

- 备选一：watchdog 两进程形态（`sebas webui` + core 经通道）——最接近生产，但 wire-webui-sebas-agent-e2e（2/11）记录的四处断点未补，审批/模型旅程必失败。待其落地后作为扩展形态加入，本变更的 spec 已按旅程而非形态表述，届时只加装配不改用例。
- 备选二：vite dev server + API mock——快，但测的是"前端对着假后端"，不是集成；与"集成/回归"的诉求不符。
- 调试形态是 AGENTS.md 已验证菜谱（`/health`、round-trip、审批全链路可跑），`--debug` 的内置 test provider 让 router 校验通过且不外拨。

### D2. 测试包位置：tests/webui-e2e 独立 pnpm 包

仓库无 pnpm workspace；frontend 也是自带 lockfile 的独立包。e2e 包独立放置：所有测试集中在 `tests/`（与 Rust 测试同层）、不动 operator 拥有的 `frontend/package.json`、playwright 重依赖（浏览器二进制）不进前端开发依赖。备选：塞进 frontend 包（耦合 dev 流程）、根级 package.json（给仓库添新 Node 面）——均否。

### D3. 装配：webServer 脚本 scripts/webui_e2e_server.sh

照 AGENTS.md 调试菜谱写一次性装配脚本（**不照抄** `test_webui_sandbox.sh`——那份用了旧命名 `SEBAS_GATEWAY_PROVIDER_OVERLAY` / `[router] state_file`，菜谱才是现行权威：`SEBAS_ROUTER_PROVIDER_OVERLAY`、`[dispatch] state_file`、`SEBAS_STATE_DB` 必须显式覆盖），auth 接线（`[watchdog.webui] auth` + `SEBAS_WEBUI_AUTH_FILE` + `webui-passwd` admin/admin）沿用 sandbox 脚本。脚本职责：建 throwaway 目录 → 写 config.toml（含 `[provider.anthropic]` 哑凭据满足 router validate）→ 起进程 → 轮询 `/health` → trap 退出时 SIGTERM + `rm -rf`（失败时按 `E2E_KEEP=1` 保留）。Playwright `webServer.command` 指向它、`url` 指向 `/health`，由 playwright 管进程组生命周期。双形态即两端口：auth 关 = 9899（主套件），auth 开 = 9898（鉴权 spec）。

### D4. Playwright 组织：双 config + 页面对象

- `playwright.config.ts`（主套件，auth 关）与 `playwright.auth.config.ts`（E2E_AUTH=1，仅鉴权 spec）——比 webServer 数组 + project 级 env 直白。retries=1（trace on-first-retry）作 flake 兜底。
- 页面对象：`Login` / `Workbench`（composer、project rail、summary）/ `Transcript` / `ReviewCard`；`helpers/api.ts` 用 playwright `request` 做 seed 与断言（会话列表、归档、auth/me）。会话 key 内嵌 NUL：一律复用应用自身的 `encodeURIComponent` 编码形态构造深链，不手工拼 URL。
- 断言纪律：web-first（`expect.toBeVisible/toHaveText` 带超时）+ `expect.poll`；流式瞬态用轮询捕获（桩的 800ms 窗口按 100ms 轮询必然命中），禁固定 sleep；关键 spec 收集 `pageerror` / console error（D4 场景要求"无控制台错误"）。

### D5. 账本同步与入口

- `invoke webui-e2e`（tasks.py）：`cargo build --bin sebas --bin fake-claude`（build.rs 自动保 dist 新鲜）→ 装 playwright 依赖（缺则 `pnpm install` + `playwright install chromium`，预检并给出清晰报错）→ 跑双 config → 失败保留现场。支持 `--case` 过滤单个旅程（透传 playwright 文件过滤）。
- 按 testsuite-acceptance 的账本同步规则，`tests/acceptance/COVERAGE.md` 增加 `testsuite-webui-browser` 行（非核心簇）；webui 能力矩阵中被浏览器旅程命中的"浏览器级 UI 渲染"豁免条目转为引用本套件证据。

### D6. 平台适配：Linux 主 + CI，Windows best-effort

调研结论（[Playwright 官方系统要求](https://playwright.dev/docs/intro#system-requirements) + 代码核查）：Playwright 支持 Windows 11+ / Windows Server 2019+ / WSL 与 Debian 12/13、Ubuntu 22.04/24.04/26.04，headless 为默认模式、无图形 CI/云端零适配（无需 X server，headless shell 减少系统依赖）；sebas 侧 in-process webui 走纯 TCP 127.0.0.1（平台无关），sebas-ipc 已有 Windows named pipe 分支（`interprocess` crate）。故 **Linux（本机 + GH ubuntu runner）为第一目标，Windows 本机 Git Bash 为 best-effort**，harness 层四项适配：

1. msys 下 `cygpath -w` 转换沙箱目录后再写入 config.toml（MSYS 的参数级路径转换帮不到文件内容，原生 exe 会把 `/tmp/...` 读成 `C:\tmp\...`）；
2. 二进制名按平台探测 `.exe` 后缀（AGENTS.md 既有约定）；
3. 清理语义按平台解读：POSIX SIGTERM 优雅退出，Windows 硬终止 + 目录删除尽力而为（一次性沙箱可接受，spec 已按此措辞）；
4. 新增 `.gitattributes`（`*.sh text eol=lf`），防 `core.autocrlf` 机器把脚本弄成 CRLF 弄崩 bash。

另：webServer 命令显式 `bash scripts/...`（不依赖 shebang，Git Bash 与 GitHub windows runner 的 bash 均在 PATH）；Windows 下 named pipe 全名 256 字符上限（sebas-ipc 注释明示），沙箱目录名取短；WSL 形态等效 Linux，适配项自动消失。

## Risks / Trade-offs

- [误触真实实例（9797 / 真实 ~/.sebas）] → 端口硬编码 9899/9898；脚本对全部默认路径 env 强制覆盖；spec 的"启动即隔离"场景逐项断言。
- [auth 在 bare-core 形态的读取点差异] → 已核实 run.rs 与独立 webui 同套（D3），无残余风险。
- [流式瞬态窗口 flake] → 桩 800ms 停顿 + 100ms 轮询 + retries=1 兜底；不断言瞬态持续时长，只断言"出现过"。
- [桩行为漂移破坏用例] → 触发词是仓库自有 fixture 的稳定契约，进程级套件同源依赖；用例把桩回复当 fixture 断言，漂移时两套件同时红，属期望行为。
- [无网环境浏览器下载失败] → invoke 任务预检 chromium 缓存，缺失时给出 `playwright install` 指引并 fail-fast。
- [Windows named pipe 256 字符上限撞长临时路径] → Windows 下沙箱目录名取短（如 `%TEMP%/sbe2eXXXX`）；unix UDS 不受影响。
- [Windows 硬终止丢优雅退出产物（state dump / socket 清理）] → 一次性沙箱、清理尽力而为；优雅退出语义仅在 POSIX 断言（spec 已按平台解读）。
- [测试只保证 chromium] → 接受：应用面向桌面 chromium 内核浏览器（Web Awesome + 现代 CSS），多浏览器矩阵列为 Non-goal。

## Migration Plan

纯增量：新包 + 新脚本 + invoke 任务 + 账本行。回滚 = 删除上述新增物，无生产代码与 API 变更。

## Open Questions

（无）

## 实现期发现（只记录不顺手修，见 spec Non-goals）

1. **WebUI review card 的 Allow-session 不持久（真实 bug）**：`webui` 的 `answer_permission`（`session_backend.rs`）把 AllowSession 映射成 `SendAcp PermissionReply`，但从未调用 `sebas-dispatch` 的 `SessionAllowlist::grant_all`（该登记在 `inbound.rs` 的 feishu 卡片点击路径上）。因此 WebUI 里点"本会话不再询问"后，同一会话的后续同类工具调用**仍会弹卡**。e2e 已捕获（`permission.spec.ts` allow-session 场景如实断言"第二次仍弹卡"）。修复需在 webui 回答路径同样 `grant_all`。
2. **`sebas-folder-picker` 渲染 wa-tree 懒加载子目录时崩溃 `nextSibling`（真实 bug）**：Web Awesome `wa-tree` 在 `item.append(child)` 时对 null 读 `nextSibling`。渲染根目录即触发一次（打开 Add-project 对话框即崩），与点选无关。e2e 以**手动路径输入**替代树点选规避；`collector.clean()` 豁免该已知 pageerror。`wa-tree` 需升级/修复。
3. **非终态错误（refuse）不在 WebUI 转写呈现**：`AcpEvent::Error{terminal:false}` 的 `❌` 文本只进 feishu 卡片 body，不进 webui 的 `turn_log`；且已 DONE 的会话收到非终态错误时 FSM 不移转（状态保持 done）。故 webui 上 refuse 没有任何可见文本——诚实断言为"会话存活且下一回合可用"，不断言虚假的错误文本。
4. **组合后端 provider 真源诚实降级**：`DualSessionBackend` 未重写 `state_snapshot`（trait 默认返回 None），`/api/settings` 上报 `providers_available: false`，composer 显示 "provider status unavailable"。这是当前如实行为（不是空的"未配置"），e2e 依此断言。
5. **沙箱需重定向 `SEBAS_PROJECTS_PATH`**：`/api/projects` 在 state store 不可达时回退到 `~/.sebas/projects.json`（真实 HOME），除非 `SEBAS_PROJECTS_PATH` 指向沙箱。harness 已设置（启动即隔离，spec 场景 1）。此前未设置时套件会读到开发机残留项目——已被捕获并修复。
