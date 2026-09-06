# Tasks — add-webui-picker-workdir-start

## 1. 服务端：统一安全入口 `fs::safe_path`（路径往返 + 安全加固）

- [x] 1.1 合并 `resolve_root` + `resolve_within_root` 为单点入口 `fs::safe_path(path, explicit_root, server_default_root)`：root 取值「显式 > 服务端默认 > 硬错误」，不再自造 `/` 默认；root `canonicalize` 失败即报错，删除 `unwrap_or_else` 静默回退。验证：单测——root 缺失且无默认时报错、root 不存在时 400。
- [x] 1.2 `safe_path` 内实现 Windows 入参归一：join/canonicalize 前把 `/` 归一为 `\`（`cfg(windows)`），绝对路径 join 替换基底的既有语义保持不变。验证：单测 `\\?\D:\/bin` 形态（混合分隔符 + verbatim 前缀）解析成功。
- [x] 1.3 `safe_path` 返回 (规范化目标, 回显简化形式)：Cargo.toml 增加直接依赖 `dunce`，回显经 `dunce::simplified` 剥 verbatim 前缀。验证：单测断言常规路径回显不含 `\\?\`。
- [x] 1.4 `browse_dirs` handler 改为只调用 `safe_path`，`params.root` 与注入的 `state.work_root` 作为其入参。验证：`cargo check` + 既有 browse-dirs 用例绿。
- [x] 1.5 安全加固：`不是目录` 类错误不再回显服务端解析路径（改回显入参或固定文案），完整路径仅进 tracing 日志。验证：单测断言错误体不含 canonicalize 后路径。
- [x] 1.6 补齐安全测试矩阵：`\\?\D:\bin` 200、混合分隔符 200、相对路径 200、`..` 逃逸 400、绝对路径越界 400、symlink 指向树外 400、root 显式传参语义不变。验证：`cargo test -p sebas-webui fs::`。

## 2. 服务端：work root 注入（server.rs / api.rs / 接线）

- [x] 2.1 webui 服务端状态增加 `work_root: Option<PathBuf>`，`run_with_admin_adapter_and_auth` 增参并贯通到 `api::browse_dirs`。验证：`cargo check`。
- [x] 2.2 两处接线：`src/run.rs`（core --webui 内嵌）与 `src/webui_cmd.rs`（独立 webui 进程）传入 `cfg.acp.work_dir_for(cfg.acp.default_kind())`（expand_tilde 已由 config 加载完成），`None` 时由 safe_path 回退 `std::env::current_dir()`。测试调用点传 `None`。验证：`cargo test`（编译期贯通 + 既有测试绿）。

## 3. 前端：folder-picker 与 API 封装

- [x] 3.1 `client.ts` 新增统一查询串构造 `withQuery(path, params)`（`URLSearchParams`），`fsBrowse` / `fsBrowseDirs` 改走它，替代手搓 `encodeURIComponent` 模板。验证：`pnpm -C sebas-webui/frontend test`（client 用例，含 `\`、`+`、中文目录名编码断言）。
- [x] 3.2 路径拼接防御：`folder-picker.ts` 两处（loadRootDirs / loadSubdirs）剥尾改为 `path.replace(/[\\/]+$/, '')` 后拼 `/`。验证：folder-picker 相关用例。
- [x] 3.3 起始目录：picker 打开时不传 `root`（省略即服务端默认 work dir）；树顶以小字展示首屏响应回显的 `path`。验证：浏览器打开 Add Project 弹窗，首层即沙箱 work dir 内容且根路径可见。
- [x] 3.4 展开失败内联报错：`loadSubdirs` catch 设置 error state，节点下方显示错误文案，再次点击重试。验证：临时断开后端或请求非法路径时树内出现错误行、恢复后可重试成功。

## 4. 联调验证（沙箱端到端）

- [x] 4.1 按 AGENTS.md 沙箱菜谱起 webui 沙箱（`bash scripts/test_webui_sandbox.sh`）：`GET /api/fs/browse-dirs?path=`（不带 root）返回 work dir 列表且回显 `path` 无 `\\?\`；回显 path 拼子目录回传 200；含 `+`/空格/中文的目录名往返正常。验证：curl 断言。
- [x] 4.2 GUI 往返：浏览器打开 Add Project 弹窗，从 work dir 起始逐层展开到目标目录、选中并注册项目；Windows 上展开全程无 400。验证：截图（gui-test-screenshots/）+ DOM 断言。（注：会话创建到该项目在本沙箱不可验——webui-only 沙箱无 core（`core not connected: socket absent`，AGENTS.md 已知限制）；会话层由既有 core_flow e2e 覆盖，与本 change 的 picker 范围无关。）
- [x] 4.3 显式 root 覆盖与逃逸拒绝回归：带 `root` 参数的请求行为不变，越界路径仍 400、错误体不含服务端解析路径。验证：curl 断言。

## 5. 收尾

- [x] 5.1 全量质量门：`cargo test`、`pnpm -C sebas-webui/frontend test`、`openspec validate add-webui-picker-workdir-start --strict`。验证：全绿。（注：本 change 触及的测试全绿——fs 14/14、sebas-webui 其余、workspace 各 crate、前端 131/131；`cargo test` 尚有 2 个 agent_kinds 预存失败（`present_binary_reports_reachable` / `opencode_acp_probe_is_compatible`，Windows 下 PATHEXT/.exe 探测缺失），已用 `git stash` 在 HEAD 基线复现证明与本 change 无关，另行立 beads 跟踪。）
