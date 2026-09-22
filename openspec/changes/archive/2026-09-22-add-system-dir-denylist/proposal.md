# Add 系统目录屏蔽名单：项目注册与目录浏览的最后防线

## Why

项目注册的合法性校验目前只有 workspace root 包含判定 + 存在性/目录判定（add-workspace-root）。当 workspace root 配置得很宽（如 `/`）或回退到进程 cwd 时，`/usr`、`/etc`、盘符根这类系统目录都能注册成项目——目录树里也会原样列出它们。需要一个不依赖 root 配置纪律的内置防呆底线。

## What Changes

- **后端注册拦截（执法层）**：`POST /api/projects`（本机）校验链在 workspace root 判定之后增加系统目录名单判定——候选路径经 canonicalize 后与名单**精确匹配**（只拦名单目录本身，子树放行，`/home`、`C:\Users` 的子目录仍是合法项目位置）即 400 拒绝，文案点名路径（不回显服务端解析形）。
- **名单原语**：内置固定名单（Unix：`/`、`/bin`、`/sbin`、`/boot`、`/dev`、`/etc`、`/lib*`、`/proc`、`/sys`、`/usr`、`/var`、`/run`、`/root`、`/home`、`/tmp`；不拦 `/opt`、`/srv`、`/mnt`、`/media`。Windows：盘符根按模式判定、`C:\Windows`、`C:\Program Files`、`C:\Program Files (x86)`、`C:\ProgramData`、`C:\Users`、`System Volume Information`、`$Recycle.Bin`，大小写不敏感）。名单条目与候选两侧都先 canonicalize 再比对（吸收 macOS `/tmp` → `/private/tmp` 变体，防 `..`、别名与 symlink 绕过）。
- **browse-dirs 树内隐藏（体验粗滤）**：`GET /api/fs/browse-dirs` 不返回命中名单的子目录条目；漏过由注册执法层兜底。
- **启动告警**：workspace root 解析为系统目录时打启动 warn（不阻断）。
- 存量已注册的名单内项目不迁移不隐藏，仅注册新项目时拦。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`：`GET /api/fs/browse-dirs` 条目列表新增系统目录过滤语义；`POST /api/projects` 注册拒绝场景新增系统目录分支。
- `project-session-actions`：「Add project via directory picker」注册校验语义变更——系统目录（含经树选择与手输两条入口）不可注册，400 点名路径。

## Impact

- `sebas-webui/src/fs.rs` 或新模块（名单与判定原语 + browse_dirs 过滤）、`sebas-webui/src/api.rs`（projects_add 校验链）、`src/config.rs` / `sebas-webui/src/server.rs`（workspace root 启动告警装配点）。
- 前端无改动（现有 addError 回显机制直接呈现 400 文案）。
- 兼容性：root 配置纪律正常时行为不变；仅名单精确命中被新增拒绝。

## Non-goals

- 不做变量展开（`$VAR`/`%VAR%`）——`[workspace] root` + `SEBAS_WORKSPACE_ROOT` + `~` 展开已满足配置复用诉求（拷问中拍板）。
- 不做远端节点协议扩展（节点侧不拦名单，其 workspace root 围栏照旧兜底）。
- 不提供名单的 config 扩展/裁剪（内置固定，YAGNI）。
- 不迁移、不隐藏存量名单内项目。
- 不对手输路径做前端即时校验（后端 400 报错回显弹窗已够）。
