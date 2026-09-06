# Add WebUI 项目目录选择器：服务端 work dir 起始 + 展开往返修复

## Why

Add Project 弹窗的目录树存在两个可用性问题：

1. **展开即报错（bug）**：在 Windows 上，`GET /api/fs/browse-dirs` 返回的 `path` 携带
   `canonicalize()` 的 verbatim 前缀（如 `\\?\D:\`，尾部为反斜杠）；前端只用
   `replace(/\/$/, '')` 剥尾部**正斜杠**再拼 `/`，得到 `\\?\D:\/bin` 这类混合分隔符路径。
   verbatim 路径不做分隔符归一化，回传后端 `canonicalize` 失败 → 400「路径不存在或无法访问」。
   已在沙箱实测复现（`\\?\D:\bin` 与相对路径 `bin` 均 200，仅混合形式 400）。
2. **起始位置不合预期**：目录树默认从文件系统根（Windows 上退化为进程所在盘符根）开始浏览，
   而操作员的项目实际都在服务端配置的 work dir（`[acp.agents.<kind>] work_dir`）之下。
   主 spec（webui / project-session-actions）本就要求 browse-dirs
   "scoped to a server-configured work root"，当前实现与 spec 存在漂移。

## What Changes

- **服务端**：`GET /api/fs/browse-dirs` 的默认 root 从 `/` 改为服务端配置的 work dir
  （默认 agent kind 的 `work_dir`，未配置时回退进程 cwd）；`root` 参数仍可显式覆盖。
- **服务端**：返回的 `path` 剥离 Windows verbatim 前缀（`\\?\` / `\\?\UNC\`），
  并容忍入参中的 `/` 与 `\` 混用——保证「响应里回显的路径可以原样回传」的往返契约。
- **前端**：`sebas-folder-picker` 路径拼接剥离尾部 `[\\/]`，不再产出混合分隔符路径；
  展开失败时在树内显示错误而非静默失败；树顶部展示当前根路径，操作员可感知起始位置。
- 手动路径输入不受树根限制（后端注册校验不变）。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

- `webui`：`GET /api/fs/browse-dirs` 语义变更——默认 root 为服务端配置的 work dir；
  新增路径往返契约（响应 `path` 可原样作为后续请求的 `path` 回传，分隔符混用被容忍）。
- `project-session-actions`：「Add project via directory picker」需求变更——
  目录树从服务端 work dir 开始展开，节点展开在 Windows 上可靠工作（不再报错）。

## Impact

- `sebas-webui/src/fs.rs`（root 解析 / 路径归一 / verbatim 简化）、`sebas-webui/src/api.rs`、
  `sebas-webui/src/server.rs`（注入 work root）。
- 接线：`src/run.rs`（core --webui 内嵌形态）与 `src/webui_cmd.rs`（独立 webui 进程），
  两处均持有 `cfg`，沿用 agent_kinds 的注入模式。
- 前端 `sebas-webui/frontend/src/components/folder-picker.ts`。
- 兼容性：`root` 显式传参的调用方行为不变；未传 `root` 的现有调用方（即 folder-picker）
  起始位置从盘符根变为 work dir——即本变更的目的。

## Non-goals

- 不做向上导航（树仍是纯下降结构）；跨盘/树外浏览通过手动路径输入满足。
- 不新增独立 endpoint（如 `/api/fs/workdir`）——省略 `root` 即服务端默认。
- 不改项目注册校验与 `projects.add` 的路径语义。
- 不处理符号链接循环、挂载点等极端文件系统形态（`canonicalize` 既有语义照旧）。
