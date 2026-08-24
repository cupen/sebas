## Why

sebas-61l 回填计划收尾批。覆盖**控制平面与运维面**：watchdog 守护（升级/回滚/服务管理/控制 RPC）、WebUI 面板、CLI 与 systemd 服务安装、离线回放调试。归档后 baseline 完整覆盖全部运行面。

## What Changes

- 新增 4 份 capability spec（英文，格式同前）：
  - `watchdog`：守护进程——core 子进程监管、升级/回滚执行与安全（超时/自动回滚）、控制 RPC（鉴权/幂等/事件时间线/危险操作确认）、gateway/webui 服务管理、bare-core 模式
  - `webui`：本地 Web 面板——路由面、本地安全基线、会话查看/切换/关闭、管理动作（restart/rollback/update）、与 watchdog 的生命周期联动
  - `cli-service`：CLI 子命令面（run/watchdog/gateway/webui/replay/control）、install-service 与 systemd unit 生成（--user 降权）、配置发现
  - `replay-debug`：--dump-inbound 录制与 `sebas replay` 离线回放——录制格式、回放保真（与线上同路径）、副作用边界
- **范围调整**：原计划的 `upgrade-command` 并入 `watchdog`——命令解析与转发语义已在 `router-commands` 覆盖，升级执行细节属 watchdog 控制面，单独成 spec 会两边重复
- 纯文档，无代码改动

## Capabilities

### New Capabilities

- `watchdog`: 守护与控制平面——watchdog 父进程监管 core 子进程、升级（dev/release/dry-run）与回滚的安全执行、控制 RPC 鉴权与幂等、事件时间线、危险操作确认授权、gateway/webui 服务生命周期、bare-core 降级模式。
- `webui`: 本地管理面板——HTTP 路由面、仅本机绑定等安全基线、会话仪表盘（查看/聚焦切换/关闭）、管理动作端点、watchdog 服务联动。
- `cli-service`: 命令行与服务安装——子命令树、无参数默认行为（裸 `sebas` 为解析错误）、install-service 生成 systemd system unit（--user 降权运行）、配置文件发现（每子命令显式 --config，默认 ./config.toml）、环境变量。
- `replay-debug`: 调试回放——入站事件录制（命名/路径/内容）、离线回放走与线上相同的路由路径、回放过滤器差异、出站副作用边界。

### Modified Capabilities

（无）

## Non-goals

- **不**重复 `router-commands` 已覆盖的「命令→watchdog 转发」语义（只写 watchdog 侧执行）
- **不**覆盖 webui 前端页面视觉细节（只写路由与行为）
- **不**改代码

## Impact

- 新增 `openspec/specs/{watchdog,webui,cli-service,replay-debug}/spec.md`
- 代码零改动
- 归档后 `openspec/specs/` 共 15 个 capability，回填完成（bead sebas-61l 可关闭）
