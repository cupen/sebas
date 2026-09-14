//! 操作者级 skill 仓 + 多 backend 方言投影（openspec/changes/add-agent-skills）。
//!
//! core 侧全部实体都在这里，CLI 与 webui 只是薄壳（design D4）：
//! - [`scan_store`]：扫仓成 `Vec<SkillInfo>`。仓即文件系统，读=现扫盘、无索引
//!   无缓存；目录缺失=空仓（空列表，不是错误）。
//! - [`add_from_local`] / [`add_from_git`] / [`add_from_npx`]：三源落仓。add 是
//!   社区方案的薄封装，不发明私有格式；撞名报错——「仓 wins」是 **sync** 语义，
//!   add 不覆盖既有条目。
//! - [`reconcile`]：把仓内容按名镜像进一个 backend 落点目录（design D2 四分支）。
//!   名册 `<落点>/.sebas-projection.json` 是全模块唯一状态，tmp+rename 原子写回。
//! - [`placement_for`]：方言表（design D1）。本期只 claude/codex 两条，
//!   [`Convert::Identity`] 唯一变体；查无 → `None`，由调用方如实呈现 NoPlacement。
//!
//! 安全约定：所有路径由调用方注入（config / CLI 参数 / 测试 tempdir），本模块不
//! 从环境推断 HOME——测试绝不落真实 `~/.claude`、`~/.agents` 等产品目录。

use crate::error::{Result, SebasError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// agentskills 格式的定义文件名（各消费方文档共同约定：文件名必须恰为
/// `SKILL.md`，大小写在大小写敏感文件系统上有别）。
pub const SKILL_FILE: &str = "SKILL.md";

/// 每 backend 一份的投影名册（design D2）：reconcile 删除语义所需的唯一状态。
pub const PROJECTION_MANIFEST: &str = ".sebas-projection.json";

/// 仓内一个条目的扫描结果（webui `GET /api/skills` 的行形状，design D4）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillInfo {
    /// 目录名（仓内 slug）。remove / sync / 投影全以它为键；frontmatter 的
    /// `name` 只做在场校验，不替代目录名。
    pub name: String,
    /// frontmatter `description`（invalid 时为 None）。
    pub description: Option<String>,
    /// `SKILL.md` 之外的随附文件（相对 skill 目录的 `/` 分隔路径，已排序）。
    pub attachments: Vec<String>,
    /// frontmatter 是否带齐 `name` + `description`。
    pub valid: bool,
    /// invalid 成因（valid 时为 None）。
    pub invalid_reason: Option<String>,
}

/// 一次 sync 对一个 backend 落点的结果（design D4 报告形状去掉 `no_placement`：
/// 那一项属于「跨全部 backend」的上层汇总，由调用方按方言表查询结果拼装）。
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    /// 本次新写入（backend 里原先没有同名条目）。
    pub written: Vec<String>,
    /// 同名覆盖（backend 里原有——含用户手改——按「仓 wins」原样换掉）。
    pub overwritten: Vec<String>,
    /// 上次投影过、这次仓里已无 → 从 backend 删除。
    pub deleted: Vec<String>,
    /// 名外条目计数（用户私产：不读、不导、不动，只报数）。
    pub private_ignored: usize,
}

// ── 扫仓（tasks 2.1）─────────────────────────────────────────────────────────

/// 扫仓：目录缺失 = 空仓（空列表，不是错误）；坏条目标 invalid 但不中断列表；
/// 点前缀条目（`.git` 等杂物）与非目录一律跳过；按名字排序。
pub fn scan_store(store: &Path) -> Vec<SkillInfo> {
    let Ok(entries) = fs::read_dir(store) else {
        return Vec::new();
    };
    let mut out: Vec<SkillInfo> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            scan_skill_entry(&name, &e.path())
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn scan_skill_entry(name: &str, dir: &Path) -> SkillInfo {
    let attachments = list_attachments(dir);
    let skill_md = dir.join(SKILL_FILE);
    let parsed = if !skill_md.is_file() {
        Err(format!("缺少 {SKILL_FILE}"))
    } else {
        fs::read_to_string(&skill_md)
            .map_err(|e| format!("{SKILL_FILE} 读取失败：{e}"))
            .and_then(|text| parse_frontmatter(&text))
    };
    match parsed {
        Ok(fm) => match (fm.name, fm.description) {
            (Some(_), Some(description)) => SkillInfo {
                name: name.to_string(),
                description: Some(description),
                attachments,
                valid: true,
                invalid_reason: None,
            },
            (Some(_), None) => invalid_entry(name, attachments, "frontmatter 缺 description"),
            (None, _) => invalid_entry(name, attachments, "frontmatter 缺 name"),
        },
        Err(reason) => invalid_entry(name, attachments, &reason),
    }
}

fn invalid_entry(name: &str, attachments: Vec<String>, reason: &str) -> SkillInfo {
    SkillInfo {
        name: name.to_string(),
        description: None,
        attachments,
        valid: false,
        invalid_reason: Some(reason.to_string()),
    }
}

/// 随附文件清单：`SKILL.md` 之外的所有普通文件，相对路径（`/` 分隔）、已排序；
/// 空目录不列。
fn list_attachments(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    collect_attachments(dir, dir, &mut out);
    out.sort();
    out
}

fn collect_attachments(root: &Path, cur: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(cur) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        if path.is_dir() {
            collect_attachments(root, &path, out);
        } else if rel != Path::new(SKILL_FILE) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// SKILL.md frontmatter 的最小解析结果（只认 `name`/`description` 两个键）。
#[derive(Debug, Default, PartialEq, Eq)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
}

/// 手写最小 YAML frontmatter 解析（仓库无 yaml 解析依赖，agentskills 要求的字段
/// 又只是平面 `key: value`，不为此引入 serde_yaml）：`---` 围栏内逐行取键，值
/// 两侧成对引号剥掉。结构性缺失（无围栏 / 围栏未闭合）返回 Err；字段缺不缺由
/// 调用方判断。
///
/// 注意签名用 `std::result::Result`：本模块顶部的 `crate::error::Result` 别名
/// 只有一个泛型参数，错误类型固定为 `SebasError`，而这里的 Err 是普通 String。
fn parse_frontmatter(text: &str) -> std::result::Result<Frontmatter, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err("SKILL.md 必须以 --- 围栏开头（frontmatter 缺失）".into());
    }
    let mut fm = Frontmatter::default();
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.trim() == "---" {
            return Ok(fm);
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue; // 围栏内的非 `key: value` 行不参与解析
        };
        let value = strip_pair_quotes(value.trim());
        match key.trim() {
            "name" => fm.name = Some(value),
            "description" => fm.description = Some(value),
            _ => {} // agentskills 未知字段本就允许，忽略
        }
    }
    Err("frontmatter 围栏未闭合（缺收尾 ---）".into())
}

