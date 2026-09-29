use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");

    // Embed git branch
    let branch = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_BRANCH={branch}");

    // Embed short git hash
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_HASH={hash}");

    // add-about-build-info 1.1：UTC 编译时刻，分钟精度（YYYY-MM-DD HH:mm）。
    // 诚实边界（design D2）：rerun-if-changed 只盯 .git/HEAD 与 refs/heads，
    // 无新提交的本地增量重编不会重跑本脚本，BUILD_TIME 可能陈旧——由 git
    // hash 区分代码版本，About 照实显示、不做运行时补偿。
    let build_time = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now())
        .format("%Y-%m-%d %H:%M")
        .to_string();
    println!("cargo:rustc-env=BUILD_TIME={build_time}");
}
