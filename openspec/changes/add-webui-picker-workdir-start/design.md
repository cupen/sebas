# Design — add-webui-picker-workdir-start

## Context

- `GET /api/fs/browse-dirs`（`sebas-webui/src/fs.rs`）当前默认 root 硬编码 `/`；Windows 上
  `canonicalize("/")` 解析为**进程所在盘符根**且返回 verbatim 形式（`\\?\D:\`）。
- 前端 `folder-picker.ts` 用 `resp.path.replace(/\/$/, '')` 剥尾后拼 `/` 生成子路径——
  Windows 回显尾部是 `\`，剥不掉，得到 `\\?\D:\/bin`；verbatim 路径不做分隔符归一化，
  回传后 `canonicalize` 失败 → 400（沙箱已复现：`\\?\D:\bin`、`bin` 均 200，仅混合形式 400）。
- 服务端"work dir"语义已有约定：会话工作目录 = `project_dir`，未绑定时回退
  `cfg.acp.work_dir_for(default_kind)`（`src/dispatch.rs:136`）。webui 两处接线
  （`src/run.rs` 内嵌、`src/webui_cmd.rs` 独立进程）均持有 `cfg` 并已有 config→webui
  注入先例（`agent_kinds`）。
- `dunce` 已在 Cargo.lock（传递依赖），升为直接依赖零成本。

## Goals / Non-Goals

**Goals:**
- browse-dirs 路径往返契约成立：响应回显的 `path` 原样回传（含拼子目录）必须成功。
- 未显式传 `root` 时，浏览起点 = 服务端 work dir（默认 agent kind 的 `work_dir`，
  未配置回退进程 cwd），与既有会话 work dir 语义一致。
- 前端展开失败有内联反馈，不再静默；树顶展示当前根路径。

**Non-Goals:**
- 不做向上导航；不新增 endpoint；不改 `projects.add` 校验（见 proposal Non-goals）。

## Decisions

### D1 往返修复：后端入参归一为根本解，回显简化为可读性，前端拼接为防御

三层修复收敛到**一个后端统一入口**（见 D2）内实现，职责如下：

1. **入参归一（根本解，Windows only）**：`safe_path` 在 join/canonicalize 前
   把入参中的 `/` 归一为 `\`（`cfg(windows)`；Unix 反斜杠是合法文件名，不可做）。
   归一后即使回显路径保留 verbatim 前缀（见下），`\\?\D:\/bin` 也能正确解析。
2. **回显简化（可读性 + 兼容现网客户端）**：返回 `path` 前剥离 verbatim 前缀
   （`\\?\C:\x` → `C:\x`，`\\?\UNC\s\p` → `\\s\p`）。用 `dunce::simplified`
   （已在依赖闭包）而非手写 strip：它对「剥离会破坏语义」的形态（UNC 边角、
   >260 字符长路径）保守保留 verbatim——正确性由第 1 层兜底，不靠回显形态。
3. **前端拼接（防御）**：`folder-picker.ts` 剥尾改为 `path.replace(/[\\/]+$/, '')`
   再拼 `/`；分隔符统一用 `/`，交由第 1 层归一，前端不做平台分支。

备选否决：前端自建逻辑路径（不复用回显）——把根路径知识复制到客户端、改动面大；
仅改前端剥尾——修不了「`\\?\` 前缀 + 平台分隔符」的根本形态问题。

单测锚点：`\\?\D:\bin` 200；`\\?\D:\/bin`（归一后）200；相对路径 200；
`..` 逃逸仍 400；回显不含 `\\?\`（常规长度路径）。

### D2 统一安全入口 `fs::safe_path`：路径语义单点封装

需求来源是操作员要求的"用封装好的 safe_url 确保 path 语义正常"。该名字的封装在
本工作区（全部 crate、前端、specs、git 历史、依赖）均不存在，故在本 change 中建立，
命名按实际语义拆为两处（fs 路径语义不是 URL，不沿用 safe_url 名）：

- **后端 `fs::safe_path`**：现散落的 `resolve_root` + `resolve_within_root` 合并为
  一个入口，签名为 `(path, explicit_root, server_default_root) -> Result<(PathBuf,
  String), String>`（返回规范化目标 + 供回显的简化形式），内部按序执行：
  1. root 取值：显式 `root` 参数 > 服务端注入默认 > （两者皆无时）**硬错误**，
     不再自造 `/`；
  2. root `canonicalize` 失败即报错——删除 `unwrap_or_else(|_| root.to_path_buf())`
     的静默回退（审计点 3：非规范化根上的前缀比较可被扰动）；
  3. 入参分隔符归一（D1-1，`cfg(windows)`）；
  4. `canonicalize` + 分量级 `starts_with` 边界校验（既有防线，不变）；
  5. 回显简化（D1-2，`dunce::simplified`）。
  `browse_dirs` handler 只调用它，不再手写路径逻辑。
- **前端 `withQuery`**：`client.ts` 新增统一的查询串构造（`URLSearchParams`），
  `fsBrowse` / `fsBrowseDirs` 走它，替代每个调用点手搓
  `encodeURIComponent` 模板。现网编码语义正确，这是防漂移的封装而非行为修复；
  其余 client 调用点（单段路径参数）本次不动。

备选否决：直接沿用旧两函数各自修补——路径语义分散在 resolve_root /
resolve_within_root / handler 三处，归一化、边界校验、回显简化的先后关系没有
单一事实来源，正是本轮 bug 的温床。

### D3 默认 root：注入静态配置值，沿用 agent_kinds 的参数注入模式

- 取值：`cfg.acp.work_dir_for(cfg.acp.default_kind())`（config 加载时已 expand_tilde），
  `None` → `std::env::current_dir()`。选择 default kind 而非遍历所有 kind：
  与 dispatch 的回退语义一致（"会话实际会在哪运行"）。
- 注入：`sebas_webui` 服务端状态增加 `work_root: Option<PathBuf>`（静态值），
  `run_with_admin_adapter_and_auth` 增参，`src/run.rs` 与 `src/webui_cmd.rs` 两处接线传入，
  测试传 `None`（回退 cwd）。不引入 provider trait——它是不可变的配置事实，
  不需要可替换行为（与 agent_kinds 的"探测行为注入"不同）。
- 生效：`browse_dirs` handler 把 `params.root` 与 `state.work_root` 交给
  `fs::safe_path`（D2），root 缺省的兜底顺序与"两者皆无即硬错误"由 safe_path 单点持有。
  显式 `root` 传参语义不变（覆盖默认）。

### D4 前端：省略 `root`，树顶展示回显根路径

- `sebas-folder-picker` 打开时不传 `root`（即服务端默认 work dir），
  `root` property 保留供显式场景使用。
- 树顶以一行小字展示首屏响应回显的 `path`，操作员可感知起始范围（否则"从哪开始"不可见）。

### D5 展开失败内联报错

- `loadSubdirs` 的 catch 从"静默 remove lazy"改为组件级 error state：
  树内该节点下方显示一行错误文案；再次点击该节点即重试。最小实现，不引入 toast。

### D6 安全加固：审计结论落地（root 硬失败 + 错误体回显收敛）

路径安全审计（proposal Why 之外的本轮新增）结论：

- **root 解析硬失败**：删除 `canonicalize().unwrap_or_else(原始值)` 静默回退——
  root 不存在直接 400，杜绝"非规范化根上做前缀比较"的扰动面（D2 第 2 步）。
- **错误体不再回显服务端解析路径**：现 `不是目录: {target.display()}` 把服务端
  canonicalize 后的真实布局反射给客户端；webui 一旦暴露到非 loopback 即泄露目录
  结构。改为回显**调用方原始入参**或固定文案，完整路径仅进服务端 tracing 日志。
  （`路径不存在或无法访问: {path}` 本就回显入参，保持不变。）
- **边界 posture 声明（不改行为）**：`root` 参数是调用方自选的浏览范围而非服务端
  强制边界（传 `root=C:\` 仍可全盘浏览）——真正的安全边界是 loopback 绑定 + 鉴权
  开关，fs.rs 文档如实自述。服务端 allowlist 式强制边界是独立能力，列为 non-goal；
  本次只把**默认起点**收敛到 work dir。
- 既有防线保持并加测：`..` 逃逸、绝对路径越界、symlink 指向树外均拒绝
  （canonicalize 后分量级 `starts_with`），进 D1 的单测锚点矩阵。

## Risks / Trade-offs

- [>260 字符长路径 dunce 保留 verbatim，回显可读性下降] → 往返正确性由 D1-1 入参归一
  兜底；回显形态仅影响展示。
- [work dir 未配置时回退 cwd，可能仍是不便的起点] → 与旧默认（盘符根）同级，非回归；
  cwd 即旧默认的实际根，行为只增不减。
- [browse-dirs 未传 `root` 的其它调用方（若有）起始位置改变] → 代码检索仅 folder-picker
  使用该 API；显式 `root` 调用方不受影响。
- [沙箱脚本 config 未设 `work_dir` 时] → 回退 cwd，脚本行为可预期；AGENTS.md 菜谱
  的 work_dir 显式存在，e2e 不受影响。

## Migration Plan

无数据/部署迁移。前后端同仓同二进制（前端随 `cargo build` 烘焙；operator 的
`pnpm run dev` 走 HMR），revert 即回滚。

## Open Questions

（无）