fn strip_pair_quotes(value: &str) -> String {
    let bytes = value.as_bytes();
    let paired = bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''));
    if paired {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

// ── 三源落仓（tasks 2.2 / 2.3）───────────────────────────────────────────────

/// 本地目录落仓：校验 `SKILL.md` 在场（缺 → Err 指名，不写盘），整目录拷贝到
/// `<store>/<basename>`。撞名报错（不覆盖）。返回落仓的名字。
pub fn add_from_local(src: &Path, store: &Path) -> Result<String> {
    if !src.join(SKILL_FILE).is_file() {
        return Err(SebasError::Skills(format!(
            "源目录 {} 缺少 {SKILL_FILE}，不是 agentskills 格式的 skill，未写入仓",
            src.display()
        )));
    }
    let Some(name) = src.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Err(SebasError::Skills(format!(
            "源路径 {} 没有目录名，无法定夺仓内条目名",
            src.display()
        )));
    };
    let dest = store.join(&name);
    if dest.exists() {
        return Err(SebasError::Skills(format!(
            "仓里已有同名条目 {name:?}；add 不覆盖既有 skill，请先 remove 或改名后再 add"
        )));
    }
    fs::create_dir_all(store)?;
    copy_dir_recursive(src, &dest)?;
    Ok(name)
}

/// git URL 落仓：clone 到临时目录 → 找「含 SKILL.md 的顶层目录」——仓库根有
/// `SKILL.md` 即单技能仓（仓库根=技能目录，仓名=URL 末段）；否则扫一级子目录
/// （多技能仓）。候选先预检撞名（任一同名 → 整体不写，避免收一半），再逐个按
/// [`add_from_local`] 语义落仓。返回落仓名列表。
pub fn add_from_git(url: &str, store: &Path) -> Result<Vec<String>> {
    let git = resolve_command("git").ok_or_else(|| {
        SebasError::Skills("本机 PATH 上找不到 git：git 形态的 skills add 依赖 git，请先安装".into())
    })?;
    let tmp = tempfile::tempdir()?;
    let repo_dir = tmp.path().join(repo_dir_name(url));
    let status = run_external(
        &git,
        &[
            OsStr::new("clone"),
            OsStr::new("--depth"),
            OsStr::new("1"),
            OsStr::new(url),
            repo_dir.as_os_str(),
        ],
    )
    .map_err(|e| SebasError::Skills(format!("无法启动 git：{e}")))?;
    require_status(status, &format!("git clone {url}"))?;

    if repo_dir.join(SKILL_FILE).is_file() {
        return add_from_local(&repo_dir, store).map(|n| vec![n]);
    }
    let mut candidates: Vec<PathBuf> = fs::read_dir(&repo_dir)?
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.join(SKILL_FILE).is_file())
        .collect();
    candidates.sort();
    if candidates.is_empty() {
        return Err(SebasError::Skills(format!(
            "克隆的仓库 {url} 里没有任何含 {SKILL_FILE} 的目录（仓库根与一级子目录都查过），无从落仓"
        )));
    }
    for cand in &candidates {
        let name = cand
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if store.join(&name).exists() {
            return Err(SebasError::Skills(format!(
                "仓里已有同名条目 {name:?}；add 不覆盖既有 skill，请先 remove 或改名后再 add"
            )));
        }
    }
    let mut added = Vec::new();
    for cand in &candidates {
        added.push(add_from_local(cand, store)?);
    }
    Ok(added)
}

/// `npx skills add <pkg>` 的薄封装（vercel-labs/skills，索引站 skills.sh）。
///
/// **已知边界**：该 CLI 没有「指定安装目录」的旗标（只有 `-g` 切项目/用户 scope、
/// `-a` 选目标 agent），安装目的地由它按本机 agent 检测自行决定，sebas 控制不了。
/// 因此本函数只封装到「命令执行成功」层，不把结果收编进仓——`store` 参数为签名
/// 一致保留、本期不消费；真实落点归 6.x 手测记录。`-y`（npx 侧：免「装包?」确认）
/// + `--yes`（skills 侧：跳过交互提示），避免命令卡在交互式提示上。
pub fn add_from_npx(pkg: &str, store: &Path) -> Result<()> {
    let _ = store; // 落点不可控（见上），本期不消费
    let npx = resolve_command("npx").ok_or_else(|| {
        SebasError::Skills(
            "本机 PATH 上找不到 npx：npx 形态的 skills add 依赖 Node.js/npx，请先安装".into(),
        )
    })?;
    let status = run_external(
        &npx,
        &[
            OsStr::new("-y"),
            OsStr::new("skills"),
            OsStr::new("add"),
            OsStr::new(pkg),
            OsStr::new("--yes"),
        ],
    )
    .map_err(|e| SebasError::Skills(format!("无法启动 npx：{e}")))?;
    require_status(status, &format!("npx skills add {pkg}"))
}

/// 从 git URL 推克隆目录名（= 单技能仓落仓名）：取末段、去 `.git` 后缀；
/// 推不出（空段）就用 `repo` 兜底。
fn repo_dir_name(url: &str) -> String {
    let trimmed = url.trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\', ':']).next().unwrap_or("");
    let stem = last.strip_suffix(".git").unwrap_or(last);
    if stem.is_empty() {
        "repo".into()
    } else {
        stem.to_string()
    }
}

// ── 投影 reconcile（tasks 2.4，design D2）────────────────────────────────────

/// 投影名册：只记最少状态——上次投影过哪些名字 + 一个人工排查指纹。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProjectionManifest {
    projected: Vec<String>,
    /// 指纹（仓内条目名排序列表的 sha256），仅供人工比对；reconcile 决策**不读**
    /// 它——覆盖与否由「仓 wins」语义决定，不用比较。
    store_hash: String,
}

