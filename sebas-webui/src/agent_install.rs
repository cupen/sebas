//! Agent auto-install（add-agent-auto-install）：从 Settings → Agents 一键把
//! 已知 agent 的 npm 包装进 sebas 私有前缀。
//!
//! 设计要点（design D1–D7，本模块只承载可复用的机制半边，路由在
//! [`crate::routes::agent_install`]）：
//! - **落点私有**（D2）：`<SEBAS_HOME>/agent-tools/<recipe>`，npm
//!   `install --global --prefix <prefix> <pkg>`；零特权、不污染操作员全局。
//! - **配方封闭**（D5）：静态表只认 claude / opencode，不接受任意包名输入。
//! - **同步执行 + 超时**（D3）：`tokio::time::timeout` 包 npm 子进程，
//!   per-recipe in-flight 锁防重入。
//! - **探测复用**：安装后用 [`crate::agent_kinds::discover_agent`] 同一口径
//!   复核可达性并取版本，不另造探测实现。

use crate::agent_kinds::{AgentKindSource, discover_agent};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 同步安装的超时上限（design D3）：npm 装两个 CLI 包通常 <2min，300s 是
/// 宽裕但确定的上限；超时即如实失败（重试幂等，D6）。
pub const INSTALL_TIMEOUT_SECS: u64 = 300;

/// npm 在场探测的超时（`npm --version` 应瞬回；挂起即判不可用）。
const NPM_PROBE_TIMEOUT_SECS: u64 = 10;

/// 一条内置安装配方（design D5）：请求只认 `recipe` 名，包名/bin 名硬编码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipe {
    /// 配方名，也是 agents store 的 id（claude / opencode）。
    pub name: &'static str,
    /// 官方 npm 包名。
    pub package: &'static str,
    /// 包提供的可执行文件名（bin shim 基名）。
    pub bin: &'static str,
    /// launch 驱动标签（封闭 `claude` | `acp`）。
    pub driver: &'static str,
    /// ACP 形态的启动参数（claude 驱动为空）。
    pub args: &'static [&'static str],
}

/// 封闭配方表（design D5）：清单之外的 recipe 一律 typed 拒绝，不 spawn。
pub const RECIPES: &[Recipe] = &[
    Recipe {
        name: "claude",
        package: "@anthropic-ai/claude-code",
        bin: "claude",
        driver: "claude",
        args: &[],
    },
    Recipe {
        name: "opencode",
        package: "opencode-ai",
        bin: "opencode",
        driver: "acp",
        // 与前端表单 opencode 形态同口径：args 首词 `acp`。
        args: &["acp"],
    },
];

/// 按名查配方；未知名 → None（调用方 typed 拒绝）。
pub fn recipe(name: &str) -> Option<&'static Recipe> {
    RECIPES.iter().find(|r| r.name == name)
}

/// 私有安装前缀：`<SEBAS_HOME>/agent-tools/<recipe>`（design D2）。
pub fn install_prefix(recipe: &str) -> PathBuf {
    install_prefix_under(&sebas_domain::state_paths::sebas_home(), recipe)
}

/// 前缀推导的纯函数半边（显式 home，便于无 env 单测）。
pub fn install_prefix_under(home: &Path, recipe: &str) -> PathBuf {
    home.join("agent-tools").join(recipe)
}

/// 配方 bin 在私有前缀里的落点（design D2 平台分支）：unix `<prefix>/bin/<bin>`，
/// windows `<prefix>/<bin>.cmd`（npm shim 形态）。
pub fn bin_path(prefix: &Path, bin: &str) -> PathBuf {
    #[cfg(windows)]
    {
        prefix.join(format!("{bin}.cmd"))
    }
    #[cfg(not(windows))]
    {
        prefix.join("bin").join(bin)
    }
}

/// 配方 → 私有 bin 绝对路径（安装应答与 store 行共用）。
pub fn recipe_bin_path(recipe: &'static Recipe) -> PathBuf {
    bin_path(&install_prefix(recipe.name), recipe.bin)
}

