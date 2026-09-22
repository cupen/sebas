# Tasks — add-system-dir-denylist

## 1. 名单判定原语（sebas-webui/src/fs.rs）

- [x] 1.1 实现 `is_system_dir(path: &Path) -> bool` 与名单构建：Unix 组（`/`、`/bin`、`/sbin`、`/boot`、`/dev`、`/etc`、`/lib`、`/lib32`、`/lib64`、`/libx32`、`/proc`、`/sys`、`/usr`、`/var`、`/run`、`/root`、`/home`、`/tmp`）、Windows 组（`C:\Windows`、`C:\Program Files`、`C:\Program Files (x86)`、`C:\ProgramData`、`C:\Users`、`System Volume Information`、`$Recycle.Bin`）+ 盘符根模式判定；条目首次使用时 canonicalize 存解析形（失败保留字面形），候选解析后比较，Windows 小写比较。单测：命中/子树放行/不存在路径 fail 路径（`cargo test -p sebas-webui fs::tests`）。
- [x] 1.2 macOS 变体与 Windows 大小写单测：`/tmp` 解析形命中、Windows 用大小写变体路径断言命中（`#[cfg(windows)]` 门控）；`cargo test` 全绿。
- [x] 1.3 `browse_dirs` 条目过滤：产出 entries 时 join 后经 `is_system_dir` 滤除；单测覆盖「名单子目录不出现、其余条目与 round-trip 不变」（复用 `dir_with_sub` 形态 + 名单形子目录；root 用 tempdir，名单判定对 tempdir 恒 false 不误伤）。

## 2. 注册执法链（sebas-webui/src/api.rs）

- [x] 2.1 `projects_add` 校验链 containment 之后插入名单判定：命中返回 400「系统目录不可注册为项目: <入参>」，解析失败不在此拒；单测：root 内造 `/usr` 形不可行——用测试专用名单注入或对 tempdir 候选断言 miss + 对 `/`（unix）断言 hit，400 文案点名入参且不含服务端解析形。
- [x] 2.2 核对 `projects::add` 全部调用方（api.rs 降级路径、tests、其他 crate）确认名单执法无旁路；grep 记录结论进 PR 描述。
- [x] 2.3 server.rs 集成测试：workspace root=`/`（unix）时 `POST /api/projects` path=`/usr` → 400、path=根内普通目录 → 201；containment 越界（root 外系统目录）仍返回越界文案（判定先行次序）。

## 3. 启动告警装配（src/run.rs、src/webui_cmd.rs）

- [x] 3.1 两处装配点：workspace root 解析后调 `sebas_webui::fs::is_system_dir`，命中 `tracing::warn!`（点名根 + 指向 `[workspace] root`），不阻断；单测直测告警判定分支（装配点函数级或抽小函数），unix 用 `/` 断言。

## 4. 验证与收尾

- [x] 4.1 全量 `cargo test`（workspace）+ `cargo clippy` 绿；前端不动故不跑前端套件，确认 `frontend/dist` 不受影响。
- [x] 4.2 沙箱 e2e 抽查（`invoke testsuite-e2e` 或按 AGENTS.md 沙箱菜谱）：注册 `/usr` 拒、注册沙箱内目录成功、browse-dirs 不再返回名单条目（root 设为沙箱根的父级时）；如实记录沙箱覆盖面与局限。
- [x] 4.3 对照 specs 增量逐场景核对（webui 三需求 8 场景 + project-session-actions 新增 2 场景），结论写进验收记录。

## 验收记录（4.3，2026-09-16）

- **webui 3 需求 8 场景**：browse-dirs 隐藏（fs 单测 symlink 形 + server 集成 root=/ 顶层 + 沙箱实证 mnt/opt/srv 不误伤）；注册执法（server 集成 `/usr`→400 点名入参、tempdir 子树→201、containment 先行次序 + 沙箱实证 `/tmp/../etc`→400）；启动告警（config 单测 + 两装配点 + 沙箱 root=/ 日志实证）。
- **project-session-actions 3 新增场景**：手输系统目录拒绝与树选同走唯一执法点 `projects_add`（前端零改动，400 文案经既有 addError 回显）；树不供名单节点 = browse-dirs 过滤同一端点；越界拒绝语义为 add-workspace-root 既有行为，本 change 未触碰。
- **Windows 局限（如实）**：大小写不敏感 + 盘符根模式判定已实现（dunce 普通形 + 小写比较 + `is_drive_root` 模式）并有 `#[cfg(windows)]` 单测，但本机 Linux 无法执行，属代码在案未实跑。
- **2.2 调用方 grep 结论**：本地注册唯一 wire 入口 `sebas-webui/src/api.rs projects_add`（执法点）；`projects::add` 仅在 api.rs:1649 降级持久化路径（执法之后，无旁路）；`add_on`（远端节点注册）按 Non-goals 不拦；测试调用方（server.rs / projects.rs tests）不构成旁路。
- **门禁**：cargo test 597 绿（+1 config）、cargo test -p sebas-webui 290 绿（+3 server 集成）、clippy 0 error（2 条既有 warning 经 stash 对照确认非本 change 引入）、沙箱 e2e 抽查全过、前端零改动未跑前端套件（tasks 4.1 口径）。
