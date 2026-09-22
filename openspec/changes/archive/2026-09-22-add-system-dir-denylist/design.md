# Design — add-system-dir-denylist

## Context

注册校验链（`sebas-webui/src/api.rs` `projects_add`）现状：workspace root 包含判定（`within_workspace_root`，canonicalize + 逐分量前缀比较）→ 存在 → 是目录 → `canonicalize_plain` → 查重。browse-dirs（`sebas-webui/src/fs.rs`）产出子目录条目时无任何过滤。路径语义原语的家是 `fs.rs`（`safe_path` / `within_workspace_root` / `canonicalize_plain`）。workspace root 解析在主 crate `src/config.rs` `resolve_workspace_root`（env > config > cwd 回退），装配点为 `src/run.rs` 与 `src/webui_cmd.rs`——既有约定：解析函数高频调用不打日志，告警由装配点打。

拷问结论（proposal.md Why / Non-goals）：变量展开不做；名单内置固定；远端节点协议不动。与未归档 change `add-workspace-root` 的交叠处理见 Risks。

## Goals / Non-Goals

**Goals:**

- 一个两 crate 共用的名单判定原语：候选经真实路径解析后与名单**精确匹配**
- 注册校验链插入名单判定（containment 之后）；browse-dirs 条目输出同步过滤
- workspace root 解析为系统目录时装配点打启动 warn

**Non-Goals:**

- 名单的 config 扩展/裁剪、远端节点协议扩展、变量展开、前端即时校验、存量数据迁移（proposal Non-goals）

## Decisions

**D1 原语放 `sebas-webui/src/fs.rs`，名单按 `cfg` 平台分组。** `is_system_dir(path: &Path) -> bool` 与 `within_workspace_root` 同居——路径语义原语的既有家，主 crate（`run.rs` / `webui_cmd.rs` 的告警装配）经 `sebas_webui::fs` 引用（装配点本就依赖该 crate）。备选：独立 `denylist.rs`——名单只有一个函数，不值得新模块；放 `projects.rs`——注册表模块不该拥有路径语义。

**D2 匹配 = 两侧解析 + 精确相等。** 候选 `std::fs::canonicalize` 后比对；名单条目在**首次构建**时逐条 canonicalize（17 条 syscall，进程一次），解析成功的条目存解析形（吸收 macOS `/tmp` → `/private/tmp`、`/var` → `/private/var` 变体），失败保留字面形（不存在的系统目录候选也命中不了，无损失）。Windows：两侧 `dunce::simplified` 后按小写字符串比较（`std` 的组件比较不做 case fold）；盘符根不进名单，用模式判定——候选解析形仅剩盘符根分量（`C:\`）即命中。精确相等（非前缀）：子树放行是 spec 明文（`/home`、`C:\Users` 的子目录是主项目位置）。

**D3 执法点只在 API 校验链，插在 containment 之后。** 顺序：containment（既有文案不变）→ 名单判定（新 400）→ exists → is_dir → `canonicalize_plain` → 查重。名单判定用 `canonicalize` 结果，解析失败不在此拒（交给既有 exists 分支保持文案）。`projects::add`（纯注册表逻辑）不判名单——与 containment 同一分布：校验是 API 层职责，注册表模块不做 fs 语义。tasks 里核对 `projects::add` 全部调用方确认无旁路。备选：下沉进 `projects::add`——会让远端注册（同一函数）误用本机语义。

**D4 browse_dirs 过滤与注册层同一函数。** 产出 entries 时对每个子目录 join 后调 `is_system_dir`（每条目一次 canonicalize；`has_subdirs` 已对每条目做过一次 `read_dir`，量级不恶化）。漏判的兜底是注册执法层，两层语义同源不会互相矛盾。备选：join 后字符串前缀比对（零 syscall）——漏 symlink 别名，与注册层语义分叉，否。

**D5 启动告警在装配点。** `run.rs` 与 `webui_cmd.rs` 拿到解析后的 workspace root 后调 `is_system_dir`，命中则 `tracing::warn!`（点名根、提示收紧 `[workspace] root`），不阻断启动。沿用「解析函数不打日志」既有约定。

**D6 被否备选**（拷问记录）：子树连坐（堵死主项目位置）；config 扩展名单（防呆底线不该可绕，YAGNI）；`$VAR`/`%VAR%` 变量展开（用户拍板现状已够）；手输框前端即时校验（双份实现，后端 400 已回显弹窗）；远端节点协议加字段（新旧混布兼容成本，节点 workspace root 围栏已兜底）。

## Risks / Trade-offs

- [workspace root=`/` 时所有顶层目录被拦，操作员以为功能损坏] → 400 文案点名「系统目录」；启动 warn 明示根过宽并指向 `[workspace] root`。
- [browse 大根（`/` 下数千条目）时每条目一次 canonicalize 变慢] → 一次性列目录的固有成本内；若实测显著，降级为不解析比对（体验层可容忍漏判），注册层不变。
- [与未归档 `add-workspace-root` 的增量交叠] → 本 change 的 `project-session-actions` MODIFIED 已基于其合并后文本；`webui` 侧全部用 ADDED（新需求头），归档顺序无关。
- [名单条目平台变体遗漏（如未预见的别名目录）] → 判定输入是解析形，条目构建同样解析，机制上变体自动吸收；个案补名单条目即可（内置名单，改代码发布）。

## Migration Plan

无持久化迁移：存量名单内项目保留原样（列表、会话照常），仅新注册被拦。回滚 = revert 代码，注册表文件格式不变。

## Open Questions

（无——拷问已收束，剩余次要细节按 proposal Non-goals 记为假设。）