/// 配方 → 标准 agents store 行定义（design D4）：path 指私有 bin 绝对路径，
/// sessions_dir/work_dir 留空（缺省），args 按配方（opencode 首词 `acp`）。
pub fn recipe_agent_definition(recipe: &'static Recipe, bin_abs: &Path) -> serde_json::Value {
    let path = bin_abs.to_string_lossy().to_string();
    let mut def = serde_json::Map::new();
    def.insert("driver".into(), serde_json::json!(recipe.driver));
    def.insert("path".into(), serde_json::json!(path));
    if !recipe.args.is_empty() {
        def.insert("args".into(), serde_json::json!(recipe.args));
    }
    serde_json::Value::Object(def)
}

/// 安装后复核：用目录探测同一口径（[`discover_agent`]）对私有 bin 探测，
/// 返回 `(reachable, version, cause)`。
pub async fn probe_recipe_bin(
    recipe: &'static Recipe,
    bin_abs: &Path,
) -> (bool, Option<String>, Option<String>) {
    let mut command = vec![bin_abs.to_string_lossy().to_string()];
    command.extend(recipe.args.iter().map(|a| (*a).to_string()));
    let info = discover_agent(&AgentKindSource {
        slug: recipe.name.to_string(),
        command,
        driver: recipe.driver.to_string(),
        display: None,
    })
    .await;
    (info.reachable, info.version, info.cause)
}

// ── npm 执行（design D3/D7）──────────────────────────────────────────────────

/// 安装失败的诚实分类（design D7）：错误形状决定 HTTP 状态与文案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallFailure {
    /// PATH 上找不到 npm（或 `npm --version` 不可达）。
    NpmMissing,
    /// npm 进程超过超时上限（被杀死）。
    Timeout,
    /// npm 非零退出；携带 stderr 尾部摘要。
    Exit { code: Option<i32>, stderr: String },
    /// 无法启动 npm 进程。
    Spawn(String),
}

impl InstallFailure {
    /// 面向操作员的最小指引文案（design D7：npm 缺失含安装 Node.js 指引）。
    pub fn message(&self) -> String {
        match self {
            InstallFailure::NpmMissing => {
                "本机 PATH 上找不到可用的 npm：请先安装 Node.js（随附 npm）后重试".to_string()
            }
            InstallFailure::Timeout => {
                format!("npm 安装超时（超过 {INSTALL_TIMEOUT_SECS} 秒）——网络慢时可重试，重装幂等")
            }
            InstallFailure::Exit { code, stderr } => {
                let code = code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "信号终止".to_string());
                if stderr.is_empty() {
                    format!("npm 安装失败（退出码 {code}）")
                } else {
                    format!("npm 安装失败（退出码 {code}）：{stderr}")
                }
            }
            InstallFailure::Spawn(cause) => format!("无法启动 npm：{cause}"),
        }
    }
}

/// stderr 尾部摘要（design D7）：取末若干行、限长，避免整段 npm 噪声。
fn stderr_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail: Vec<&str> = lines.iter().rev().take(20).rev().copied().collect();
    let out = tail.join("\n");
    const MAX: usize = 2000;
    if out.len() <= MAX {
        return out;
    }
    // 省略号 `…` 本身占 3 字节——截断后总长仍不超过 MAX。
    let keep = MAX.saturating_sub(3);
    let mut start = out.len() - keep;
    // 字节边界可能切进多字节字符：从合法 char 边界开始截断。
    while start < out.len() && !out.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &out[start..])
}

/// 解析 PATH 上的可执行程序（语义抄 `sebas::skills::resolve_command`：裸名扫
/// PATH，Windows 按 PATHEXT 展开——`npm` 实际是 `npm.cmd`）。
fn resolve_program(program: &str) -> Option<PathBuf> {
    if program.trim().is_empty() {
        return None;
    }
    let p = Path::new(program);
    if program.contains('/') || program.contains('\\') {
        return is_executable_file(p).then(|| p.to_path_buf());
    }
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        for candidate in program_candidates(program) {
            let full = dir.join(&candidate);
            if is_executable_file(&full) {
                return Some(full);
            }
        }
    }
    None
}

fn program_candidates(program: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        if Path::new(program).extension().is_some() {
            return vec![program.to_string()];
        }
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        exts.split(';')
            .filter(|e| e.starts_with('.'))
            .map(|e| format!("{program}{e}"))
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![program.to_string()]
    }
}

fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.is_file()
            && std::fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// npm 在场探测：`npm --version` 成功即在场（design D7；task 1.2）。
