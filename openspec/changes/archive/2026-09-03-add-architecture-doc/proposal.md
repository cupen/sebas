# add-architecture-doc

## Why

sebas 的进程拓扑(watchdog / core / webui / gateway)与进程间通信语义目前散落在代码注释、提交历史和各模块 doc-comment 里,没有一处权威描述。新贡献者或后续 AI 会话要回答"哪些进程、谁拉起谁、控制走哪条通道、core 为什么默认不启动"这类问题,只能靠重新读代码拼凑。

## What Changes

- 新增 `docs/architecture.md`,作为进程结构与功能的单一事实来源,内容包括:
  - 进程全景:单一 `sebas` 二进制,子命令决定进程角色(watchdog / gateway / webui / control CLI / record / replay / update / service)
  - watchdog 监督模型:core / webui / gateway 三服务,spawn → readiness 门 → 崩溃退避的生命周期
  - IPC 语义:管道仅承载 readiness 握手(`{"cmd":"ready"}`),控制操作(启停/重启/升级/回滚/服务开关)走 Unix socket control RPC
  - 默认启动策略:仅 WebUI 默认启用,core(飞书 bot + ACP)与 gateway 默认停用,经 WebUI 服务页或配置开启并持久化到 services.json
  - CLI 控制面与 crate 职责速查
- 文档以当前代码实现为准(2026-08 快照),关键结论标注来源文件,便于日后核对
- 纯文档变更,不改动任何代码行为

## Capabilities

### New Capabilities

(无 — 本 change 为纯文档变更,不新增系统行为;已在 `.openspec.yaml` 设置 `skip_specs: true`)

### Modified Capabilities

(无 — 各既有能力的规格均不变)

## Non-goals

- 不修改任何代码、配置格式或进程行为
- 不重写 README:README 面向使用者,architecture.md 面向贡献者,二者定位不同
- 不变更 `openspec/specs/` 下既有能力规格
- 不做文档与代码的自动化一致性校验(可作为后续独立 change)

## Impact

- 新增 `docs/architecture.md` 一个文件
- 无代码、API、依赖或运行时影响