/// D2 四分支投影，全部幂等：
///
/// 1. 仓内每个**有效** skill → 写/覆盖 backend 同名目录（先删后拷，整目录镜像），
///    记入新名册；
/// 2. 旧名册 − 仓 → 从 backend 删除，移出新名册；
/// 3. backend 里名外条目 → 用户私产，不读不动，只计数；
/// 4. 名册原子替换（tmp+rename）。
///
/// 名册缺失/损坏 → 退化为「只投影不删除」（名外全算私产，安全方向），本轮结束
/// 照常写入新名册，下一轮 sync 恢复删除语义。
///
/// 只投影**有效**条目：invalid 的目录不是 skill，不往 backend 传播；它若曾被
/// 投影过，本轮按分支 2 从 backend 删除（仓里已「没有」这个 skill）。
pub fn reconcile(store: &Path, backend_dir: &Path) -> Result<SyncReport> {
    let store_skills = scan_store(store);
    let store_names: Vec<String> = store_skills
        .iter()
        .filter(|s| s.valid)
        .map(|s| s.name.clone())
        .collect();
    let store_set: HashSet<&str> = store_names.iter().map(String::as_str).collect();

    fs::create_dir_all(backend_dir)?;
    let manifest_path = backend_dir.join(PROJECTION_MANIFEST);
    let old_projected: Vec<String> = fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|text| serde_json::from_str::<ProjectionManifest>(&text).ok())
        .map(|m| m.projected)
        .unwrap_or_default();

    let mut report = SyncReport::default();
    let mut new_projected = Vec::new();

    // 分支 1：仓 wins 写入。
    for name in &store_names {
        let dest = backend_dir.join(name);
        let existed = dest.exists();
        if existed {
            fs::remove_dir_all(&dest)?;
        }
        copy_dir_recursive(&store.join(name), &dest)?;
        if existed {
            report.overwritten.push(name.clone());
        } else {
            report.written.push(name.clone());
        }
        new_projected.push(name.clone());
    }

    // 分支 2：旧名册 − 仓 → 删。名册可能被手改：只信到「一级条目名」为止，
    // 绝不顺着一个带路径分隔符 / `..` 的名字删到落点目录之外。
    for name in &old_projected {
        if store_set.contains(name.as_str()) || !is_safe_entry_name(name) {
            continue;
        }
        let dest = backend_dir.join(name);
        if dest.is_dir() {
            fs::remove_dir_all(&dest)?;
            report.deleted.push(name.clone());
        }
    }

    // 分支 3：名外即私产——只计数，不读不动。
    for entry in fs::read_dir(backend_dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == PROJECTION_MANIFEST || store_set.contains(name.as_str()) {
            continue;
        }
        if old_projected.contains(&name) {
            continue; // 旧名册成员已由分支 2 处置（删了，或落点上本来就不在）
        }
        report.private_ignored += 1;
    }

    // 分支 4：名册原子写回。缺册/坏册的退化轮也写——下一轮 sync 恢复删除语义。
    let manifest = ProjectionManifest {
        projected: new_projected,
        store_hash: store_hash(&store_names),
    };
    write_manifest_atomic(&manifest_path, &manifest)?;
    Ok(report)
}

/// 名册条目/仓条目名只允许充当一级目录名（reconcile 删除路径与 remove /
/// URL 参数共享的安全阀）：拒绝空名、`.` / `..`、任何带路径分隔符与 NUL 的
/// 名字。
pub fn is_safe_entry_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', '\0'])
        && Path::new(name).file_name().is_some_and(|n| n == name)
}

/// 指纹：条目名（已排序）以 NUL 连接后的 sha256。仅人工比对用。
fn store_hash(names: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for name in names {
        hasher.update(name.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

/// tmp+rename 原子写名册（同卷；Windows 的 `fs::rename` 走
/// MOVEFILE_REPLACE_EXISTING，可替换既有文件）。
fn write_manifest_atomic(path: &Path, manifest: &ProjectionManifest) -> Result<()> {
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| SebasError::Skills(format!("投影名册序列化失败：{e}")))?;
    let tmp = path
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{PROJECTION_MANIFEST}.tmp"));
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)?.flatten() {
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

// ── 方言表（tasks 3.1，design D1）────────────────────────────────────────────

/// backend 消费格式转换（D1）：本期只有 Identity——同构整目录拷贝，frontmatter
/// 逐字保留。枚举先占位，给将来真异构的 backend 留扩展点，不为它写任何转换代码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Convert {
    Identity,
}

/// 一个 backend 的 skill 落点（D1 静态方言表的一行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub backend_kind: &'static str,
    /// 落点目录（已按**注入的 home** 展开——测试绝不落真实 HOME）。
    pub dir: PathBuf,
    pub convert: Convert,
}

/// 静态方言表：`(backend kind, home 相对落点)`。本期只 claude/codex 两条（spec
/// 只承诺 claude 的 placement）。gemini / opencode 的调研结论见 design.md
/// 「方言矩阵调研」——同构属实，但加行留给后续 change，不随本表扩。
const PLACEMENTS: &[(&str, &str)] = &[("claude", ".claude/skills"), ("codex", ".codex/skills")];

/// 方言表查询。查无 → `None`，由调用方呈现 NoPlacement（spec：如实报告，不许
/// 静默跳过）。
pub fn placement_for(backend_kind: &str, home: &Path) -> Option<Placement> {
    PLACEMENTS
        .iter()
        .find(|(kind, _)| *kind == backend_kind)
        .map(|(kind, rel)| Placement {
            backend_kind: kind,
            dir: home.join(rel),
            convert: Convert::Identity,
        })
}

// ── 薄壳共用操作面（tasks 4.1–4.4 CLI / 5.1 webui，design D4）───────────────

/// 家目录解析（sync 投影落点的 home 来源）：env 优先（`HOME` > `USERPROFILE`
/// ——测试覆写即钉住沙箱，与 `im_cmd` 的 env-first 同款）；缺失回退
/// `dirs::home_dir()`（与 `config::expand_tilde` 同源；注意 Windows 上它走
/// Known Folder API、不吃 env 覆写，所以 env 检测必须在前面）；再缺失回退
/// 当前目录。
pub fn resolve_home() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|h| !h.is_empty()))
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 按名删除仓内条目（tasks 4.3 / webui DELETE）：只删 `<store>/<name>`，
/// **不动任何 backend**（backend 里的副本归下一次 sync 清理）。名字非法或
/// 条目不在仓 → Err。
pub fn remove_skill(store: &Path, name: &str) -> Result<()> {
    if !is_safe_entry_name(name) {
        return Err(SebasError::Skills(format!("非法的 skill 名 {name:?}")));
    }
    let dir = store.join(name);
    if !dir.is_dir() {
        return Err(SebasError::Skills(format!(
            "仓里没有条目 {name:?}（store={}），无需删除",
            store.display()
        )));
    }
    fs::remove_dir_all(&dir)?;
    Ok(())
}