pub async fn npm_available() -> bool {
    match resolve_program("npm") {
        Some(npm) => npm_available_at(&npm, Duration::from_secs(NPM_PROBE_TIMEOUT_SECS)).await,
        None => false,
    }
}

/// [`npm_available`] 的显式程序形态（单测注入 fake npm，避免动全局 PATH）。
async fn npm_available_at(npm: &Path, timeout: Duration) -> bool {
    // `npm --version` 的 status 探测：ETXTBSY 重试与其它 exec 点同口径
    // （多线程下刚写出的 shim 会被继承的写 fd 短暂占住）。
    let fut = crate::agent_kinds::output_with_etxtbsy_retry(|| {
        let mut cmd = tokio::process::Command::new(npm);
        cmd.arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        cmd
    });
    matches!(
        tokio::time::timeout(timeout, fut).await,
        Ok(Ok(out)) if out.status.success()
    )
}

/// 执行 npm 安装（design D2/D3）：`npm install --global --prefix <prefix> <pkg>`。
pub async fn run_npm_install(prefix: &Path, package: &str) -> Result<(), InstallFailure> {
    run_npm_install_timeout(prefix, package, Duration::from_secs(INSTALL_TIMEOUT_SECS)).await
}

/// [`run_npm_install`] 的可注入超时形态（单测用短超时驱动挂起剧本）。
pub async fn run_npm_install_timeout(
    prefix: &Path,
    package: &str,
    timeout: Duration,
) -> Result<(), InstallFailure> {
    let npm = resolve_program("npm").ok_or(InstallFailure::NpmMissing)?;
    run_npm_install_at(&npm, prefix, package, timeout).await
}

/// npm 执行的程序可注入形态（单测直接给 fake npm 路径，不依赖全局 PATH）。
async fn run_npm_install_at(
    npm: &Path,
    prefix: &Path,
    package: &str,
    timeout: Duration,
) -> Result<(), InstallFailure> {
    let fut = crate::agent_kinds::output_with_etxtbsy_retry(|| {
        let mut cmd = tokio::process::Command::new(npm);
        cmd.arg("install")
            .arg("--global")
            .arg("--prefix")
            .arg(prefix)
            .arg(package)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // 超时取消时 drop 子进程 → 杀进程（不留挂起的 npm）。
            .kill_on_drop(true);
        cmd
    });
    match tokio::time::timeout(timeout, fut).await {
        Err(_) => Err(InstallFailure::Timeout),
        Ok(Err(e)) => Err(InstallFailure::Spawn(e.to_string())),
        Ok(Ok(out)) if out.status.success() => Ok(()),
        Ok(Ok(out)) => Err(InstallFailure::Exit {
            code: out.status.code(),
            stderr: stderr_tail(&out.stderr),
        }),
    }
}

// ── per-recipe in-flight 锁（design D3）──────────────────────────────────────

fn in_flight() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 同一配方的安装守卫：拿不到即并发冲突（409）。drop 时自动释放名额。
#[must_use = "guard 掉出作用域即释放 in-flight 名额"]
pub struct InstallGuard {
    recipe: String,
}

impl Drop for InstallGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = in_flight().lock() {
            set.remove(&self.recipe);
        }
    }
}

/// 尝试占用某配方的安装名额；已被占用 → None（调用方 409）。
pub fn try_acquire(recipe: &str) -> Option<InstallGuard> {
    let mut set = in_flight().lock().unwrap_or_else(|e| e.into_inner());
    if set.contains(recipe) {
        return None;
    }
    set.insert(recipe.to_string());
    Some(InstallGuard {
        recipe: recipe.to_string(),
    })
}

// ── 建行判定（design D4）─────────────────────────────────────────────────────

/// 按 spawn 解析口径（agents store ∪ config 注册表，config 侧扣除墓碑 id）
/// 判定该 recipe id 是否**已有定义**；无定义才建行（design D4）。
///
/// 与 `api::agent_kinds` 的 union 同构：store 行永远算定义；config 条目在未被
/// 删除墓碑覆盖时算定义。
pub fn has_existing_definition(
    store_ids: &HashSet<String>,
    config_ids: &HashSet<String>,
    deleted_ids: &HashSet<String>,
    recipe: &str,
) -> bool {
    if store_ids.contains(recipe) {
        return true;
    }
    config_ids.contains(recipe) && !deleted_ids.contains(recipe)
}

