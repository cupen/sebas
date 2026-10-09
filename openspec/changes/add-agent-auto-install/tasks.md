# add-agent-auto-install — Tasks

## 1. 后端安装模块（sebas-webui）

- [x] 1.1 新建 `sebas-webui/src/agent_install.rs`：封闭配方表（claude → `@anthropic-ai/claude-code`/bin `claude`；opencode → `opencode-ai`/bin `opencode`，含各自 agent 定义模板）、私有前缀推导（`sebas_home()`/`agent-tools/<recipe>`）、bin 绝对路径平台分支（unix `<prefix>/bin/<bin>`、windows `<prefix>/<bin>.cmd`）。验证：单测覆盖前缀派生（钉 `SEBAS_HOME`）、未知 recipe 的 typed 拒绝、两配方模板形状（opencode 的 args 首词 `acp`）。
- [x] 1.2 安装执行：spawn `npm install --global --prefix <prefix> <package>`（先探测 `npm --version` 判 npm 在场），`tokio::time::timeout` 300s、非零退出携带 stderr 尾部摘要、成功后对私有 bin 复用 `discover_agent` 口径探测并返回 `{installed, path, version}`。验证：单测用 PATH 垫 fake `npm` 脚本（成功/退出非零/挂起超时三剧本）断言 argv 含 `--prefix` 与包名、错误摘要与超时路径。
- [x] 1.3 装完建行：按 spawn 解析口径（agents store ∪ config 注册表）判该 id 是否已有定义；无则经 `state_mutate("agents", {"op":"put",…})` 建标准行（path 指私有 bin、sessions_dir/work_dir 留空），有则不动，返回 `agent_created`。验证：单测覆盖三情形——全新建行、store 已有行不动、config 种子条目受尊重。

## 2. 路由、权限与联动

- [x] 2.1 挂载 `POST /api/agents/install`（body `{recipe}`）：SettingsManage 权限 + loopback origin 守卫 + per-recipe in-flight 锁（并发同 recipe → 409）、npm 缺失 typed 4xx 带指引、错误走既有错误信封。验证：`sebas-webui` 集成测试断言未知 recipe 4xx、无权限 4xx、外源 403、并发 409、fake npm 成功路径 201/200 响应形状。
- [x] 2.2 目录联动：安装建行后 `GET /api/agents` 立即反映新行（无需重启）。验证：集成测试在安装请求后直接读目录断言新行 reachable 与 path 绝对路径。

## 3. 前端（sebas-webui/frontend）

- [x] 3.1 `api/client.ts` 新增 `agentInstall(recipe)` 与响应类型。验证：Vitest 单测断言 POST 路径与载荷形状。
- [x] 3.2 Settings → Agents 分区新增「可安装 agent」区：配方行（名称、npm 包名、状态徽标复用探测口径——bin 名与私有前缀均不可达才显示「安装」），安装按钮 busy 态、成功/失败通知（失败含 stderr 摘要或 npm 缺失指引）、成功后刷新目录。写按钮按 `canManageAgents` 裁剪。验证：Vitest 组件测试覆盖未装显示按钮、已装不显示、busy 期间禁用、失败通知文案。
- [x] 3.3 安装响应中 `agent_created=false` 时向操作员明示「已存在定义未改动」。验证：Vitest 断言该提示分支。

## 4. 进程级与浏览器验收

- [x] 4.1 进程级 e2e：沙箱（`SEBAS_HOME` 钉一次性目录）+ PATH 垫 fake `npm`，旅程 = 打开 Settings → Agents → 点安装 → 目录出现新行且可达 → 新建会话对话框可选。验证：`tests/testsuite_e2e_test.rs` 新旅程用例通过（`invoke testsuite-e2e --case <name>`）。
- [x] 4.2 浏览器 GUI 验收：`invoke testsuite-webui-sandbox`（auth 关）手验安装按钮、busy、通知与目录刷新；确认真实 npm 全程未被触发（fake npm 日志为证）。验证：沙箱会话记录 + 截图，验收清单落 `verification/`。

## 5. 回归与收口

- [x] 5.1 全量回归：`rtk cargo test`、`rtk pnpm vitest`（frontend）、`cargo clippy` 与 `cargo fmt --check` 无新告警。验证：命令退出码 0。
- [x] 5.2 规范与工件收口：`openspec validate add-agent-auto-install --strict` 通过；specs 增量与实现逐场景对账（尤其「npm 缺失诚实报错」「重复安装即升级」「config 种子受尊重」三场景）。验证：validate 输出绿。