/// 一个条目的详情（webui `GET /api/skills/{name}` 的取材）：SKILL.md 原文
/// + attachments 文件名列表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDetailText {
    pub name: String,
    /// SKILL.md 原文；条目在仓但缺该文件（invalid）时为 None——诚实呈现，
    /// 不冒充 404。
    pub text: Option<String>,
    pub attachments: Vec<String>,
}

/// 读条目详情：名字非法或条目不在仓 → None。
pub fn skill_detail(store: &Path, name: &str) -> Option<SkillDetailText> {
    if !is_safe_entry_name(name) {
        return None;
    }
    let entry = scan_store(store).into_iter().find(|s| s.name == name)?;
    let text = fs::read_to_string(store.join(name).join(SKILL_FILE)).ok();
    Some(SkillDetailText {
        name: name.to_string(),
        text,
        attachments: entry.attachments,
    })
}

/// 一次 sync 对一个 configured backend 的结果：`report = Some` → 有落点、
/// 已投影；`None` → NoPlacement（spec「reported, not skipped」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendOutcome {
    pub backend: String,
    pub report: Option<SyncReport>,
}

/// 对一个 configured backend kind 跑投影（tasks 4.4）：方言表命中 →
/// reconcile，未命中 → [`BackendOutcome`] 带 `report = None`。
pub fn sync_kind(store: &Path, kind: &str, home: &Path) -> Result<BackendOutcome> {
    match placement_for(kind, home) {
        Some(placement) => Ok(BackendOutcome {
            backend: kind.to_string(),
            report: Some(reconcile(store, &placement.dir)?),
        }),
        None => Ok(BackendOutcome {
            backend: kind.to_string(),
            report: None,
        }),
    }
}

/// 全量 sync：对每个 configured backend kind（顺序 = [`configured_kinds`]）
/// 各出一个 [`BackendOutcome`]。单个 backend 投影失败即整单失败——报告不
/// 完整宁可报错。
pub fn sync_all(store: &Path, kinds: &[String], home: &Path) -> Result<Vec<BackendOutcome>> {
    kinds.iter().map(|k| sync_kind(store, k, home)).collect()
}

/// configured backend kinds（排序去重）：取 `[acp.agents.*]` 键；一个都没配
/// 时回退 default kind（历史语义：无配置时的隐式 claude，与
/// `AcpConfig::default_kind_binary` 的兜底理由一致）——`sebas skills sync`
/// 在出厂配置上也有明确可投影的对象。
pub fn configured_kinds(cfg: &crate::config::Config) -> Vec<String> {
    let mut kinds: Vec<String> = cfg.acp.agents.keys().cloned().collect();
    if kinds.is_empty() {
        kinds.push(cfg.acp.default_kind().to_string());
    }
    kinds.sort();
    kinds
}

// ── webui 接缝的文件系统实现（tasks 5.1；trait 见 sebas_webui::skills）──────

/// [`sebas_webui::skills::SkillsService`] 的文件系统实现：仓目录与
/// placement / no_placement 表在装配时定死，操作全部委托本模块 core 函数
/// ——CLI 与 webui 两调用方同源（design「core 是唯一逻辑所在地」）。
pub struct FsSkillsService {
    store: PathBuf,
    /// 有落点的 configured backend：(kind, 落点目录)。
    placements: Vec<(String, PathBuf)>,
    /// 无落点的 configured backend（spec：如实报告，不许静默跳过）。
    no_placement: Vec<String>,
}

impl FsSkillsService {
    /// 生产装配：仓目录 = config `[skills] dir`（已展开 `~`），placement 表
    /// = configured kinds × 方言表（home 经 [`resolve_home`] 解析）。
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        let store = PathBuf::from(cfg.skills_dir());
        let home = resolve_home();
        let mut placements = Vec::new();
        let mut no_placement = Vec::new();
        for kind in configured_kinds(cfg) {
            match placement_for(&kind, &home) {
                Some(p) => placements.push((kind, p.dir)),
                None => no_placement.push(kind),
            }
        }
        Self {
            store,
            placements,
            no_placement,
        }
    }

    /// 测试装配：store 与落点表直接注入（tempdir 沙箱，绝不触真实 HOME）。
    pub fn with_placements(
        store: PathBuf,
        placements: Vec<(String, PathBuf)>,
        no_placement: Vec<String>,
    ) -> Self {
        Self {
            store,
            placements,
            no_placement,
        }
    }
}

impl sebas_webui::skills::SkillsService for FsSkillsService {
    fn list(&self) -> Vec<sebas_webui::skills::SkillEntry> {
        scan_store(&self.store)
            .into_iter()
            .map(|s| sebas_webui::skills::SkillEntry {
                name: s.name,
                description: s.description,
                attachments: s.attachments,
                valid: s.valid,
                reason: s.invalid_reason,
            })
            .collect()
    }

    fn detail(&self, name: &str) -> Option<sebas_webui::skills::SkillDetail> {
        skill_detail(&self.store, name).map(|d| sebas_webui::skills::SkillDetail {
            name: d.name,
            text: d.text,
            attachments: d.attachments,
        })
    }

    fn delete(&self, name: &str) -> std::result::Result<bool, String> {
        if !is_safe_entry_name(name) {
            return Ok(false);
        }
        let dir = self.store.join(name);
        if !dir.is_dir() {
            return Ok(false);
        }
        fs::remove_dir_all(&dir)
            .map(|_| true)
            .map_err(|e| e.to_string())
    }