/// 从 agents 快照提取 store 行 id 集（与 `routes::stored_agents` 同口径）。
pub fn store_ids_from_snapshot(snapshot: &serde_json::Value) -> HashSet<String> {
    snapshot
        .get("agents")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| r.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 从 agents 快照提取墓碑 id 集（UI 删除 config 种子后的墓碑）。
pub fn deleted_ids_from_snapshot(snapshot: &serde_json::Value) -> HashSet<String> {
    snapshot
        .get("deleted_ids")
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    //! 纯单元测试（不触碰进程全局 env）：前缀派生、配方表形状、npm 执行三剧本
    //! （fake npm 经**显式路径**注入，不动 PATH）、in-flight 锁、建行判定。
    //!
    //! 端到端路由旅程（`POST /api/agents/install` 的成功/409/未知 recipe/目录
    //! 联动）在 `tests/agent_install_endpoint_test.rs`——那里要钉 SEBAS_HOME 与
    //! PATH 并装进程级引擎，单独进程隔离（同 `agent_store_probe_test.rs` 的
    //! 理由：全局引擎/env 一装，同进程其它测试会互相污染）。

    use super::*;

    // ── 1.1 前缀派生 / 配方表 ────────────────────────────────────────────────

    #[test]
    fn install_prefix_derives_under_the_home() {
        let home = Path::new("/home/op/.sebas");
        let prefix = install_prefix_under(home, "claude");
        assert_eq!(prefix, home.join("agent-tools").join("claude"));
        assert!(prefix.starts_with(home), "{prefix:?}");
    }

    #[test]
    fn bin_path_has_platform_shape() {
        let prefix = Path::new("/tmp/agent-tools/claude");
        let bin = bin_path(prefix, "claude");
        #[cfg(unix)]
        assert_eq!(bin, prefix.join("bin").join("claude"));
        #[cfg(windows)]
        assert_eq!(bin, prefix.join("claude.cmd"));
    }

    #[test]
    fn unknown_recipe_is_rejected_without_a_match() {
        assert!(recipe("claude").is_some());
        assert!(recipe("opencode").is_some());
        assert!(recipe("malicious-pkg").is_none());
        assert!(recipe("").is_none());
        assert_eq!(RECIPES.len(), 2, "封闭配方表只有两条");
    }

    #[test]
    fn recipe_templates_have_the_expected_shape() {
        let bin = Path::new("/home/op/.sebas/agent-tools/claude/bin/claude");

        let claude = recipe_agent_definition(recipe("claude").unwrap(), bin);
        assert_eq!(claude["driver"], "claude");
        assert_eq!(claude["path"], bin.to_string_lossy().as_ref());
        assert!(claude.get("args").is_none(), "claude 驱动无 args: {claude}");
        assert!(claude.get("sessions_dir").is_none(), "留空走默认: {claude}");
        assert!(claude.get("work_dir").is_none(), "留空走默认: {claude}");

        let opencode_bin = Path::new("/home/op/.sebas/agent-tools/opencode/bin/opencode");
        let oc = recipe_agent_definition(recipe("opencode").unwrap(), opencode_bin);
        assert_eq!(oc["driver"], "acp");
        assert_eq!(oc["path"], opencode_bin.to_string_lossy().as_ref());
        assert_eq!(oc["args"], serde_json::json!(["acp"]), "首词 acp: {oc}");
    }

    #[test]
    fn recipe_package_names_are_the_official_ones() {
        assert_eq!(
            recipe("claude").unwrap().package,
            "@anthropic-ai/claude-code"
        );
        assert_eq!(recipe("opencode").unwrap().package, "opencode-ai");
        assert_eq!(recipe("claude").unwrap().bin, "claude");
        assert_eq!(recipe("opencode").unwrap().bin, "opencode");
    }

    /// 配方定义必须过 core 侧 store 校验（`validate_agent_definition`）——否则
    /// 安装成功但建行被拒。这条把两配方的定义钉在真实校验器上。
    #[test]
    fn recipe_definitions_pass_the_store_validator() {
        for r in RECIPES {
            let bin = Path::new("/home/op/.sebas/agent-tools/x/bin/x");
            let def = recipe_agent_definition(r, bin);
            let obj = def.as_object().expect("definition object");
            let validated = sebas_dispatch::state_store::validate_agent_definition(obj);
            let parsed = validated.unwrap_or_else(|e| panic!("recipe {} rejected: {e}", r.name));
            assert_eq!(parsed.driver, r.driver, "recipe {}", r.name);
            assert_eq!(parsed.args, r.args, "recipe {}", r.name);
            assert!(parsed.sessions_dir.is_none(), "留空走默认");
            assert!(parsed.work_dir.is_none(), "留空走默认");
        }
    }

    // ── 1.2 npm 执行三剧本（fake npm，显式路径注入）─────────────────────────

    /// 写一个 fake `npm`（unix sh 脚本）：剧本与日志路径**烘焙进脚本**（不读
    /// 进程 env——模块单测因此零全局副作用、可并行）。返回脚本路径。
    #[cfg(unix)]
    fn fake_npm(dir: &Path, mode: &str, log: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("npm");
        let body = format!(
            r#"#!/bin/sh
printf '%s\n' "$@" > "{log}"
case "{mode}" in
  ok)
    # argv: install --global --prefix <prefix> <package>
    prefix="$4"
    pkg="$5"
    case "$pkg" in
      "@anthropic-ai/claude-code") name=claude ;;
      "opencode-ai") name=opencode ;;
      *) name=pkg ;;
    esac
    mkdir -p "$prefix/bin"
    printf '#!/bin/sh\necho 9.9.9\n' > "$prefix/bin/$name"
    chmod +x "$prefix/bin/$name"
    exit 0
    ;;
  fail)
    echo "npm ERR! network unreachable" >&2
    echo "npm ERR! registry refused" >&2
    exit 7
    ;;
  hang)
    sleep 60
    exit 0
    ;;
