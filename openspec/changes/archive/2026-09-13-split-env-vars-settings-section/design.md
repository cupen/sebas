# 设计：split-env-vars-settings-section

## Context

revamp-settings-nav-and-models-editor 1.1 把 Env 并入 Generic 后，Generic 名实不符：全是环境变量只读表（前端硬编码清单，值列一律写死 "managed by core config"）。操作员无法得知实例实际生效的配置路径，敏感与非敏感一概而论。见 proposal.md — Why。

## Goals / Non-Goals

**Goals:** Generic 收敛为纯偏好分区（占位文案）；新只读分区 Env Vars；新端点 `GET /api/env` 服务端策划 + 服务端遮蔽；补 `SEBAS_STATE_DB`。
**Non-Goals:** 不做环境变量的编辑/写入；不做语言切换本身（只留 IA 位置）；i18n 映射不在本轮（分区标签用英文 `Env Vars`）；内部/测试变量一律不列（不是隐藏值，是整个条目不出现）。

## Decisions

### D1. 策划清单落服务端 Rust（sebas-webui），前端不再持有清单

`GET /api/env` 在 webui 进程内 `std::env::var` 逐项读取。清单定义为静态表（名字、解释、分类、未设置时的默认值说明），与前端硬编码表一一对应迁移 + 补 `SEBAS_STATE_DB`。理由：遮蔽必须在服务端（敏感值不过 wire），清单进 Rust 后前端只做渲染，单一事实源。当前前端表里 `SEBAS_WEBUI_TOKEN` 的描述含"token or password accepted"——按现状文案迁移，RBAC 变更落地时由其同步收敛。

### D2. 三分类与展示形态

响应每项 `{ name, what, kind, value }`：`kind: "plain" | "set_unset"`；`plain` 已设置 `value = 实际值`，未设置 `value = null` + `what` 携带默认值说明（前端标注「未设置（用默认）」）；`set_unset` 项 `value = null`，仅 `what`/独立布尔表达已设置与否——**敏感值在任何字段都不出现**。分类归属：`*_PASSWORD` / `*_TOKEN` / `*_SECRET` / `FEISHU_APP_SECRET` → set_unset；其余 → plain。

### D3. 端点性质与降级

纯 webui 面（读自身 env），不依赖 core 通道——core 不可达时端点照常工作。经既有 webui 鉴权中间件（auth=false 沙箱直通）。前端 Env Vars 分区加载失败 → 分区内联错误态（复用既有分区错误呈现模式），不渲染空表假象。

### D4. 分区顺序落 nav 表

settings-modal.ts 分区表加 `env-vars` 项，置于底部组 `Env Vars · About`（弹性留白 + 分隔线组内与 About 并列）；Generic 主区换占位文案；env 表渲染逻辑迁入 env-vars 分区组件。localStorage 记忆/回退逻辑不变（新分区名进表即自动可记忆）。

## Risks / Trade-offs

- [策划清单与实际环境变量漂移] → 清单迁移自现役 UI 表 + code grep 校对（SEBAS_STATE_DB 纳入）；后续新变量由 code review 把关。
- [`SEBAS_WEBUI_PASSWORD` 描述在 RBAC 后过时] → 属 RBAC 变更的 spec/实现责任，此处保持现状语义。
- [值列显示实际路径可能含用户名等本机信息] → 端点在既有鉴权之后，查看者本就是本机操作员；与 `/api/summary` 的 workspace 根目录披露同级，可接受。

## Migration Plan

纯增量端点 + 前端分区迁移，无数据迁移。回滚 = revert。

## Open Questions

（无。）
