# tasks — add-webui-playwright-tests

## 1. 脚手架与沙箱装配

- [x] 1.1 建 `tests/webui-e2e` 独立 pnpm 包：`package.json`（@playwright/test devDep）、`tsconfig.json`、`playwright.config.ts` 骨架（baseURL 9899、chromium-only、retries=1、trace on-first-retry）、`test-results/` 忽略；验证：`pnpm install` 与 `pnpm exec playwright --version` 成功
- [x] 1.2 写 `scripts/webui_e2e_server.sh`（auth 关，端口 9899）：throwaway 目录、按 AGENTS.md 菜谱写 config.toml（fake-claude、dispatch/media/acp/watchdog/router 全沙箱路径、`[provider.anthropic]` 哑凭据）、env 强制覆盖（`SEBAS_CORE_SECRET`/`SEBAS_STATE_DB`/`SEBAS_STATE_FILE`/`SEBAS_ROUTER_PROVIDER_OVERLAY`）、起 `sebas core --router --debug --webui --webui-port 9899`、轮询 `/health`、trap 内 SIGTERM + `rm -rf`（`E2E_KEEP=1` 保留现场）；平台适配（design D6）：msys 下 `cygpath -w` 转换沙箱目录写配置、`.exe` 后缀探测、Windows 短目录名；验证：Linux 上脚本起后 `curl /health` 返回 ok，结束（term）后目录删除、端口释放、`/api/summary` 的 execution acp `ok: true`
- [x] 1.3 扩装配脚本支持 auth 开形态（`E2E_AUTH=1` → 端口 9898、`[watchdog.webui] auth = true`、`SEBAS_WEBUI_AUTH_FILE` + `sebas webui-passwd` 建 admin/admin）；验证：`E2E_AUTH=1` 起后 `/api/auth/me` 报告鉴权开启，admin/admin 登录返回 200、错误密码 401
- [x] 1.4 平台收尾：新增 `.gitattributes`（`*.sh text eol=lf` 等）；playwright config 的 webServer 命令显式 `bash scripts/...`（不依赖 shebang）；验证：`.gitattributes` 生效（`git check-attr eol scripts/webui_e2e_server.sh` 报 lf），config 里两处 webServer 均为显式 bash 调用

## 2. 页面对象与 helper

- [x] 2.1 写 `tests/helpers/api.ts`：playwright `request` 封装（summary、sessions 列表/详情、归档列表、auth/me/login）+ 会话 key `encodeURIComponent` 编码工具；验证：一条对沙箱的冒烟断言脚本取到 `reachability` 且 acp `ok: true`
- [x] 2.2 写页面对象 `Login` / `Workbench`（composer、project-rail、summary 区）/ `Transcript` / `ReviewCard` / `ProjectRail` 与 console/pageerror 收集 helper；验证：临时冒烟 spec 在真实沙箱上定位到 composer 与项目栏后删除

## 3. 旅程用例（spec-driven，逐条对应 spec 场景）

- [x] 3.1 `first-paint.spec.ts`：根路径渲染项目栏/composer/summary，reachability 显示沙箱真实状态（acp ok、native 如实不可用）；验证：spec 过且无 console error
- [x] 3.2 `session-roundtrip.spec.ts`：composer 提交文本 → 用户消息与桩回复（"hello world"）按序出现 → 状态收敛 Done → 刷新后回到会话 transcript 从持久化恢复；验证：spec 过
- [x] 3.3 `streaming.spec.ts`：`stream` 触发词 → `expect.poll` 观察到分批 chunk 与运行中瞬态状态 → 最终完成；验证：spec 过且断言零固定 sleep
- [x] 3.4 `permission.spec.ts`：`perm` 触发 → review card 出现（含 Bash 命令语义）→ deny 路径（拒绝语义工具结果、回合完成）与 allow-once 路径（允许语义）→ allow-session 后再次触发同类调用；验证：三条场景 spec 过。⚠️ 发现：WebUI `answer_permission` 的 AllowSession 未登记 `grant_all`，故同一会话第二次同类调用仍弹卡（如实断言"仍弹卡"，见 design 实现期发现 1）
- [x] 3.5 `errors.spec.ts`：`refuse` → 非终态错误呈现且下一条消息仍可用；`crash` → 子进程死亡如实呈现、无伪装成功；验证：spec 过
- [x] 3.6 `projects.spec.ts`：经 folder-picker 添加沙箱内目录 → 项目栏出现 → 移除 → 消失；验证：spec 过。⚠️ 发现：`wa-tree` 懒加载渲染崩溃 `nextSibling`（读 null），经 Add-project 对话框**手动路径输入**规避（见 design 实现期发现 2）
- [x] 3.7 `session-mgmt.spec.ts`：close 后离开活动列表；archive 后活动列表隐藏且归档视图可见；会话深链经 SPA fallback 直达；`/settings` 重定向 `/`；桩会话无模型下拉（D4）且无 console error；验证：五条场景 spec 过
- [x] 3.8 `auth.spec.ts` + `playwright.auth.config.ts`（E2E_AUTH=1、9898、仅本 spec）：免登录直达（对照主套件）、错误凭据拒绝、admin/admin 登录进 workbench、登出回未鉴权态、鉴权下深链重定向登录页；验证：`pnpm exec playwright test --config playwright.auth.config.ts` 过

## 4. 入口与账本

- [x] 4.1 tasks.py 加 `invoke webui-e2e`（`cargo build --bin sebas --bin fake-claude` → pnpm/浏览器依赖预检与安装 → 跑主 config + auth config → 失败保留现场输出路径）与 `--case` 单旅程过滤；验证：仓库根 `invoke webui-e2e` 一键全绿；`invoke webui-e2e --case auth` 仅跑鉴权 spec 且清理行为一致
- [x] 4.2 `tests/acceptance/COVERAGE.md` 增加 `testsuite-webui-browser` 行（非核心簇、含证据引用），webui 能力行中"浏览器级 UI 渲染"豁免条目转为引用本套件旅程；验证：矩阵行完整、无空白条目

## 5. 收尾

- [x] 5.1 稳定性复跑：同一提交连续 3 次 `invoke webui-e2e` 全绿（flake 检查，任何偶发失败按 design D4 断言纪律修复）；验证：3/3 通过
- [x] 5.2 README 测试小节补一行入口说明（`invoke webui-e2e` 与失败留现场用法）；验证：文档含可复制的命令
