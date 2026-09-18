//! `sebas skills` CLI 集成测试（add-agent-skills 4.1–4.4）：list / add /
//! remove / sync 四个子命令，全部经进程内 `run()` 直调（与
//! webui_passwd_cli_test 同形态）。安全约定：HOME/USERPROFILE 覆写指向
//! tempdir 沙箱——投影落点（`~/.claude/skills` 等）与仓目录（config
//! `[skills] dir`）全部钉在沙箱里，绝不写真实 HOME。

use sebas::skills;
use sebas::skills_cmd::{
    AddSource, SkillsArgs, SkillsCmd, classify_source, list_lines, run, sync_lines,
};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// env 是进程全局的：HOME/USERPROFILE 覆写用例共用一把锁串行（同
/// webui_passwd_cli_test 姿态）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// HOME/USERPROFILE 覆写守卫：进入时设为沙箱，离开时**恢复原值**（测试
/// 进程可能原本带着真实的 HOME——Git Bash 环境）。
struct HomeGuard {
    home: Option<OsString>,
    userprofile: Option<OsString>,
}

fn set_home(dir: &Path) -> HomeGuard {
    let home = std::env::var_os("HOME");
    let userprofile = std::env::var_os("USERPROFILE");
    // SAFETY: ENV_LOCK 由调用方持有，无并发 env 访问。
    unsafe {
        std::env::set_var("HOME", dir);
        std::env::set_var("USERPROFILE", dir);
    }
    HomeGuard { home, userprofile }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        // SAFETY: 同上，ENV_LOCK 仍被持有。
        unsafe {
            match self.home.take() {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match self.userprofile.take() {
                Some(v) => std::env::set_var("USERPROFILE", v),
                None => std::env::remove_var("USERPROFILE"),
            }
        }
    }
}

/// 沙箱：一个 tempdir 下齐备仓目录 / 伪 HOME / config.toml。
struct Sandbox {
    dir: tempfile::TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        Sandbox {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// 仓目录（`[skills] dir` 指向它；不预创建——空仓语义要求缺失也算空）。
    fn store(&self) -> PathBuf {
        self.dir.path().join("skills")
    }

    /// 伪 HOME（投影落点 `<home>/.claude/skills` 的根）。
    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn config_path(&self) -> PathBuf {
        self.dir.path().join("config.toml")
    }

    /// 写 config：`[skills] dir` 指向沙箱仓 + configured backends
    /// claude（方言表命中）与 gemini（NoPlacement，如实报告用）。
    /// TOML basic string 里写正斜杠（Windows 反斜杠是非法转义）。
    fn write_config(&self) {
        let store = self.store().display().to_string().replace('\\', "/");
        let toml = format!(
            "[skills]\ndir = \"{store}\"\n\n[acp.agents.claude]\ndriver = \"claude\"\n\n\
             [acp.agents.gemini]\ndriver = \"acp\"\ncommand = [\"sebas-not-a-binary-xyz\"]\n"
        );
        fs::write(self.config_path(), toml).unwrap();
    }

    /// 持锁覆写 HOME/USERPROFILE 并写好 config（返回守卫，调用方持有）。
    fn enter(&self) -> HomeGuard {
        self.write_config();
        set_home(&self.home())
    }

    fn args(&self, cmd: SkillsCmd) -> SkillsArgs {
        SkillsArgs {
            config: self.config_path().display().to_string(),
            cmd,
        }
    }
}

const SKILL_BODY: &str = "---\nname: beads\ndescription: beads 工作流\n---\n\n# beads\n";

fn make_skill(root: &Path, name: &str, body: &str) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), body).unwrap();
    dir
}

/// 递归快照目录树（相对路径 → 文件内容），供「backend 字节不变」断言。
fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    fn walk(cur: &Path, rel: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(cur).unwrap().flatten() {
            let p = entry.path();
            let r = rel.join(entry.file_name());
            if p.is_dir() {
                walk(&p, &r, out);
            } else {
                out.push((
                    r.to_string_lossy().replace('\\', "/"),
                    fs::read(&p).unwrap(),
                ));
            }
        }
    }
    if root.is_dir() {
        walk(root, Path::new(""), &mut out);
    }
    out.sort();
    out
}

// ── 4.1 list ─────────────────────────────────────────────────────────────────

/// 仓目录缺失按空仓处理：`sebas skills list` 不报错（tasks 4.1）。
#[test]
fn list_missing_store_is_not_an_error() {
    let sb = Sandbox::new();
    let _env = ENV_LOCK.lock().unwrap();
    let _home = sb.enter();
    assert!(!sb.store().exists(), "前置：仓目录不存在");
    run(sb.args(SkillsCmd::List)).expect("空仓 list 必须是 Ok（空仓语义，不是错误）");
}

