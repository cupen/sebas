# add-agent-skills Tasks

## 1. 方言矩阵调研（实证落盘，不改代码）

- [x] 1.1 调研 gemini CLI 是否有官方 skills 消费机制（查其 docs 与 `~/.gemini/` 实际布局；区分「custom TOML commands」与「agentskills 目录」两类），结论写入本 change 的 design.md Open Questions 段更新或 D1 表注释。验证：design 或 D1 表中出现带出处（文档链接 / 本机路径）的 gemini 结论。
- [x] 1.2 调研 opencode 是否有 agentskills 同构的 skill 目录（同样实证 `~/.config/opencode/` 或等效路径），结论同 1.1 落盘。验证：同上。

## 2. Core skills 模块（`src/skills.rs`）

- [x] 2.1 实现 `scan_store(dir)`：扫仓返回 `Vec<SkillInfo>{name, description, attachments, valid, invalid_reason}`；frontmatter 解析最小化（只取 `name`/`description`，缺 `SKILL.md` 或字段缺标记 invalid 但不中断列表）。验证：`cargo test skills::scan` 覆盖正常/缺文件/缺 frontmatter 三态的单元测试通过。
- [x] 2.2 实现 `add_from_local(src, store)`：校验 `SKILL.md` 存在再整目录拷贝，非法源报错且不写盘。验证：`cargo test skills::add_local`（含非法源不写盘断言）。
- [x] 2.3 实现 `add_from_git(url, store)` 与 `add_from_npx(pkg, store)`：探测 `git`/`npx` 可用性，不在场即报错；成功路径委托外部命令后落到仓。验证：`cargo test` 用 fake 命令路径（注入 PATH 的 stub 脚本）覆盖「命令缺失报错」分支；真 clone/npx 归 6.x 手测。
- [x] 2.4 实现 `reconcile(store, backend_dir) -> SyncReport`：四分支投影（写/覆盖→记名册；旧名册-仓→删；仓外→不动），原子写回 `<backend>/.sebas-projection.json`（tmp+rename）；manifest 缺失时退化为只投影不删除。验证：`cargo test skills::reconcile` 覆盖 spec 四个 scenario（overwrite 用户手改、私产不动、删除已投影项、manifest 缺失退化）。

## 3. 方言表与 config

- [x] 3.1 在 core 内置方言表（D1）：`claude → ~/.claude/skills`、`codex → ~/.codex/skills`，其余 backend kind（含 gemini）返回 `NoPlacement`；`Convert` 枚举只留 `Identity` 变体。验证：`cargo test skills::placement`：claude/codex 命中 Identity，gemini 命中 NoPlacement。
- [x] 3.2 config 新增可选 `[skills] dir = "…"`（默认 `~/.agents/skills`），与 router/provider 同享「可选段」惯例；不新增 `enabled` / per-backend 开关。验证：`cargo test config::skills_dir_default_and_override` 两断言通过（缺省与覆盖）。

## 4. CLI：`sebas skills`

- [x] 4.1 `sebas skills list`：打印 name/description/invalid 标记；仓目录缺失按空仓处理不报错。验证：`cargo test --test cli_skills_test`（或现有 CLI 测试形态）断言空仓输出非错误。
- [x] 4.2 `sebas skills add <src>`：按 local/git/npx 三形态分派（D4），全部成功路径落盘后 `list` 可见。验证：local 形态有单元测试（2.2 复用），git/npx 在 6.x 手测各跑一次真实源记账。
- [x] 4.3 `sebas skills remove <name>`：只删仓内目录，不动任何 backend；不存在即报错。验证：`cargo test` 断言 backend 目录在 remove 后字节不变。
- [x] 4.4 `sebas skills sync`：跑全量 reconcile，输出每 backend 的 `{written, overwritten, deleted, private_ignored, no_placement}` 汇总。验证：沙箱（临时 HOME）内构造仓 + 伪 backend 目录，跑一次 sync 后断言四列表数字与文件树一致。

## 5. Webui：Settings/Skills 页 + `/api/skills*`

- [x] 5.1 后端 `GET /api/skills`（扫仓）+ `GET /api/skills/:name`（SKILL.md 原文 + attachments 列表）+ `DELETE /api/skills/:name`（只删仓）+ `POST /api/skills/sync`（reconcile + SyncReport 返回体）。验证：`cargo test -p sebas-webui` 的 handler 级测试覆盖四端点的 happy path 与「name 不存在」404。
- [x] 5.2 前端 Settings 新增 Skills 区：列表（name/desc/attachment 数/invalid 徽标）、点开展开预览（渲染 SKILL.md markdown + attachment 文件名列表）、删除按钮（确认弹窗注明「backend 目录将在下次 sync 时清理」）、刷新按钮、同步按钮（结果面板呈现 overwritten/deleted/private_ignored/noPlacement 各列表）。验证：沙箱 `invoke testsuite-webui-sandbox` 起后端后，Playwright acceptance 用例走通「list→预览→删除→刷新→sync 结果可见」旅程（记入 tests/acceptance 既有套件）。
- [x] 5.3 RBAC 与既有 webui 管理面一致：skills 端点挂到与 provider 管理面相同的权限档。验证：对照 provider 端点权限断言的现有测试形态，补一条等价断言。

## 6. 沙箱端到端 + 文档

- [x] 6.1 用 AGENTS.md 沙箱配方（`<SB>` 穿刺所有路径）跑全流程：`sebas skills add`（local 形态）→ `sebas skills list` → `sebas skills sync`（针头 backend 目录指向 `<SB>` 内伪落点）→ 删 → 再 sync 验证删除。验证：手工运行记录贴在 change 的验证说明中，四步产物树与预期一致。
- [x] 6.2 手测 `sebas skills add` 的 git 与 npx 真源各一次（真实网络允许时），记结果；不可外联环境注明「未验证，沙箱不覆盖」如实报告。验证：任务完成说明里二选一（成功日志 / 环境限制声明）。
- [x] 6.3 AGENTS.md 操作指引补一段 skills 仓与 sync 语义（含「webui 删除后需 sync 才清 backend」），并检查 AGENTS.md 沙箱 recipe 的 config 示例是否需要 `[skills]` 穿刺项。验证：`git diff AGENTS.md` 可见新增段落。

## 7. 收尾

- [x] 7.1 `openspec validate add-agent-skills --strict` 全绿；`cargo clippy`、`cargo test` 全绿。验证：三条命令输出无 error。
- [x] 7.2 按仓库 git 协议拆分提交（core skills / CLI / webui 各一笔或按粒度合并），conventional commit。验证：`git log` 可见。
