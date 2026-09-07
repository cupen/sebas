# tasks — harden-core-channel-deployment

## 1. core 自动武装与 secret 文件（design D1/D3/D5）

- [ ] 1.1 config：新增 `[watchdog.core] secret_file` 显式键，缺省 `<config 目录>/core.secret`；解析函数 + 单测（显式键优先、缺省推导、沙箱 config 隔离）；验证：`cargo test config_test`
- [ ] 1.2 core 自动武装：`src/run.rs` 武装门从"env 非空"改为恒武装；secret 取 env 或现场随机生成；原子写 secret 文件（tmp+rename，unix 0600），graceful exit 不删文件；单测：无 env 启动 → socket 与 secret 文件同现、0600、内容可完成握手；env 提供时 env 优先且文件内容一致；验证：`cargo test --lib core_channel`
- [ ] 1.3 bind 失败硬失败：通道 bind 失败时 core 以 `EXIT_BIND_FAILED`(75) 退出（不再静默运行）；单测：路径被存活 listener 占用 → 启动函数返回特定错误并进程退出码 75；验证：`cargo test --lib core_channel` + 新增断言

## 2. 客户端 secret 发现（design D2）

- [ ] 2.1 `CoreChannelBackend` secret 动态解析：env 缓存 → 每次连接前读 secret 文件 → 皆缺省输出一次 warn 并以空 secret 尝试；`secret rejected` / 断线重连时重读文件；单测：文件发现连接成功、换钥后重连自愈、双缺省 warn；验证：`cargo test --lib core_channel::client`
- [ ] 2.2 im 与 router 订阅侧接入同一解析函数（替换裸 `env::var`），保持既有 env 行为不变；验证：`cargo test`（im_cmd / router 订阅相关既有用例全绿）

## 3. readiness 契约（design D4）

- [ ] 3.1 core ready 打点后移至通道 bind 完成之后；bind 失败路径不产生 ready 直接退出 75；单测覆盖启动时序（mock/shell 层断言 ready 前 socket 已存在）；验证：`cargo test --test testsuite_e2e_test -- startup`（既有启动用例不回归）
- [ ] 3.2 supervisor 对 core 退出码 75 标 Degraded 的分类确认（复用 webui 既有分支，补 core 路径单测）；验证：`cargo test --lib watchdog::supervisor`

## 4. webui 诚实外显（design D6/D7）

- [ ] 4.1 app-shell 全局"核心不可达"横幅：自持 `/api/summary` 轮询（与 composer 同间隔），`role=alert` + cause 文案，恢复即消失；前端单测覆盖出现/消失/不阻塞浏览；验证：`pnpm test`（app-shell.test.ts）
- [ ] 4.2 `POST /api/projects` 降级标记：本地降级路径响应携带 `degraded: {cause}`，状态库路径不带，双失败仍 503；端点测试；验证：`cargo test -p sebas-webui api_endpoints`
- [ ] 4.3 前端降级提示：项目落栏时就地提示"核心不可达，已写入本地注册表"；前端单测；验证：`pnpm test`（projects 相关组件测试）

## 5. 测试套件（spec 场景逐条落地）

- [ ] 5.1 进程 e2e「无密钥装配旅程」：双进程均无 `SEBAS_CORE_SECRET` 启动 → reachable + 会话往返（事故回归用例）；并确认既有带 env 用例全绿；验证：`invoke testsuite-e2e --case <new>`
- [ ] 5.2 进程 e2e「密钥轮换自愈旅程」：kill core → 同 config 重启（新钥）→ 不重启的 webui 恢复 reachable，期间 cause 如实；验证：`invoke testsuite-e2e --case <new>`
- [ ] 5.3 进程 e2e「监督重启恢复旅程」：watchdog 监督形态拉起 core+webui，杀 core → supervisor 自动重启 → webui 恢复（收窄账本缺口 #3）；验证：`invoke testsuite-e2e --case <new>`
- [ ] 5.4 浏览器 detached 变体：`testsuite-webui` 新增双进程沙箱装配（core + 独立 webui，auth 关）与 `--case deployment` 旅程：core 停 → 横幅出现含 cause、加项目出现降级提示、composer 门禁生效；core 恢复 → 横幅消失；验证：`invoke testsuite-webui --case deployment`
- [ ] 5.5 稳定性复跑：同一提交连续 3 次 `invoke testsuite-e2e` 全绿 + 3 次 `invoke testsuite-webui --case deployment` 全绿；验证：复跑记录落在任务备注

## 6. 文档与账本

- [ ] 6.1 AGENTS.md 沙箱配方简化：退役 `SEBAS_CORE_SECRET=fake` 注入仪式（env 三件套中仅保留与通道无关项），保留 `channel_path` 显式要求；验证：按新配方从零手跑一次沙箱可达
- [ ] 6.2 COVERAGE.md 更新：缺口 #3 标注收窄证据（5.3 旅程）、core-session-channel / webui / testsuite 各行补新旅程证据；验证：矩阵无空白条目