/// 排版：有效条目 `name  desc (N attachments)`、invalid 条目带原因、按名
/// 排序。
#[test]
fn list_lines_format_valid_invalid_and_attachments() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("skills");
    make_skill(&store, "beads", SKILL_BODY);
    let deploy = make_skill(
        &store,
        "my-deploy",
        "---\nname: my-deploy\ndescription: 部署脚本\n---\nbody",
    );
    fs::write(deploy.join("a.sh"), "#!/bin/sh").unwrap();
    fs::write(deploy.join("b.md"), "doc").unwrap();
    make_skill(&store, "broken", "这个目录没有 SKILL.md");

    let lines = list_lines(&store);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert_eq!(lines[0], "beads  beads 工作流", "按名字排序");
    assert!(
        lines[1].starts_with("broken  [invalid]") && lines[1].contains("SKILL.md"),
        "invalid 条目必须带成因: {}",
        lines[1]
    );
    assert_eq!(lines[2], "my-deploy  部署脚本 (2 attachments)");
}

// ── 4.2 add ──────────────────────────────────────────────────────────────────

/// local 形态端到端：`sebas skills add <dir>` 落仓后 list 可见（tasks 4.2）。
#[test]
fn add_local_lands_in_store_and_list_shows_it() {
    let sb = Sandbox::new();
    let _env = ENV_LOCK.lock().unwrap();
    let _home = sb.enter();

    let src_tmp = tempfile::tempdir().unwrap();
    let src = make_skill(src_tmp.path(), "my-skill", SKILL_BODY);
    fs::write(src.join("extra.txt"), "attachment").unwrap();

    run(sb.args(SkillsCmd::Add {
        source: src.display().to_string(),
    }))
    .expect("local add 应成功");

    let landed = sb.store().join("my-skill");
    assert_eq!(
        fs::read_to_string(landed.join("SKILL.md")).unwrap(),
        SKILL_BODY
    );
    assert_eq!(
        fs::read_to_string(landed.join("extra.txt")).unwrap(),
        "attachment"
    );
    let lines = list_lines(&sb.store());
    assert!(
        lines.iter().any(|l| l.starts_with("my-skill  ")),
        "{lines:?}"
    );
}

/// 非法源（无 SKILL.md）报错且不写盘（spec「Add rejects」scenario）。
#[test]
fn add_rejects_dir_without_skill_md_and_store_unchanged() {
    let sb = Sandbox::new();
    let _env = ENV_LOCK.lock().unwrap();
    let _home = sb.enter();

    let src_tmp = tempfile::tempdir().unwrap();
    let src = src_tmp.path().join("not-a-skill");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("README.md"), "no skill").unwrap();

    let err = run(sb.args(SkillsCmd::Add {
        source: src.display().to_string(),
    }))
    .expect_err("无 SKILL.md 的源必须报错");
    assert!(err.to_string().contains("SKILL.md"), "{err}");
    assert!(!sb.store().exists(), "非法源不得写盘");
}

/// 三源分派（design D4）：本地目录 > git 形态（http(s)/git@ 前缀、.git
/// 后缀）> 其余交 npx。
#[test]
fn classify_source_dispatches_three_forms() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("real-dir");
    fs::create_dir_all(&dir).unwrap();

    assert_eq!(
        classify_source(&dir.display().to_string()),
        AddSource::Local(dir.clone()),
        "存在的本地目录最优先"
    );
    assert_eq!(
        classify_source("https://example.com/x.git"),
        AddSource::Git("https://example.com/x.git".into())
    );
    assert_eq!(
        classify_source("git@github.com:owner/repo.git"),
        AddSource::Git("git@github.com:owner/repo.git".into())
    );
    assert_eq!(
        classify_source("http://example.com/plain"),
        AddSource::Git("http://example.com/plain".into())
    );
    assert_eq!(
        classify_source("some-package-name.git"),
        AddSource::Git("some-package-name.git".into()),
        "非目录的 .git 后缀按 git URL 处理"
    );
    assert_eq!(
        classify_source("owner/skills-pkg"),
        AddSource::Npx("owner/skills-pkg".into()),
        "其余形态交 npx"
    );
}

// ── 4.3 remove ───────────────────────────────────────────────────────────────