    fn sync(&self) -> std::result::Result<sebas_webui::skills::SkillsSyncOutcome, String> {
        let reports = self
            .placements
            .iter()
            .map(|(kind, dir)| {
                reconcile(&self.store, dir).map(|report| sebas_webui::skills::BackendSyncReport {
                    backend: kind.clone(),
                    written: report.written,
                    overwritten: report.overwritten,
                    deleted: report.deleted,
                    private_ignored: report.private_ignored,
                })
            })
            .collect::<std::result::Result<Vec<_>, SebasError>>()
            .map_err(|e| e.to_string())?;
        Ok(sebas_webui::skills::SkillsSyncOutcome {
            reports,
            no_placement: self.no_placement.clone(),
        })
    }
}

// ── 外部命令探测与执行（tasks 2.3）───────────────────────────────────────────

/// 探测外部命令并解析出完整路径（语义抄自 sebas-node/src/config.rs 的
/// probe_command + path_candidates：显式路径直接验存在；裸名扫 PATH，Windows 按
/// PATHEXT 展开候选）。返回路径而非 bool——Windows 上 `Command::new` 不做 PATHEXT
/// 解析（`npx` 实际是 `npx.cmd`），spawn 必须用解析到的完整路径。
fn resolve_command(command: &str) -> Option<PathBuf> {
    if command.trim().is_empty() {
        return None;
    }
    let path = Path::new(command);
    if command.contains('/') || command.contains('\\') {
        return is_executable_file(path).then(|| path.to_path_buf());
    }
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        for candidate in path_candidates(command) {
            let full = dir.join(&candidate);
            if is_executable_file(&full) {
                return Some(full);
            }
        }
    }
    None
}

/// 裸命令名在 PATH 上的候选文件名（sebas-node 同款）：Windows 按 PATHEXT 解析
/// （`git` 要能找到 `git.cmd`）；带扩展名的名字与其他平台原样查找。
fn path_candidates(command: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        if Path::new(command).extension().is_some() {
            return vec![command.to_string()];
        }
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        exts.split(';')
            .filter(|e| e.starts_with('.'))
            .map(|e| format!("{command}{e}"))
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![command.to_string()]
    }
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.is_file()
            && fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// 起外部命令、继承 stdio（用户要看得见 git/npx 的输出）、等待退出。
/// Windows 上 `.cmd`/`.bat` 无法被 CreateProcess 直接执行，经 `cmd /c` 中转。
fn run_external(program: &Path, args: &[&OsStr]) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(windows)]
    {
        let is_script = matches!(
            program.extension().and_then(|e| e.to_str()),
            Some("cmd") | Some("bat")
        );
        let mut cmd = if is_script {
            let mut c = Command::new("cmd");
            c.arg("/c").arg(program);
            c
        } else {
            Command::new(program)
        };
        cmd.args(args).status()
    }
    #[cfg(not(windows))]
    {
        Command::new(program).args(args).status()
    }
}

