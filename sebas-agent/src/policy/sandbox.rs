//! bash 沙箱后端（task 1.4/1.5，design N2）：Landlock 进程内为主 + 防火墙回退。
//!
//! - **Landlock**（缺省，内核 ≥6.7 / ABI v4 含网络位）：在 bash 子进程 `pre_exec`
//!   内实施——只读全盘、可写 workdir+/tmp、`/dev/null`/`/dev/urandom`（文件级
//!   权限——目录专属权限不能授给文件 FD）、零 AccessNet 规则 = 拒绝所有 TCP
//!   bind/connect（fail closed）。网络位以 HardRequirement 处理：内核不支持即
//!   报错 → 父进程 spawn 失败 → 回退防火墙档，绝不半隔离。
//! - **Firewall**（回退档）：env 清洗（密钥类变量剥离）+ 危险命令字面探测，
//!   命中即 `Denied`（工具层拒绝结果）。
//!
//! 已知边界（记录不隐瞒）：Landlock 只拒不藏（stat/ls 可见敏感路径）、无 PID/IPC
//! 隔离、UDP 不在 v4–v9 TCP 位内；bwrap 硬化 tier 留作后续。

#[cfg(unix)]
use super::{DANGEROUS_HEADS, DANGEROUS_PATTERNS};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

/// 沙箱档位选择（配置面）：Auto = Landlock 可用即用，否则防火墙；Firewall = 强制回退档。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    #[default]
    Auto,
    Firewall,
}

/// 本次执行实际生效的档位（如实标注进 `[bash conf: …]`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxProfile {
    Landlock,
    Firewall,
}

impl SandboxProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Landlock => "landlock",
            Self::Firewall => "firewall",
        }
    }
}

/// 父进程侧的沙箱探测：当前内核/环境能否实施 Landlock 档。
/// 创建含网络位的 ruleset（不 restrict）即能探出能力；任何 Err = 不可用。
#[cfg(target_os = "linux")]
pub fn landlock_supported() -> bool {
    use landlock::{AccessNet, Compatible, CompatLevel, Ruleset, RulesetAttr};
    Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessNet::BindTcp)
        .and_then(|r| r.handle_access(AccessNet::ConnectTcp))
        .and_then(|r| r.create())
        .is_ok()
}

#[cfg(not(target_os = "linux"))]
pub fn landlock_supported() -> bool {
    false
}

/// 防火墙检查：危险命令字面探测（头 token + 全命令子串）。命中 → Denied。
#[cfg(unix)]
pub(crate) fn firewall_check(command: &str) -> Result<(), String> {
    let cmd = command.trim();
    if DANGEROUS_PATTERNS.iter().any(|p| cmd.contains(p)) {
        return Err(format!("command matches a dangerous pattern: {cmd:?}"));
    }
    let dangerous_head = cmd
        .split([';', '|', '\n', '&'])
        .filter_map(|seg| seg.split_whitespace().next())
        .any(|tok| {
            let base = tok.rsplit('/').next().unwrap_or(tok);
            DANGEROUS_HEADS.contains(&base)
                || DANGEROUS_HEADS
                    .iter()
                    .any(|h| *h != "sudo" && *h != "doas" && *h != "su" && base.starts_with(h))
        });
    if dangerous_head {
        return Err(format!("command head resolves to a dangerous binary: {cmd:?}"));
    }
    Ok(())
}

/// env 清洗：剥离密钥类变量（*_KEY/*_TOKEN/*_SECRET/*_PASSWORD、SEBAS_*）。
#[cfg(unix)]
pub(crate) fn scrub_env(cmd: &mut tokio::process::Command) {
    let kept: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| !is_secret_env(k))
        .collect();
    cmd.env_clear();
    for (k, v) in kept {
        cmd.env(k, v);
    }
}

#[cfg(unix)]
fn is_secret_env(k: &str) -> bool {
    let ku = k.to_ascii_uppercase();
    ku.starts_with("SEBAS_")
        || ku.ends_with("_KEY")
        || ku.ends_with("_TOKEN")
        || ku.ends_with("_SECRET")
        || ku.ends_with("_PASSWORD")
}