/// `sebas skills remove` 只删仓内条目，backend 目录字节不变（tasks 4.3）。
#[test]
fn remove_deletes_store_entry_and_leaves_backend_bytes_untouched() {
    let sb = Sandbox::new();
    let _env = ENV_LOCK.lock().unwrap();
    let _home = sb.enter();
    sb.write_config();

    let store = sb.store();
    make_skill(&store, "beads", SKILL_BODY);
    make_skill(&store, "keep", SKILL_BODY);
    // 先 sync 一次，让 backend 里有投影副本 + 名册。
    run(sb.args(SkillsCmd::Sync)).expect("首次 sync");

    let backend = sb.home().join(".claude").join("skills");
    assert!(backend.join("beads").is_dir(), "前置：投影已发生");
    // 用户私产也放一份——remove 同样不许碰它。
    let private = make_skill(
        &backend,
        "user-byhand",
        "---\nname: user-byhand\ndescription: 私产\n---\n",
    );
    fs::write(private.join("secret.txt"), "do not touch").unwrap();
    let before = snapshot(&backend);

    run(sb.args(SkillsCmd::Remove {
        name: "beads".into(),
    }))
    .expect("remove 应成功");
    assert!(!store.join("beads").exists(), "仓内条目被删");
    assert!(store.join("keep").is_dir(), "其余条目不受影响");
    assert_eq!(
        snapshot(&backend),
        before,
        "backend 目录必须字节不变（清理归下一次 sync）"
    );

    // 不存在的条目：报错且指名。
    let err = run(sb.args(SkillsCmd::Remove {
        name: "absent".into(),
    }))
    .expect_err("remove 不存在的条目必须报错");
    assert!(err.to_string().contains("absent"), "{err}");

    // 穿越名：报错，绝不顺着 `../` 删到仓外。
    let escape = sb.dir.path().join("escape");
    fs::create_dir_all(&escape).unwrap();
    fs::write(escape.join("victim.txt"), "survive").unwrap();
    let err = run(sb.args(SkillsCmd::Remove {
        name: "../escape".into(),
    }))
    .expect_err("穿越名必须被拒绝");
    assert!(err.to_string().contains("非法"), "{err}");
    assert_eq!(
        fs::read_to_string(escape.join("victim.txt")).unwrap(),
        "survive"
    );
}

// ── 4.4 sync ─────────────────────────────────────────────────────────────────

/// `sebas skills sync` 端到端：投影落到沙箱 HOME 的 `.claude/skills`；无
/// 落点的 gemini 如实进 no placement；重跑如实报覆盖（tasks 4.4）。
#[test]
fn sync_projects_to_placement_and_reports_no_placement() {
    let sb = Sandbox::new();
    let _env = ENV_LOCK.lock().unwrap();
    let _home = sb.enter();

    let store = sb.store();
    make_skill(&store, "beads", SKILL_BODY);

    // 经 run() 全链路（config → configured kinds → resolve_home → reconcile）。
    run(sb.args(SkillsCmd::Sync)).expect("首次 sync 应成功");
    let backend = sb.home().join(".claude").join("skills");
    assert_eq!(
        fs::read_to_string(backend.join("beads").join("SKILL.md")).unwrap(),
        SKILL_BODY,
        "投影 = 整目录逐字拷贝到方言表落点"
    );
    assert!(backend.join(".sebas-projection.json").is_file(), "名册落地");

    // 报告行：claude 命中（written=1）、gemini 无落点如实报告。
    let home = sb.home();
    let outcome = skills::sync_all(&store, &["claude".into(), "gemini".into()], &home).unwrap();
    let lines = sync_lines(&outcome);
    assert!(
        lines[0].starts_with("claude: written=0 overwritten=1 deleted=0 private_ignored=0"),
        "重跑 sync 同名按覆盖语义如实报告: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("gemini: no placement")),
        "no placement 必须如实报告而不是静默跳过: {lines:?}"
    );

    // 仓里删条目 → 下一次 sync 从 backend 删除（镜像语义闭环）。
    fs::remove_dir_all(store.join("beads")).unwrap();
    run(sb.args(SkillsCmd::Sync)).expect("二次 sync 应成功");
    assert!(
        !backend.join("beads").exists(),
        "已投影条目随仓删除（下次 sync 清理语义）"
    );
}