fn require_status(status: std::process::ExitStatus, what: &str) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    let detail = status
        .code()
        .map(|c| format!("exit {c}"))
        .unwrap_or_else(|| "进程异常终止".into());
    Err(SebasError::Skills(format!("{what} 失败（{detail}）")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// env 是进程全局的：动 PATH 的用例互相污染，用互斥锁串行化（同
    /// `src/provider.rs` 的 ENV_LOCK 惯例）。
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const SKILL_BODY: &str = "---\nname: beads\ndescription: beads 工作流\n---\n\n# beads\n";

    fn make_skill(root: &Path, name: &str, body: &str) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(SKILL_FILE), body).unwrap();
        dir
    }

    /// 临时 PATH：把 `dir` 前置到现有 PATH 跑 `f`，结束恢复（锁内执行）。
    fn with_path_prepended<R>(dir: &Path, f: impl FnOnce() -> R) -> R {
        let _env = ENV_LOCK.lock().unwrap();
        let original = std::env::var_os("PATH");
        let joined: std::ffi::OsString = match &original {
            Some(p) => std::env::join_paths(
                std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(p)),
            )
            .unwrap(),
            None => dir.as_os_str().to_os_string(),
        };
        unsafe { std::env::set_var("PATH", &joined) };
        let out = f();
        match original {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        out
    }

    /// 清空 PATH（指向一个空目录）跑 `f`，结束恢复——覆盖「命令缺失报错」分支。
    fn with_path_cleared<R>(f: impl FnOnce() -> R) -> R {
        let _env = ENV_LOCK.lock().unwrap();
        let original = std::env::var_os("PATH");
        let empty = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("PATH", empty.path()) };
        let out = f();
        if let Some(p) = original {
            unsafe { std::env::set_var("PATH", p) };
        }
        out
    }

    /// stub `git`：行为由 `SEBAS_STUB_GIT_MODE` 决定——缺省 clone 出单技能仓根；
    /// `multi` = 一级子目录 a/b 各一个技能；`empty` = 空目录（无 SKILL.md）。
    /// 返回 stub 所在目录（塞进 PATH 用）。
    fn make_git_stub(tmp: &Path) -> PathBuf {
        let bin = tmp.join("bin-git");
        fs::create_dir_all(&bin).unwrap();
        #[cfg(windows)]
        let script = r#"@echo off
if /I not "%~1"=="clone" exit /b 1
mkdir "%~5"
if "%SEBAS_STUB_GIT_MODE%"=="multi" (
  mkdir "%~5\a"
  > "%~5\a\SKILL.md" echo ---
  >> "%~5\a\SKILL.md" echo name: a
  mkdir "%~5\b"
  > "%~5\b\SKILL.md" echo ---
  >> "%~5\b\SKILL.md" echo name: b
  exit /b 0
)
if "%SEBAS_STUB_GIT_MODE%"=="empty" exit /b 0
> "%~5\SKILL.md" echo ---
>> "%~5\SKILL.md" echo name: stub-skill
exit /b 0
"#;
        #[cfg(not(windows))]
        let script = r#"#!/bin/sh
[ "$1" = "clone" ] || exit 1
mkdir -p "$5"
if [ "$SEBAS_STUB_GIT_MODE" = "multi" ]; then
  mkdir -p "$5/a" "$5/b"
  printf -- '---\nname: a\n---\n' > "$5/a/SKILL.md"
  printf -- '---\nname: b\n---\n' > "$5/b/SKILL.md"
  exit 0
fi
if [ "$SEBAS_STUB_GIT_MODE" = "empty" ]; then exit 0; fi
printf -- '---\nname: stub-skill\n---\n' > "$5/SKILL.md"
exit 0
"#;
        let path = if cfg!(windows) {
            bin.join("git.cmd")
        } else {
            bin.join("git")
        };
        fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        bin
    }

    /// stub `npx`：把收到的 argv 原样写进 `SEBAS_STUB_NPX_LOG` 指向的文件后退出
    /// 0（`SEBAS_STUB_NPX_EXIT=1` 时退出 1）。返回 stub 所在目录。
    fn make_npx_stub(tmp: &Path) -> PathBuf {
        let bin = tmp.join("bin-npx");
        fs::create_dir_all(&bin).unwrap();
        #[cfg(windows)]
        let script = "@echo off\n\
            if \"%SEBAS_STUB_NPX_EXIT%\"==\"1\" exit /b 1\n\
            > \"%SEBAS_STUB_NPX_LOG%\" echo %*\n\
            exit /b 0\n";
        #[cfg(not(windows))]
        let script = "#!/bin/sh\n\
            [ \"$SEBAS_STUB_NPX_EXIT\" = \"1\" ] && exit 1\n\
            printf '%s\\n' \"$*\" > \"$SEBAS_STUB_NPX_LOG\"\n\
            exit 0\n";
        let path = if cfg!(windows) {
            bin.join("npx.cmd")
        } else {
            bin.join("npx")
        };
        fs::write(&path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        bin
    }

    // ── scan_store ──────────────────────────────────────────────────────────

    #[test]
    fn scan_lists_valid_skills_with_attachments() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("skills");
        make_skill(&store, "beads", SKILL_BODY);
        let deploy = make_skill(
            &store,
            "my-deploy",
            "---\nname: my-deploy\ndescription: 部署\n---\nbody",
        );
        fs::create_dir_all(deploy.join("scripts")).unwrap();
        fs::write(deploy.join("scripts").join("deploy.sh"), "#!/bin/sh\n").unwrap();
        fs::write(deploy.join("ref.md"), "doc").unwrap();

        let skills = scan_store(&store);
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].name, "beads", "按名字排序");
        assert!(skills[0].valid);
        assert_eq!(skills[0].description.as_deref(), Some("beads 工作流"));
        assert!(skills[0].attachments.is_empty());
        assert_eq!(skills[1].name, "my-deploy");
        assert_eq!(
            skills[1].attachments,
            vec!["ref.md".to_string(), "scripts/deploy.sh".to_string()]
        );
    }

    #[test]
    fn scan_marks_broken_entries_invalid_without_aborting() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("skills");
        make_skill(&store, "beads", SKILL_BODY); // 唯一的有效条目
        make_skill(&store, "no-skill-md", "其实没有 SKILL.md 的杂物目录");
        make_skill(&store, "no-name", "---\ndescription: 缺 name\n---\n");
        make_skill(&store, "no-desc", "---\nname: no-desc\n---\n");
        make_skill(&store, "no-fence", "name: x\ndescription: 无围栏\n");
        make_skill(&store, "unterminated", "---\nname: x\ndescription: y\n");

        let skills = scan_store(&store);
        assert_eq!(skills.len(), 6, "坏条目不中断列表");
        let valid: Vec<_> = skills.iter().filter(|s| s.valid).collect();
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].name, "beads");
        for s in skills.iter().filter(|s| !s.valid) {
            let reason = s.invalid_reason.as_deref().unwrap();
            assert!(!reason.is_empty(), "{} 必须带成因", s.name);
        }
        let no_skill_md = skills.iter().find(|s| s.name == "no-skill-md").unwrap();
        assert!(no_skill_md.invalid_reason.as_deref().unwrap().contains("SKILL.md"));
    }

    #[test]
    fn scan_missing_store_dir_is_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("does-not-exist");
        assert!(scan_store(&missing).is_empty(), "目录缺失=空仓，不是错误");
    }

    #[test]
    fn scan_skips_dotted_and_nondirectory_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("skills");
        fs::create_dir_all(&store).unwrap();
        make_skill(&store, "beads", SKILL_BODY);
        fs::create_dir_all(store.join(".git")).unwrap();
        fs::write(store.join(".git").join("HEAD"), "ref: x").unwrap();
        fs::write(store.join("stray-file.txt"), "not a dir").unwrap();

        let skills = scan_store(&store);
        assert_eq!(skills.len(), 1, "点前缀目录与非目录不进列表");
        assert_eq!(skills[0].name, "beads");
    }

    #[test]
    fn frontmatter_parses_crlf_and_quoted_values() {
        let fm = parse_frontmatter("---\r\nname: \"my-skill\"\r\ndescription: 'does things'\r\n---\r\nbody")
            .unwrap();
        assert_eq!(fm.name.as_deref(), Some("my-skill"));
        assert_eq!(fm.description.as_deref(), Some("does things"));
        assert!(parse_frontmatter("no fence at all").is_err());
        assert!(parse_frontmatter("---\nname: x\n").is_err(), "围栏未闭合必须报错");
    }

    // ── add_from_local ──────────────────────────────────────────────────────

    #[test]
    fn add_local_copies_whole_directory_into_store() {
        let tmp = tempfile::tempdir().unwrap();
        let src = make_skill(tmp.path(), "my-skill", SKILL_BODY);
        fs::write(src.join("extra.txt"), "attachment").unwrap();
        let store = tmp.path().join("store");

        let name = add_from_local(&src, &store).unwrap();
        assert_eq!(name, "my-skill");
        assert_eq!(
            fs::read_to_string(store.join("my-skill").join(SKILL_FILE)).unwrap(),
            SKILL_BODY,
            "整目录逐字拷贝"
        );
        assert_eq!(fs::read_to_string(store.join("my-skill").join("extra.txt")).unwrap(), "attachment");
        assert_eq!(scan_store(&store)[0].attachments, vec!["extra.txt".to_string()]);
    }

    #[test]
    fn add_local_missing_skill_md_errors_without_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("not-a-skill");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("README.md"), "no skill here").unwrap();
        let store = tmp.path().join("store");

        let err = add_from_local(&src, &store).unwrap_err();
        assert!(err.to_string().contains("SKILL.md"), "报错指名缺 SKILL.md：{err}");
        assert!(!store.exists(), "非法源不得写盘");
    }

    #[test]
    fn add_local_name_collision_errors_and_keeps_original() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let first = make_skill(tmp.path().join("srcs").as_path(), "dup", SKILL_BODY);
        add_from_local(&first, &store).unwrap();

        let second_dir = tmp.path().join("srcs2");
        let second = make_skill(&second_dir, "dup", "---\nname: dup\ndescription: 另一份\n---\n");
        let err = add_from_local(&second, &store).unwrap_err();
        assert!(err.to_string().contains("dup"), "撞名报错指名条目：{err}");
        assert_eq!(
            fs::read_to_string(store.join("dup").join(SKILL_FILE)).unwrap(),
            SKILL_BODY,
            "既有条目原样保留（add 不覆盖）"
        );
    }

    // ── add_from_git / add_from_npx（外部命令分支）──────────────────────────

    #[test]
    fn add_git_reports_missing_git() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let err = with_path_cleared(|| add_from_git("https://example.com/x.git", &store)).unwrap_err();
        assert!(err.to_string().contains("git"), "报错须明说缺 git：{err}");
        assert!(!store.exists());
    }

    #[test]
    fn add_git_single_skill_repo_lands_at_repo_name() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let stub_bin = make_git_stub(tmp.path());
        let added = with_path_prepended(&stub_bin, || {
            add_from_git("https://example.com/stub-repo.git", &store)
        })
        .unwrap();
        assert_eq!(added, vec!["stub-repo".to_string()], "单技能仓以 URL 末段落仓");
        let landed = store.join("stub-repo").join(SKILL_FILE);
        assert!(landed.is_file());
        let text = fs::read_to_string(landed).unwrap();
        assert!(text.contains("name: stub-skill"), "{text}");
    }

    #[test]
    fn add_git_multi_skill_repo_harvests_first_level_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let stub_bin = make_git_stub(tmp.path());
        // stub 模式变量在锁内的闭包里设/撤（with_path_prepended 持 ENV_LOCK）。
        let added = with_path_prepended(&stub_bin, || {
            unsafe { std::env::set_var("SEBAS_STUB_GIT_MODE", "multi") };
            let r = add_from_git("https://example.com/stub-repo.git", &store);
            unsafe { std::env::remove_var("SEBAS_STUB_GIT_MODE") };
            r
        })
        .unwrap();
        assert_eq!(added, vec!["a".to_string(), "b".to_string()]);
        assert!(store.join("a").join(SKILL_FILE).is_file());
        assert!(store.join("b").join(SKILL_FILE).is_file());
    }

    #[test]
    fn add_git_repo_without_skills_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let stub_bin = make_git_stub(tmp.path());
        let err = with_path_prepended(&stub_bin, || {
            unsafe { std::env::set_var("SEBAS_STUB_GIT_MODE", "empty") };
            let r = add_from_git("https://example.com/stub-repo.git", &store);
            unsafe { std::env::remove_var("SEBAS_STUB_GIT_MODE") };
            r
        })
        .unwrap_err();
        assert!(err.to_string().contains("SKILL.md"), "{err}");
        assert!(!store.exists(), "无可收技能时不得写盘");
    }

    #[test]
    fn add_npx_reports_missing_npx() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let err = with_path_cleared(|| add_from_npx("some/skills-pkg", &store)).unwrap_err();
        assert!(err.to_string().contains("npx"), "报错须明说缺 npx：{err}");
    }

    #[test]
    fn add_npx_invokes_cli_with_pkg_and_reports_success() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let stub_bin = make_npx_stub(tmp.path());
        let log = tmp.path().join("npx-argv.log");
        with_path_prepended(&stub_bin, || {
            unsafe { std::env::set_var("SEBAS_STUB_NPX_LOG", log.as_os_str()) };
            let r = add_from_npx("some/skills-pkg", &store);
            unsafe { std::env::remove_var("SEBAS_STUB_NPX_LOG") };
            r
        })
        .unwrap();
        let argv = fs::read_to_string(&log).unwrap();
        assert!(
            argv.contains("skills") && argv.contains("add") && argv.contains("some/skills-pkg"),
            "薄封装应把 pkg 交给 skills CLI：{argv}"
        );
    }

    #[test]
    fn add_npx_command_failure_propagates() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let stub_bin = make_npx_stub(tmp.path());
        let err = with_path_prepended(&stub_bin, || {
            unsafe { std::env::set_var("SEBAS_STUB_NPX_EXIT", "1") };
            let r = add_from_npx("some/skills-pkg", &store);
            unsafe { std::env::remove_var("SEBAS_STUB_NPX_EXIT") };
            r
        })
        .unwrap_err();
        assert!(err.to_string().contains("npx skills add"), "{err}");
    }

    // ── reconcile（spec 四 scenario + 退化/原子性）───────────────────────────

    #[test]
    fn reconcile_overwrites_user_modified_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let backend = tmp.path().join("backend");
        make_skill(&store, "grill-me", SKILL_BODY);
        reconcile(&store, &backend).unwrap();

        // 用户手改 backend 里那份。
        let hand_edited = "---\nname: grill-me\ndescription: 我手改过\n---\n";
        fs::write(backend.join("grill-me").join(SKILL_FILE), hand_edited).unwrap();
        // 仓里那份也改掉（与两边都不同）。
        let new_store_body = "---\nname: grill-me\ndescription: 仓的新版\n---\n";
        fs::write(store.join("grill-me").join(SKILL_FILE), new_store_body).unwrap();

        let report = reconcile(&store, &backend).unwrap();
        assert_eq!(report.overwritten, vec!["grill-me".to_string()]);
        assert!(report.written.is_empty());
        assert_eq!(
            fs::read_to_string(backend.join("grill-me").join(SKILL_FILE)).unwrap(),
            new_store_body,
            "仓 wins：backend 版本被换成仓的新版"
        );
    }

    #[test]
    fn reconcile_leaves_private_entries_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let backend = tmp.path().join("backend");
        make_skill(&store, "beads", SKILL_BODY);
        // 用户手放的私产（仓里没有）。
        let private = make_skill(&backend, "user-byhand", "---\nname: user-byhand\ndescription: 私产\n---\n");
        fs::write(private.join("secret.txt"), "do not touch").unwrap();

        let report = reconcile(&store, &backend).unwrap();
        assert_eq!(report.written, vec!["beads".to_string()]);
        assert_eq!(report.private_ignored, 1, "私产只计数");
        assert_eq!(
            fs::read_to_string(private.join("secret.txt")).unwrap(),
            "do not touch",
            "私产字节不动"
        );
        assert!(!report.overwritten.contains(&"user-byhand".to_string()));
    }

    #[test]
    fn reconcile_deletes_projected_entry_removed_from_store() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let backend = tmp.path().join("backend");
        make_skill(&store, "beads", SKILL_BODY);
        make_skill(&store, "old-skill", "---\nname: old-skill\ndescription: 旧\n---\n");
        reconcile(&store, &backend).unwrap();
        assert!(backend.join("old-skill").is_dir());

        // 仓里删掉 old-skill（模拟 remove），再 sync。
        fs::remove_dir_all(store.join("old-skill")).unwrap();
        let report = reconcile(&store, &backend).unwrap();
        assert_eq!(report.deleted, vec!["old-skill".to_string()]);
        assert!(!backend.join("old-skill").exists(), "已投影条目随仓删除");
        assert!(backend.join("beads").is_dir(), "仍在仓的条目不受影响");
    }

    #[test]
    fn reconcile_without_manifest_projects_without_deleting_then_writes_roster() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let backend = tmp.path().join("backend");
        // 模拟「上次投影过但名册丢了」：backend 里有一份旧投影拷贝，名册不存在。
        make_skill(&backend, "stale", "---\nname: stale\ndescription: 旧投影\n---\n");
        make_skill(&store, "beads", SKILL_BODY);

        let report = reconcile(&store, &backend).unwrap();
        assert_eq!(report.written, vec!["beads".to_string()]);
        assert!(report.deleted.is_empty(), "缺名册退化为只投影不删除");
        assert!(backend.join("stale").is_dir(), "名外全算私产，不动");
        assert_eq!(report.private_ignored, 1);

        // 本轮结束时写入了新名册——下一轮 sync 恢复删除语义。
        let manifest_path = backend.join(PROJECTION_MANIFEST);
        let manifest: ProjectionManifest =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.projected, vec!["beads".to_string()]);

        // 从仓里删掉 beads 后再 sync：名册在场 → 删除语义生效。
        fs::remove_dir_all(store.join("beads")).unwrap();
        let report = reconcile(&store, &backend).unwrap();
        assert_eq!(report.deleted, vec!["beads".to_string()]);
        assert!(!backend.join("beads").exists());
    }

    #[test]
    fn reconcile_rerun_reports_rewrite_but_state_stays_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let backend = tmp.path().join("backend");
        make_skill(&store, "beads", SKILL_BODY);

        let first = reconcile(&store, &backend).unwrap();
        assert_eq!(first.written, vec!["beads".to_string()]);
        let second = reconcile(&store, &backend).unwrap();
        // 报告按「同名=覆盖」的存在性语义判定，不做内容比较（D2：覆盖与否由
        // 「仓 wins」语义决定）——重跑 sync 如实报覆盖；幂等指的是终态与名册。
        assert_eq!(second.overwritten, vec!["beads".to_string()]);
        assert!(
            second.written.is_empty() && second.deleted.is_empty(),
            "重跑 sync 不新增不删除：{second:?}"
        );
        let leftovers: Vec<_> = fs::read_dir(&backend)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "原子写不得留 tmp 残渣：{leftovers:?}");
        let manifest: ProjectionManifest =
            serde_json::from_str(&fs::read_to_string(backend.join(PROJECTION_MANIFEST)).unwrap())
                .unwrap();
        assert_eq!(manifest.projected, vec!["beads".to_string()]);
        assert!(!manifest.store_hash.is_empty(), "指纹字段必须在场");
    }

    #[test]
    fn reconcile_skips_invalid_store_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let backend = tmp.path().join("backend");
        make_skill(&store, "beads", SKILL_BODY);
        make_skill(&store, "broken", "---\nname: broken\n---\n"); // 缺 description

        let report = reconcile(&store, &backend).unwrap();
        assert_eq!(report.written, vec!["beads".to_string()]);
        assert!(!backend.join("broken").exists(), "invalid 条目不往 backend 传播");
    }

    #[test]
    fn reconcile_never_follows_unsafe_roster_names() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let backend = tmp.path().join("backend");
        fs::create_dir_all(&backend).unwrap();
        // 预置一个「越界删除目标」：若 reconcile 顺着 `../escape` 删，它会消失。
        let escape = tmp.path().join("escape");
        fs::create_dir_all(&escape).unwrap();
        fs::write(escape.join("victim.txt"), "survive").unwrap();
        // 手改名册：塞进一个越界名字（不该被顺着删）和一个已消失的正常名字。
        let manifest = r#"{"projected": ["../escape", "gone"], "store_hash": "x"}"#;
        fs::write(backend.join(PROJECTION_MANIFEST), manifest).unwrap();
        make_skill(&store, "beads", SKILL_BODY);

        let report = reconcile(&store, &backend).unwrap();
        assert_eq!(report.written, vec!["beads".to_string()]);
        assert!(report.deleted.is_empty(), "越界名册条目必须被安全阀跳过");
        assert_eq!(
            fs::read_to_string(escape.join("victim.txt")).unwrap(),
            "survive",
            "越界目标安然无恙"
        );
    }

    // ── 方言表 ──────────────────────────────────────────────────────────────

    #[test]
    fn placement_claude_and_codex_hit_identity() {
        let home = tempfile::tempdir().unwrap();
        let claude = placement_for("claude", home.path()).unwrap();
        assert_eq!(claude.backend_kind, "claude");
        assert_eq!(claude.dir, home.path().join(".claude/skills"));
        assert_eq!(claude.convert, Convert::Identity);

        let codex = placement_for("codex", home.path()).unwrap();
        assert_eq!(codex.backend_kind, "codex");
        assert_eq!(codex.dir, home.path().join(".codex/skills"));
        assert_eq!(codex.convert, Convert::Identity);
    }

    #[test]
    fn placement_unknown_kinds_report_none() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(placement_for("gemini", home.path()), None, "gemini 本期不加表行");
        assert_eq!(placement_for("no-such-backend", home.path()), None);
    }
}
