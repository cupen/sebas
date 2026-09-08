# Proposal — harden-core-channel-deployment

## Why

真机事故：webui 可用、添加项目返回成功，新建会话却报"核心不可达： socket absent"。根因是 core session channel 的武装完全依赖 `SEBAS_CORE_SECRET` env 注入（run.rs:322），任何脱离 watchdog 亲缘的组件（手动 `sebas webui`、升级半途的旧 core、secret 丢失）都会掉进"core 活着但 channel 永不武装"的盲区——watchdog 监督只看进程活着，看不见这个契约断裂；webui 也只在 composer 局部提示，加项目更被静默降级掩盖。

## What Changes

- **channel 自动武装**：core 不再依赖 env 才武装 channel；env 缺失时现场生成随机 secret，写 0600 secret 文件（路径由同一份 config 解析），env 显式提供时仍优先（watchdog 路径不变）。
- **secret 发现机制**：webui/router/im 解析顺序 env → secret 文件；客户端在握手失败/重连时重读文件，容忍 core 重启换钥；解析不到时启动 warn（对齐 im_cmd 现状）。
- **readiness 契约**：core 的 ready 信号延后到 channel 武装之后；channel bind 失败以退出码 75 退出 → supervisor 标 Degraded，消灭"Running 但没武装"。
- **webui 诚实外显**：app-shell 增加全局"核心不可达"横幅（带 cause，对齐现有 ws-banner）；项目注册走本地降级时响应携带降级标记，UI 如实提示"核心不可达，已写入本地注册表"。
- **测试补齐**：进程级 e2e 新增装配旅程（无 secret 启动即达 reachable——本次事故的回归用例；secret 轮换后老 webui 自愈；watchdog 监督重启后 webui 恢复——收窄账本缺口 #3）；浏览器旅程新增 detached 形态用例（横幅出现/恢复、降级提示）；沙箱配方随 auto-arm 简化（AGENTS.md 退役 `SEBAS_CORE_SECRET=fake` 仪式）。

## Capabilities

### New Capabilities

（无——全部落在既有能力上）

### Modified Capabilities

- `core-session-channel`：新增自动武装与 secret 文件发现/轮换要求；channel bind 失败的失败形态（退出码）。
- `watchdog`：readiness 契约——core ready 蕴含 channel 已武装；bind 失败 → Degraded 而非静默 Running。
- `webui`：全局核心可达性横幅；项目注册降级的如实呈现。
- `testsuite-process-e2e`：新增装配/轮换/监督恢复旅程。
- `testsuite-webui-browser`：新增 detached 形态横幅与降级旅程。

## Impact

- 代码：`src/run.rs`、`src/core_channel/{server,client}.rs`、`src/webui_cmd.rs`、`src/im_cmd.rs`、`src/watchdog.rs`（readiness 时序）、`sebas-webui/src/api.rs`、`sebas-webui/frontend/src/app-shell.ts`。
- 兼容性：watchdog 既有部署不受影响（env 优先）； secret 文件为新增产物，含迁移说明。
- 文档：AGENTS.md 沙箱配方简化。
- 安全：unix 上边界不变（socket 0600 + uid 校验）； secret 文件 0600 ≈ 同 uid 可读，与 env 实际强度等价，设计文档如实记录。

## Non-goals

- 不做跨机/网络通道，channel 仍是本机 IPC。
- 不引入密钥轮换调度/多钥共存（重启换钥 + 重连重读已够）。
- 不改 router admin bearer 体系（④ 边现状已够用）。
- 不做 supervisor 深度健康探测（连 channel 验活）——留作后续，依赖本 change 的 secret 文件先落地。
- 不改"webui 在 core 挂时继续服务"的姿态（既有 spec，本 change 只加强坏消息的显著度）。
