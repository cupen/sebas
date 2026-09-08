# tasks — harden-core-channel-deployment

> 进展（2026-09-08 agent 执行、验收人勾选）：§1–§4 代码已在 worktree 落地（commits 37912bf…84060f1），`cargo test --lib core_channel` 32 pass、`config::` 15 pass、supervisor 新单测 ok、webui api 端点 3 pass、app-shell 4 pass、live smoke（无 secret 启动即达 reachable、换钥自愈）通过。勾选仅代表代码+单测验收；e2e/稳定性/文档待 §5–§6。

## 1. core 自动武装与 secret 文件（design D1/D3/D5）

- [x] 1.1 config：新增 `[watchdog.core] secret_file` 显式键，缺省 `<config 目录>/core.secret`；解析函数 + 单测（显式键优先、缺省推导、沙箱 config 隔离）；验证：`cargo test config_test`
- [x] 1.2 core 自动武装：`src/run.rs` 武装门从"env 非空"改为恒武装；secret 取 env 或现场随机生成；原子写 secret 文件（tmp+rename，unix 0600），graceful exit 不删文件；单测：无 env 启动 → socket 与 secret 文件同现、0600、内容可完成握手；env 提供时 env 优先且文件内容一致；**双断言**：优雅退出后 secret 文件仍在、闩锁文件（`SEBAS_STARTUP_ERROR_FILE`）已清除——两套文件 lifecycle 语义相反，防后人"顺手统一"清理逻辑；验证：`cargo test --lib core_channel`
- [x] 1.3 bind 失败硬失败：把 `serve()` 的 bind 提前到主路径（spawn task 之前），bind 失败走 `exit_startup_failure` 统一出口（75 + 摘要行 + 错误文件）——`bind_channel_socket` 的 live 占用硬错误与 75 等价声明均已存在，本 task 只有传播层约 20 行；单测：路径被存活 listener 占用 → 启动函数返回特定错误并进程退出码 75；验证：`cargo test --lib core_channel` + 新增断言

## 2. 客户端 secret 发现（design D2）

- [x] 2.1 `CoreChannelBackend` secret 动态解析：env 缓存 → 每次连接前读 secret 文件 → 皆缺省输出一次 warn 并以空 secret 尝试；`secret rejected` / 断线重连时重读文件；单测：文件发现连接成功、换钥后重连自愈、双缺省 warn；验证：`cargo test --lib core_channel::client`
- [ ] 2.2 im 与 router 订阅侧接入同一解析函数（替换裸 `env::var`），保持既有 env 行为不变；验证：`cargo test`（im_cmd / router 订阅相关既有用例全绿）

## 3. readiness 契约（design D4）

- [x] 3.1 core ready 打点后移至通道 bind 完成之后；bind 失败路径不产生 ready 直接退出 75；单测覆盖启动时序（mock/shell 层断言 ready 前 socket 已存在）；验证：`cargo test --test testsuite_e2e_test -- startup`（既有启动用例不回归）——startup 过滤 3 用例全绿（2026-09-09）；bind 时序单测见 core_channel/tests.rs `arm_fails_hard_when_socket_path_is_taken_by_live_listener`
- [x] 3.2 supervisor 对 core 退出码 75 标 Degraded 的分类确认（复用 webui 既有分支，补 core 路径单测）；验证：`cargo test --lib watchdog::supervisor`

## 4. webui 诚实外显（design D6/D7）

- [x] 4.1 app-shell 全局"核心不可达"横幅：自持 `/api/summary` 轮询（与 composer 同间隔），`role=alert` + cause 文案，恢复即消失；前端单测覆盖出现/消失/不阻塞浏览；验证：`pnpm test`（app-shell.test.ts）
- [x] 4.2 `POST /api/projects` 降级标记：本地降级路径响应携带 `degraded: {cause}`，状态库路径不带，双失败仍 503；端点测试；验证：`cargo test -p sebas-webui api_endpoints`
- [x] 4.3 前端降级提示：项目落栏时就地提示"核心不可达，已写入本地注册表"；前端单测；验证：`pnpm test`（projects 相关组件测试）

## 5. 测试套件（spec 场景逐条落地）

- [x] 5.1 进程 e2e「无密钥装配旅程」：双进程均无 `SEBAS_CORE_SECRET` 启动 → reachable + 会话往返（事故回归用例）；并确认既有带 env 用例全绿；验证：`invoke testsuite-e2e --case <new>`
- [x] 5.2 进程 e2e「密钥轮换自愈旅程」：kill core → 同 config 重启（新钥）→ 不重启的 webui 恢复 reachable，期间 cause 如实；验证：`invoke testsuite-e2e --case <new>`
- [x] 5.3 进程 e2e「监督重启恢复旅程」：watchdog 监督形态拉起 core+webui，杀 core → supervisor 自动重启 → webui 恢复（收窄账本缺口 #3）；验证：`invoke testsuite-e2e --case <new>`
- [x] 5.4 浏览器 detached 变体：`testsuite-webui` 新增**可复用的双进程沙箱 fixture**（core + 独立 webui，auth 关）与 `--case deployment` 旅程：core 停 → 横幅出现含 cause、加项目出现降级提示、composer 门禁生效；core 恢复 → 横幅消失；**fixture 是交付物**：cover B1 的 detached approval e2e 直接复用，不重写 harness；验证：`invoke testsuite-webui --case deployment`——全绿（10.2s，2026-09-09）；主套件 32 passed 回归确认；fixture = tests/testsuite-webui/tests/helpers/detached.ts（stopCore/startCore/isCoreAlive/waitForCoreReachability/detachedSceneDir），harness 经 TESTSUITE_MODE=detached（tasks.py，core+独立 webui、无 SEBAS_CORE_SECRET、pids.json 发布）
- [ ] 5.5 稳定性复跑：同一提交连续 3 次 `invoke testsuite-e2e` 全绿 + 3 次 `invoke testsuite-webui --case deployment` 全绿；验证：复跑记录落在任务备注

## 6. 文档与账本

- [ ] 6.1 AGENTS.md 沙箱配方简化：退役 `SEBAS_CORE_SECRET=fake` 注入仪式（env 三件套中仅保留与通道无关项），保留 `channel_path` 显式要求；验证：按新配方从零手跑一次沙箱可达
- [ ] 6.2 COVERAGE.md 更新：缺口 #3 标注收窄证据（5.3 旅程）、core-session-channel / webui / testsuite 各行补新旅程证据；验证：矩阵无空白条目
- [ ] 6.3 fail-fast 回归重跑：本 change 动了 `run.rs` 武装门与 bind 路径，fail-fast 的 `startup_failure_core/run` e2e 重跑一次（rebase 验证，非重做）；若 COVERAGE 缺口 3 的"进程级注入不可达"注记因 spawn 路径变化部分失效，同步改写注记；验证：`invoke testsuite-e2e --case startup_failure` 全绿
