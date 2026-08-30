# 任务：workspace 成员 crate 统一 sebas- 前缀

前置说明：在 `feat/*` 分支上实施（仓库 Git 工作流约定）；映射表与决策见 design.md D3/D4。

## 1. 清单与目录重命名

- [x] 1.1 根 `Cargo.toml`：更新 workspace members 与 `[dependencies]` 中 5 个成员的名称（`sebas-router = { path = "sebas-router" }` 等），并 `git mv` 五个目录 `router/ feishu/ acp-claude/ gateway/ webui/` → `sebas-*`；验证 `ls` 下目录名与映射表一致、`git status` 显示 rename
- [x] 1.2 五个成员各自 `Cargo.toml` 的 `[package] name` 改为 `sebas-*`，并更新成员间路径依赖（`sebas-router` 内对 `sebas-feishu`/`sebas-acp-claude`、`sebas-webui` 内对三个成员）；验证 `cargo metadata --no-deps` 列出新包名且无解析错误
- [x] 1.3 xtask 路径常量：`xtask/src/main.rs` 中 `gateway/src/models.rs` 相关定位改为 `sebas-gateway/src/models.rs`（含文档注释）；验证 `cargo run -p xtask -- --help` 正常输出

## 2. 代码导入与编译修复

- [x] 2.1 全仓替换 `use` 导入与路径引用：`router::`→`sebas_router::`、`feishu::`→`sebas_feishu::`、`acp_claude::`→`sebas_acp_claude::`、`gateway::`→`sebas_gateway::`、`webui::`→`sebas_webui::`（含根包 `src/`、各成员、`tests/`、`examples/`）；注意逐条核对避免误改同名本地模块或字符串字面量；验证 `cargo check --workspace` 通过
- [x] 2.2 全量测试与告警清理：`cargo test --workspace` 全绿；`grep -rn "\b(router|feishu|acp_claude|gateway|webui)::" --include="*.rs"` 排查残留（应仅剩成员内部 `crate::`/本地模块引用）

## 3. 构建环境

- [x] 3.1 `Dockerfile`：COPY 路径改为 `COPY sebas-router ./sebas-router` 等 5 处；验证本地 `docker build .` 成功（或至少 `docker build` 前的 COPY 层路径核对 + CI docker 流程通过）
- [x] 3.2 CI 确认：核对 `.github/workflows/ci.yml`、`docker.yml`、`release.yml` 无成员包名引用（当前为 `--bin sebas`/`--bin fake-claude`/`cargo test --workspace`，预期零改动）；验证推送分支后 CI 全绿

## 4. 文档与规划工件一致性

- [x] 4.1 README 与 `docs/` 中明确指 crate 的引用更新为 `sebas-*`（保留 CLI 子命令、配置段、capability 名原样）；验证 `grep -rn "crate (router|feishu|acp-claude|gateway|webui)\b" README.md docs/` 无遗漏
- [x] 4.2 活跃 OpenSpec 变更工件按 design.md D5 逐条核对：`add-core-session-channel`、`add-project-workbench`、`redesign-webui-console` 中指 crate 的引用改为 `sebas-*`，capability 名（如 `specs/webui/`）不动；验证三个变更目录内 grep 复核无 crate 名残留
- [x] 4.3 项目上下文同步：检查 `openspec/config.yaml`、`AGENTS.md`、`CLAUDE.md` 如有成员 crate 名提及则更新；验证相关文件 grep 干净

## 5. 收尾验证

- [x] 5.1 终验：`cargo build --bin sebas`、`cargo test --workspace`、`cargo run -p xtask -- --help` 全部通过；脚本冒烟 `scripts/e2e_gateway.sh` 通过；验证无任何运行时行为差异（二进制名、CLI 子命令、配置段名不变）
- [x] 5.2 按 Conventional Commits 提交（建议 `refactor(workspace): rename member crates with sebas- prefix`），按仓库约定 rebase 到 main 后合并；验证合并后 main 上 CI 绿

## 备注（验证结果与遗留项）

- **2.2 / 5.1 通过标准 = 与 main 基线等价**（用户确认合并时认可）：全量测试除 7 个失败外全绿，7 个失败均已逐一归因、与改名无关——
  - `config_env_test.rs::env_overrides_toml...`：main 上的真实遗留 bug（970c57f 放宽 validate 未同步测试）→ bead `sebas-umh`；
  - `e2e_gateway.sh` 失败：脚本用旧 provider 字段 `base_url=`，main 上即坏 → bead `sebas-3n4`（schema 兼容的 `e2e_gateway_admin.sh` 全流程 PASS，验证运行时行为无损）；
  - 其余 5 个（router provider_state×4、gateway server_smoke×1、config_test validate_runtime×1）：本沙箱 `~/.sebas`、`~/.config` 只读（EROFS）所致，可写 HOME 下全部 PASS，main 上同样失败。
- **3.2 / 5.2 的「push 后 CI 绿」**：未获 push 授权，本地合并完成；push 后验证 → bead `sebas-ci9`。Dockerfile 未做本地镜像构建（docker daemon 不可用），已核对 COPY 路径与 `--locked` 构建一致性。

## 6. 修订：sebas-acp-claude → sebas-acp + claude 子模块（2026-08-30，D7）

- [x] 6.1 `git mv sebas-acp-claude sebas-acp`；模块收进 `src/claude/`（mod.rs 持声明与根重导出，lib.rs 只留 `pub mod claude;`），内部 `crate::X` → `crate::claude::X`；验证 `cargo check --workspace` 通过
- [x] 6.2 清单同步：根 Cargo.toml members/deps、sebas-router/sebas-webui 依赖、Dockerfile COPY；外部导入 `sebas_acp_claude::` → `sebas_acp::claude::`（59 处，含 crate 自身 tests、gateway proto.rs 文档注释）
- [x] 6.3 文档与工件：README（图/表/目录树）、design-history 修订注；本变更 proposal/design/tasks 增补修订记录；验证全仓 grep 无 `sebas-acp-claude`/`sebas_acp_claude` 残留（Cargo.lock 除外）