/// 私产不动 + 删除报告：backend 里名外条目 sync 后字节不变、计数如实。
#[test]
fn sync_leaves_private_entries_and_counts_them() {
    let sb = Sandbox::new();
    let _env = ENV_LOCK.lock().unwrap();
    let _home = sb.enter();

    let store = sb.store();
    make_skill(&store, "beads", SKILL_BODY);
    let backend = sb.home().join(".claude").join("skills");
    let private = make_skill(
        &backend,
        "user-byhand",
        "---\nname: user-byhand\ndescription: 私产\n---\n",
    );
    fs::write(private.join("secret.txt"), "do not touch").unwrap();

    let outcome = skills::sync_all(&store, &["claude".into()], &sb.home()).unwrap();
    let lines = sync_lines(&outcome);
    assert!(
        lines[0].contains("private_ignored=1"),
        "私产只计数: {lines:?}"
    );
    assert_eq!(
        fs::read_to_string(private.join("secret.txt")).unwrap(),
        "do not touch",
        "私产字节不动"
    );
    assert!(
        !lines.iter().any(|l| l.contains("user-byhand")),
        "私产不得列名: {lines:?}"
    );
}

// ── 补充覆盖（review add-agent-skills）───────────────────────────────────────

/// 空仓（目录缺失同）list 的排版：一行说明指名仓路径、不是错误（tasks 4.1
/// 文档化输出约定）。
#[test]
fn list_lines_empty_store_states_store_path_without_error() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("does-not-exist");
    let lines = list_lines(&missing);
    assert_eq!(lines.len(), 1, "空仓打一行说明: {lines:?}");
    assert!(
        lines[0].contains("skill 仓为空") && lines[0].contains(&missing.display().to_string()),
        "说明须点名仓路径: {}",
        lines[0]
    );
}

/// sync 报告排版：written/overwritten/deleted 列表逐名带 `+`/`~`/`-` 前缀、
/// 计数与名单一致；no placement 的 backend 未写任何文件（只一行说明）。
#[test]
fn sync_lines_render_name_lists_and_no_placement_wrote_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp.path().join("store");
    // 落点经方言表从 home 推导：`<home>/.claude/skills`。
    let home = tmp.path().join("home");
    let backend = home.join(".claude").join("skills");
    fs::create_dir_all(&backend).unwrap();
    make_skill(&store, "fresh", SKILL_BODY);
    make_skill(&store, "updated", SKILL_BODY);

    // 预置：updated 已投影过（重跑成覆盖）、stale 只在名册里（→ deleted）、
    // user-byhand 是用户私产（→ 只计数）。
    make_skill(&backend, "updated", SKILL_BODY);
    let stale = make_skill(
        &backend,
        "stale",
        "---\nname: stale\ndescription: 旧\n---\n",
    );
    fs::write(stale.join("old.txt"), "old").unwrap();
    make_skill(&backend, "user-byhand", SKILL_BODY);
    fs::write(
        backend.join(sebas::skills::PROJECTION_MANIFEST),
        r#"{"projected": ["updated", "stale"], "store_hash": "x"}"#,
    )
    .unwrap();

    let outcome = skills::sync_all(&store, &["claude".into()], &home).unwrap();
    assert_eq!(outcome[0].backend, "claude");
    let report = outcome[0].report.as_ref().unwrap();
    assert_eq!(report.written, vec!["fresh".to_string()]);
    assert_eq!(report.overwritten, vec!["updated".to_string()]);
    assert_eq!(report.deleted, vec!["stale".to_string()]);
    assert_eq!(report.private_ignored, 1);

    let lines = sync_lines(&outcome);
    assert!(
        lines[0].contains("written=1 overwritten=1 deleted=1 private_ignored=1"),
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.trim() == "+ fresh"),
        "写入名单逐名呈现: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("  ~ updated")),
        "覆盖名单带仓 wins 注记: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.starts_with("  - stale")),
        "删除名单逐名呈现: {lines:?}"
    );

    // NoPlacement 形态：一行说明，明确「未写任何文件」。
    let outcome = skills::sync_all(&store, &["gemini".into()], &home).unwrap();
    let lines = sync_lines(&outcome);
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("gemini: no placement") && lines[0].contains("未写任何文件"),
        "NoPlacement 如实一行: {}",
        lines[0]
    );
}

/// add 之后仓即唯一记录：store 下只有条目目录本身，没有任何 sebas 维护的
/// 索引/状态文件（spec「the store on disk is the only record」）。
#[test]
fn add_leaves_no_index_file_store_is_the_only_record() {
    let sb = Sandbox::new();
    let _env = ENV_LOCK.lock().unwrap();
    let _home = sb.enter();

    let src_tmp = tempfile::tempdir().unwrap();
    let src = make_skill(src_tmp.path(), "my-skill", SKILL_BODY);
    run(sb.args(SkillsCmd::Add {
        source: src.display().to_string(),
    }))
    .expect("local add 应成功");

    let mut entries: Vec<String> = fs::read_dir(sb.store())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec!["my-skill".to_string()],
        "仓内只有条目目录，无索引/状态文件"
    );
}
