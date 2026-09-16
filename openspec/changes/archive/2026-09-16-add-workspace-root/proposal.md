## Why

项目目录目前没有任何机器级边界：`POST /api/projects` 可以注册本机任意路径（手动输入路径明确不受树根约束），已注册项目在查看和打开时也不复查路径；远端节点注册同样只判存在性。操作者需要一个可配置的工作区根目录（workspace root），让服务端严格把关：项目只能建在根下，也只能看/开根下的项目。

## What Changes

- 新增**工作区根目录**概念，按机器各自生效、互不影响；取值顺序：`SEBAS_WORKSPACE_ROOT` 环境变量 > 配置项 > 回退进程 cwd 并在启动时打告警（即边界恒存在）。
- 服务端严格校验（canonicalize 后逐分量前缀比较，fail-closed）：
  - 本机项目注册：路径必须落在 workspace root 之内；
  - 项目列表：越界的历史项目从 `/api/projects` 隐藏；
  - 查看/打开：越界项目及其会话的 detail / message / switch / 创建一律拒绝（close / archive 保留，便于清理）；
  - browse-dirs：浏览起点与显式 root 均收敛到 workspace root 之内。
- 远端节点：主控仅约束本机；`sebas-node` 各自配置自己的 workspace root，注册远端项目时由该节点在 `CheckPath` 里判定是否越界（协议应答携带判定结果）。
- **BREAKING**：删除 `[watchdog.webui] allowed_roots` 多根白名单及其全部装配（`webui_allowed_roots`、`within_allowed_roots`、`build_router_with_allowed_roots`），由 workspace root 单根接管；旧配置键被忽略。

## Capabilities

### New Capabilities

- `workspace-root`: 工作区根目录的解析顺序（env > 配置 > cwd 回退 + 启动告警）、每机器独立生效、canonicalize + 逐分量前缀的 fail-closed 范围判定语义。

### Modified Capabilities

- `webui`: 删除 allowed_roots 白名单需求；browse-dirs 与项目注册改由 workspace root 约束；新增列表隐藏与查看/打开拒绝的需求。
- `project-session-actions`: 目录选择器/手动输入注册项目必须落在 workspace root 之内（推翻「manual path outside the tree root still registers」）。
- `execution-node`: 节点以自己的 workspace root 参与 `CheckPath` 判定（越界如实上报）。
- `node-session-channel`: `PathChecked` 应答携带 `within_workspace` 判定字段。

## Impact

- 代码：`src/config.rs`、`src/webui_cmd.rs`、`src/run.rs`（装配）；`sebas-webui/src/{server,api,fs,projects}.rs`（校验面）；`sebas-node/src/{config,session}.rs`（节点侧）；`sebas-node-link/src/lib.rs`（协议）。
- API：`/api/projects`、`/api/projects/{id}/branch`、`/api/fs/browse-dirs`、`/api/sessions*` 的越界行为变化。
- 兼容：允许旧配置含 `allowed_roots`（字段忽略，不炸解析）；老节点未上报 `within_workspace` 时视为通过（升级节点后获得完整约束）。
- 测试与沙箱：`tasks.py` 沙箱配置、e2e/验收套件需显式配置 workspace root。

## Non-goals

- 不约束会话 spawn 之外的 agent 子进程文件访问（ACP 子进程仍按自身权限运行）。
- 不做 core / dispatch 层的二次校验（本变更只在 webui API 面与节点 CheckPath 面执法）。
- 不迁移/清洗越界的历史项目数据（只隐藏与拒绝，不删除）。
- 不提供多根白名单（单根；多目录请用符号链接或调整根目录）。