esac
exit 0
"#,
            log = log.display(),
            mode = mode,
        );
        std::fs::write(&script, body).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_npm_success_runs_expected_argv_and_probes_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        let log = tmp.path().join("argv.log");
        let npm = fake_npm(tmp.path(), "ok", &log);
        let recipe = recipe("claude").unwrap();

        assert!(npm_available_at(&npm, Duration::from_secs(5)).await);
        run_npm_install_at(&npm, &prefix, recipe.package, Duration::from_secs(5))
            .await
            .expect("fake npm ok script installs successfully");

        let argv: Vec<String> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(argv[0], "install", "argv: {argv:?}");
        assert!(argv.iter().any(|a| a == "--global"), "argv: {argv:?}");
        assert!(argv.iter().any(|a| a == "--prefix"), "argv: {argv:?}");
        assert!(
            argv.iter()
                .any(|a| a == &prefix.to_string_lossy().to_string()),
            "prefix must ride argv: {argv:?}"
        );
        assert!(
            argv.iter().any(|a| a == "@anthropic-ai/claude-code"),
            "package must ride argv: {argv:?}"
        );

        // 假 bin 就位 → 探测口径复核可达 + 版本。
        let bin = bin_path(&prefix, recipe.bin);
        assert!(bin.is_file(), "fake npm wrote the bin: {bin:?}");
        let (reachable, version, cause) = probe_recipe_bin(recipe, &bin).await;
        assert!(
            reachable,
            "installed bin probes reachable (cause={cause:?})"
        );
        assert_eq!(version.as_deref(), Some("9.9.9"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_npm_failure_carries_stderr_tail_and_creates_no_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        let log = tmp.path().join("argv.log");
        let npm = fake_npm(tmp.path(), "fail", &log);
        let recipe = recipe("opencode").unwrap();

        let outcome =
            run_npm_install_at(&npm, &prefix, recipe.package, Duration::from_secs(5)).await;

        match outcome {
            Err(InstallFailure::Exit { code, stderr }) => {
                assert_eq!(code, Some(7));
                assert!(
                    stderr.contains("network unreachable"),
                    "stderr tail: {stderr}"
                );
                assert!(stderr.contains("registry refused"), "stderr tail: {stderr}");
            }
            other => panic!("expected Exit failure, got {other:?}"),
        }
        let msg = InstallFailure::Exit {
            code: Some(7),
            stderr: "npm ERR! network unreachable".into(),
        }
        .message();
        assert!(
            msg.contains("npm 安装失败") && msg.contains("network"),
            "{msg}"
        );
        assert!(!bin_path(&prefix, recipe.bin).exists(), "失败不落假 bin");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn fake_npm_hang_hits_the_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        let log = tmp.path().join("argv.log");
        let npm = fake_npm(tmp.path(), "hang", &log);
        let recipe = recipe("claude").unwrap();

        let outcome =
            run_npm_install_at(&npm, &prefix, recipe.package, Duration::from_millis(300)).await;

        assert_eq!(outcome, Err(InstallFailure::Timeout));
        assert!(InstallFailure::Timeout.message().contains("超时"));
    }

    #[test]
    fn missing_npm_is_reported_honestly() {
        // 显式指向不存在路径：诚实 NpmMissing，不 panic。
        let missing = Path::new("/definitely/not/here/npm-xyz-12345");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (available, outcome) = rt.block_on(async {
            let available = npm_available_at(missing, Duration::from_millis(200)).await;
            let outcome = run_npm_install_at(
                missing,
                Path::new("/tmp/prefix"),
                "@anthropic-ai/claude-code",
                Duration::from_millis(200),
            )
            .await;
            (available, outcome)
        });
        assert!(!available);
        // run_npm_install_at 显式路径：spawn 失败 → Spawn（npm 缺失的 resolve
        // 分支由 run_npm_install_timeout 承担）。
        assert!(
            matches!(outcome, Err(InstallFailure::Spawn(_))),
            "{outcome:?}"
        );
        let msg = InstallFailure::NpmMissing.message();
        assert!(msg.contains("Node.js"), "最小指引点名 Node.js: {msg}");
    }

    // ── 1.2 in-flight 锁 ────────────────────────────────────────────────────

    #[test]
    fn concurrent_same_recipe_install_is_refused() {
        let first = try_acquire("claude").expect("first acquire wins");
        assert!(try_acquire("claude").is_none(), "同配方并发 → 拒绝");
        // 不同配方互不阻塞。
        let other = try_acquire("opencode").expect("other recipe independent");
        drop(first);
        assert!(try_acquire("claude").is_some(), "释放后可再占用");
        drop(other);
    }

    // ── 1.3 建行判定三情形 ──────────────────────────────────────────────────

    fn ids(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn seed_decision_covers_the_three_cases() {
        // 全新建行：store 与 config 均无该 id。
        assert!(!has_existing_definition(
            &ids(&[]),
            &ids(&[]),
            &ids(&[]),
            "opencode"
        ));

        // store 已有行 → 不动。
        assert!(has_existing_definition(
            &ids(&["opencode"]),
            &ids(&[]),
            &ids(&[]),
            "opencode"
        ));

        // config 种子条目 → 受尊重（不建行）。
        assert!(has_existing_definition(
            &ids(&[]),
            &ids(&["opencode"]),
            &ids(&[]),
            "opencode"
        ));

        // config 条目被 UI 删除（墓碑）→ 该 id 无定义，重装可再建行。
        assert!(!has_existing_definition(
            &ids(&[]),
            &ids(&["opencode"]),
            &ids(&["opencode"]),
            "opencode"
        ));
    }

    #[test]
    fn snapshot_id_extraction_matches_the_store_shape() {
        let snapshot = serde_json::json!({
            "agents": [
                {"id": "claude", "driver": "claude"},
                {"id": "opencode", "driver": "acp"}
            ],
            "deleted_ids": ["gone"]
        });
        assert_eq!(
            store_ids_from_snapshot(&snapshot),
            ids(&["claude", "opencode"])
        );
        assert_eq!(deleted_ids_from_snapshot(&snapshot), ids(&["gone"]));
        assert!(store_ids_from_snapshot(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn stderr_tail_keeps_the_last_lines_and_caps_length() {
        let long = (1..=50)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = stderr_tail(long.as_bytes());
        assert!(tail.contains("line-50"), "tail keeps newest: {tail}");
        assert!(!tail.contains("line-1\n"), "old lines dropped: {tail}");
        assert!(tail.len() <= 2000, "capped: {}", tail.len());

        let long_single = "x".repeat(5000);
        let tail = stderr_tail(long_single.as_bytes());
        assert!(tail.starts_with('…'));
        assert!(tail.len() <= 2000);
    }

    // ── 2.1 路由分支（未知 recipe / 缺字段 / 并发 409）────────────────────────
    //
    // 这些分支在 npm 探测**之前**返回，故用 FakeBackend 装配的 router 即可覆盖，
    // 零 env 改动、零子进程（真 npm 全程不触）。

    fn fake_app() -> axum::Router {
        crate::server::build_router(
            std::sync::Arc::new(crate::session_backend::FakeBackend::new()),
            crate::models::RouterInfo::default(),
            sebas_feishu::cards::CardConfig::default(),
        )
    }

    async fn post_install(
        app: &axum::Router,
        body: serde_json::Value,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/agents/install")
                    .header("origin", "http://127.0.0.1:9877")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn unknown_recipe_is_a_typed_400() {
        let app = fake_app();
        let (status, v) = post_install(&app, serde_json::json!({"recipe": "left-pad"})).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{v}");
        assert!(
            v["error"]
                .as_str()
                .unwrap_or_default()
                .contains("未知 recipe"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn missing_recipe_field_is_a_typed_400() {
        let app = fake_app();
        let (status, v) = post_install(&app, serde_json::json!({})).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "{v}");
        assert!(
            v["error"].as_str().unwrap_or_default().contains("recipe"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn concurrent_same_recipe_is_a_409() {
        let app = fake_app();
        let held = try_acquire("claude").expect("hold claude slot");
        let (status, v) = post_install(&app, serde_json::json!({"recipe": "claude"})).await;
        assert_eq!(status, axum::http::StatusCode::CONFLICT, "{v}");
        assert!(
            v["error"].as_str().unwrap_or_default().contains("正在安装"),
            "{v}"
        );
        drop(held);
    }

    // ── 2.2 建行 → 目录快照 → 探测（2.2 的机制半边）──────────────────────────
    //
    // 装完建行走 `state_mutate("agents")` → `AgentRow::snapshot_value` →
    // catalog union 的同一链路。这里用进程级内存引擎（`install_fresh`，全局
    // 串行锁）真跑一遍，断言新行落库且能被目录 union 探测源读回（reachable
    // 取决于私有 bin 是否存在——本测试只钉「行落库 + 快照可见 + 定义形状」）。

    #[cfg(unix)]
    #[tokio::test]
    async fn seeded_row_round_trips_through_the_agents_snapshot() {
        use std::os::unix::fs::PermissionsExt;
        let _engine = sebas_dispatch::test_engine::install_fresh();
        let engine = sebas_dispatch::state_store::engine().expect("fixture engine");

        // 落一个假的私有 bin（可执行），模拟装完后探测应可达。
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("agent-tools").join("opencode");
        let bin = bin_path(&prefix, "opencode");
        std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
        std::fs::write(&bin, "#!/bin/sh\necho 2.0.8\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

        let def = recipe_agent_definition(recipe("opencode").unwrap(), &bin);
        sebas_dispatch::state_store::agents_mutation(
            engine,
            &serde_json::json!({"op": "put", "id": "opencode", "agent": def}),
        )
        .await
        .expect("seed row");

        // 快照形状与 InProcessBackend/core channel 共用（AgentRow::snapshot_value）。
        let rows = engine.load_agents().await.expect("agents load");
        let snapshot = sebas_models::agent::AgentRow::snapshot_value(&rows);
        let store_ids = store_ids_from_snapshot(&snapshot);
        assert!(store_ids.contains("opencode"), "row lands in the snapshot");

        // catalog union 的 store 半边：source_from_store_item → discover_agent。
        let item = snapshot["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "opencode")
            .unwrap();
        let src = crate::agent_kinds::source_from_store_item(item).expect("probe source");
        assert_eq!(src.command, vec![bin.to_string_lossy().to_string(), "acp".to_string()]);
        let info = crate::agent_kinds::discover_agent(&src).await;
        assert!(info.reachable, "private bin is reachable: {info:?}");
        assert_eq!(info.version.as_deref(), Some("2.0.8"));

        // 再装一次（幂等）：store 已有该 id → has_existing_definition = true。
        assert!(has_existing_definition(
            &store_ids,
            &HashSet::new(),
            &HashSet::new(),
            "opencode"
        ));
    }
}
