# Tasks — add-system-dir-denylist

## 1. 名单判定原语（sebas-webui/src/fs.rs）

- [ ] 1.1 实现 `is_system_dir(path: &Path) -> bool` 与名单构建：Unix 组（`/`、`/bin`、`/sbin`、`/boot`、`/dev`、`/etc`、`/lib`、`/lib32`、`/lib64`、`/libx32`、`/proc`、`/sys`、`/usr`、`/var`、`/run`、`/root`、`/home`、`/tmp`）、Windows 组（`C:\Windows`、`C:\Program Files`、`C:\Program Files (x86)`、`C:\ProgramData`、`C:\Users`、`System Volume Information`、`$Recycle.Bin`）+ 盘符根模式判定；条目首次使用时 canonicalize 存解析形（失败保留字面形），候选解析后比较，Windows 小写比较。单测：命中/子树放行/不存在路径 fail 路径（`cargo test -p sebas-webui fs::tests`）。
- [ ] 1.2 macOS 变体与 Windows 大小写单测：`/tmp` 解析形命中、Windows 用大小写变体路径断言命中（`#[cfg(windows)]` 门控）；`cargo test` 全绿。
- [ ] 1.3 `browse_dirs` 条目过滤：产出 entries 时 join 后经 `is_system_dir` 滤除；单测覆盖「名单子目录不出现、其余条目与 round-trip 不变」（复用 `dir_with_sub` 形态 + 名单形子目录；root 用 tempdir，名单判定对 tempdir 恒 false 不误伤）。

## 2. 注册执法链（sebas-webui/src/api.rs）

- [ ] 2.1 `projects_add` 校验链 containment 之后插入名单判定：命中返回 400「系统目录不可注册为项目: <入参>」，解析失败不在此拒；单测：root 内造 `/usr` 形不可行——用测试专用名单注入或对 tempdir 候选断言 miss + 对 `/`（unix）断言 hit，400 文案点名入参且不含服务端解析形。
- [ ] 2.2 核对 `projects::add` 全部调用方（api.rs 降级路径、tests、其他 crate）确认名单执法无旁路；grep 记录结论进 PR 描述。
- [ ] 2.3 server.rs 集成测试：workspace root=`/`（unix）时 `POST /api/projects` path=`/usr` → 400、path=根内普通目录 → 201；containment 越界（root 外系统目录）仍返回越界文案（判定先行次序）。

## 3. 启动告警装配（src/run.rs、src/webui_cmd.rs）

- [ ] 3.1 两处装配点：workspace root 解析后调 `sebas_webui::fs::is_system_dir`，命中 `tracing::warn!`（点名根 + 指向 `[workspace] root`），不阻断；单测直测告警判定分支（装配点函数级或抽小函数），unix 用 `/` 断言。

## 4. 验证与收尾

- [ ] 4.1 全量 `cargo test`（workspace）+ `cargo clippy` 绿；前端不动故不跑前端套件，确认 `frontend/dist` 不受影响。
- [ ] 4.2 沙箱 e2e 抽查（`invoke testsuite-e2e` 或按 AGENTS.md 沙箱菜谱）：注册 `/usr` 拒、注册沙箱内目录成功、browse-dirs 不再返回名单条目（root 设为沙箱根的父级时）；如实记录沙箱覆盖面与局限。
- [ ] 4.3 对照 specs 增量逐场景核对（webui 三需求 8 场景 + project-session-actions 新增 2 场景），结论写进验收记录。