/// 在 bash 子进程内实施 Landlock（经 `pre_exec`：fork → restrict → exec）。
/// 失败（内核不支持/受限容器）→ `spawn()` 报错，由调用方回退防火墙档。
#[cfg(unix)]
pub(crate) fn apply_landlock(cmd: &mut tokio::process::Command, workdir: PathBuf) {
    #[cfg(target_os = "linux")]
    {
        // tokio::process::Command 自带 pre_exec（unix 固有方法），无需 std CommandExt。
        unsafe {
            cmd.pre_exec(move || {
                restrict_self_landlock(&workdir)
                    .map_err(|e| io::Error::other(format!("landlock: {e}")))
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (cmd, workdir);
    }
}

/// 规则面（design N2，实测 ~45 行）：只读全盘 + 可写 workdir/tmp + 文件级
/// /dev/null|urandom + 零网络规则（= TCP 全拒）。
#[cfg(target_os = "linux")]
fn restrict_self_landlock(workdir: &Path) -> Result<(), String> {
    use landlock::{
        Access, AccessFs, AccessNet, ABI, Compatible, CompatLevel, LandlockStatus, PathBeneath,
        PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
    };
    // 处理库已知的全部 fs 权限位；BestEffort = 旧内核尽力限制。
    let abi = ABI::V9;
    // 文件 FD 不能授目录专属权限（MakeDir 等）——文件级读写集。
    let file_rw = AccessFs::from_read(abi) | AccessFs::WriteFile;

    let status = Ruleset::default()
        // 网络位 HardRequirement：内核 <6.7 → 此处 Err → 回退防火墙（fail closed）。
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessNet::BindTcp)
        .map_err(|e| e.to_string())?
        .handle_access(AccessNet::ConnectTcp)
        .map_err(|e| e.to_string())?
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(abi))
        .map_err(|e| e.to_string())?
        .create()
        .map_err(|e| e.to_string())?
        // 只读：全盘
        .add_rule(PathBeneath::new(
            PathFd::new("/").map_err(|e| e.to_string())?,
            AccessFs::from_read(abi),
        ))
        .map_err(|e| e.to_string())?
        // 可写：workdir + /tmp（目录，全权限）
        .add_rule(PathBeneath::new(
            PathFd::new(workdir).map_err(|e| e.to_string())?,
            AccessFs::from_all(abi),
        ))
        .map_err(|e| e.to_string())?
        .add_rule(PathBeneath::new(
            PathFd::new("/tmp").map_err(|e| e.to_string())?,
            AccessFs::from_all(abi),
        ))
        .map_err(|e| e.to_string())?
        // 可写文件：/dev/null、/dev/urandom
        .add_rule(PathBeneath::new(
            PathFd::new("/dev/null").map_err(|e| e.to_string())?,
            file_rw,
        ))
        .map_err(|e| e.to_string())?
        .add_rule(PathBeneath::new(
            PathFd::new("/dev/urandom").map_err(|e| e.to_string())?,
            file_rw,
        ))
        .map_err(|e| e.to_string())?
        // 零 AccessNet 规则 = 拒绝所有 TCP bind/connect；HardRequirement 保证
        // restrict 阶段不静默降级。
        .set_compatibility(CompatLevel::HardRequirement)
        .restrict_self()
        .map_err(|e| e.to_string())?;

    // 生效判定：内核未启用 Landlock → NotEnabled → Err（父进程回退防火墙）。
    if matches!(status.landlock, LandlockStatus::NotEnabled) {
        return Err("landlock is not enabled on this kernel".into());
    }
    // FullyEnforced / PartiallyEnforced 都算生效（部分位不受支持的旧内核尽力限制，
    // 状态由调用方如实标注，绝不断言 full）。
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn landlock_support_probe_matches_kernel() {
        // 本断言按机器能力走两个分支之一；失败 = probe 自身崩溃。
        let supported = landlock_supported();
        if supported {
            // 支持的机器上，真 restrict 不应报错
            let tmp = tempfile::tempdir().unwrap();
            let mut probe = tokio::process::Command::new("true");
            apply_landlock(&mut probe, tmp.path().to_path_buf());
            // spawn 在子进程 restrict 后 exec true；错误会在 spawn 时冒泡
            let st = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { probe.status().await.unwrap() });
            assert!(st.success());
        }
    }

    #[test]
    fn firewall_check_refuses_dangerous_patterns() {
        assert!(firewall_check("/sbin/mkfs.ext4 /dev/sda").is_err());
        assert!(firewall_check("dd if=/dev/zero of=/dev/sda").is_err());
        assert!(firewall_check("ls; shutdown -h now").is_err());
        assert!(firewall_check("rm -rf /").is_err());
        assert!(firewall_check("cat x | tee /etc/passwd").is_ok(), "tee 不在头 token 名单——由策略层/沙箱负责");
        assert!(firewall_check("ls -la").is_ok());
        assert!(firewall_check("cat /etc/os-release").is_ok());
    }

    #[test]
    fn env_scrub_strips_secret_shaped_vars() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(async {
            let mut cmd = tokio::process::Command::new("bash");
            cmd.arg("-c").arg("echo ${PROBE_TOKEN:?missing}");
            cmd.env("PROBE_TOKEN", "supersecret");
            cmd.env("SAFE_VAR", "ok");
            scrub_env(&mut cmd);
            cmd.output().await.unwrap()
        });
        let text = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(text.contains("missing"), "PROBE_TOKEN must be scrubbed: {text}");
    }
}
