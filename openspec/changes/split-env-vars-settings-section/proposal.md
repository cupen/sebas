## Why

revamp-settings-nav-and-models-editor 1.1 把 Env 分区并入了 Generic，Generic 从此名实不符：名字是"通用偏好"，内容却全是环境变量只读表。且当时后端没有 env 端点，值一列只能全部写死 "managed by core config"——操作员无法得知实例实际生效的配置路径、监听地址等真实值，敏感与非敏感也被一概而论。

## What Changes

- 设置弹窗新增只读分区 **Env Vars**（环境变量），环境变量表从 Generic 迁出；Generic 收敛为纯通用可配置项分区，暂留占位文案（语言切换等以后再说）。
- 新增只读端点 `GET /api/env`：webui 进程读取自身环境，返回策划过的变量清单（名字、解释、值）。**遮蔽在服务端完成**，敏感值永不出现在响应里。端点经既有鉴权。
- 变量三分类：
  - 非敏感（config/listener/state file 等路径与开关）→ 显示实际值；未设置则标注"未设置（用默认）"并在解释里写明默认值；
  - 敏感（password / token / secret 类）→ 只显示已设置/未设置，不显示值；
  - 内部管道与测试专用变量 → 不列。
- 顺带补遗漏：`SEBAS_STATE_DB` 纳入清单（当前 UI 表缺失）。
- 导航位置放底部 About 旁（两个都是只读参考性质），分区标签用 `Env Vars`，i18n 上线后再映射"环境变量"。

## Capabilities

### New Capabilities

（无——环境变量展示属于 webui 既有能力面的延伸）

### Modified Capabilities

- `webui`：「设置弹窗分区与缺省首项」的分区顺序改为 `generic → appearance ┊ services → models ┊ env-vars · about`；新增「环境变量只读展示（/api/env）」要求：三分类展示语义、服务端遮蔽、未设置标注、无 env 端点时的降级表现。

## Impact

- 代码：`sebas-webui/src/api.rs` + `server.rs`（新只读端点）、`sebas-webui/frontend/src/views/settings-modal.ts`（分区拆分 + 迁移 env 表）、`settings-modal.test.ts`。
- 无破坏性 API 变更；`/api/env` 为纯只读。
- **顺序依赖**：revamp-settings-nav-and-models-editor 的 spec delta 同改「设置弹窗分区与缺省首项」且尚未归档——须先归档它，再套用本变更，避免同 requirement 的 delta 冲突。

## Non-goals

- 不做环境变量的编辑/新增/删除（纯只读）。
- 不做 i18n（标签先用英文 `Env Vars`）。
- Generic 本期不新增任何可配置项，占位即可。
- 内部管道变量（`SEBAS_CORE_SOCKET`、`SEBAS_IPC` 等）与测试专用变量不进清单；node-link 内部变量 v1 不列。
