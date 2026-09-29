//! Agent Skills - 版本管理示例
//!
//! 这个示例展示了如何使用 VersionManager 进行语义化版本管理和兼容性检查
//!
//! 运行: cargo run --example 35_agent_skills_version

use claude_agent_sdk::skills::{CompatibilityResult, VersionManager};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔢 Agent Skills - 版本管理示例\n");

    // 1. 创建版本管理器
    println!("1️⃣  创建版本管理器");
    let mut version_manager = VersionManager::new();
    println!("   ✅ 版本管理器创建成功");
    println!();

    // 2. 注册技能版本
    println!("2️⃣  注册技能版本");
    version_manager.add_version("data-processor", "1.5.0")?;
    version_manager.add_version("utils", "2.3.1")?;
    version_manager.add_version("logger", "1.0.0")?;
    version_manager.add_version("analytics", "2.0.0-beta")?;
    println!("   ✅ 注册了 4 个技能版本:");
    println!("      - data-processor: 1.5.0");
    println!("      - utils: 2.3.1");
    println!("      - logger: 1.0.0");
    println!("      - analytics: 2.0.0-beta");
    println!();

    // 3. 版本兼容性检查
    println!("3️⃣  版本兼容性检查");
    let checks = vec![
        ("1.5.0", "^1.0.0"),
        ("2.0.0", "^1.0.0"),
        ("1.0.0", ">=1.0.0, <2.0.0"),
        ("2.3.1", "~2.3.0"),
    ];

    for (version, requirement) in checks {
        let result = version_manager.check_requirement(version, requirement);
        println!("   {}", result);
    }
    println!();

    // 4. 查找兼容版本
    println!("4️⃣  查找兼容版本");
    let compatible = version_manager.find_compatible_version("data-processor", "^1.0.0");
    match compatible {
        Some(version) => println!("   ✅ 找到兼容版本: {}", version),
        None => println!("   ❌ 没有找到兼容版本"),
    }

    let incompatible = version_manager.find_compatible_version("data-processor", "^2.0.0");
    match incompatible {
        Some(version) => println!("   ✅ 找到兼容版本: {}", version),
        None => println!("   ❌ 没有找到兼容版本 (requirement: ^2.0.0)"),
    }
    println!();

    // 5. 版本比较
    println!("5️⃣  版本比较");
    let comparisons = vec![
        ("2.0.0", "1.0.0"),
        ("1.0.0", "2.0.0"),
        ("1.0.0", "1.0.0"),
        ("1.2.0", "1.10.0"),
    ];

    for (v1, v2) in comparisons {
        match version_manager.compare_versions(v1, v2)? {
            std::cmp::Ordering::Greater => {
                println!("   📈 {} > {}", v1, v2);
            },
            std::cmp::Ordering::Less => {
                println!("   📉 {} < {}", v1, v2);
            },
            std::cmp::Ordering::Equal => {
                println!("   ⚖️  {} == {}", v1, v2);
            },
        }
    }
    println!();

    // 6. 预发布版本比较
    println!("6️⃣  预发布版本比较");
    let prerelease_checks = vec![
        ("1.0.0-alpha", "1.0.0"),
        ("1.0.0-alpha", "1.0.0-beta"),
        ("1.0.0-alpha.1", "1.0.0-alpha"),
    ];

    for (v1, v2) in prerelease_checks {
        match version_manager.compare_versions(v1, v2)? {
            std::cmp::Ordering::Less => println!("   🔄 {} < {}", v1, v2),
            std::cmp::Ordering::Greater => println!("   🔄 {} > {}", v1, v2),
            std::cmp::Ordering::Equal => println!("   🔄 {} == {}", v1, v2),
        }
    }
    println!();

    // 7. 获取最新版本
    println!("7️⃣  获取最新版本");
    let versions = vec![
        "1.0.0".to_string(),
        "2.0.0".to_string(),
        "1.5.0".to_string(),
        "invalid".to_string(),
        "1.10.0".to_string(),
    ];

    let latest = version_manager.latest_version(&versions);
    match latest {
        Some(version) => println!("   🏆 最新版本: {}", version),
        None => println!("   ❌ 无法确定最新版本"),
    }
    println!();

    // 8. 检查更新
    println!("8️⃣  检查技能更新");
    let update_checks = vec![
        ("data-processor", "1.0.0"),
        ("data-processor", "1.5.0"),
        ("data-processor", "2.0.0"),
        ("nonexistent", "1.0.0"),
    ];

    for (skill_id, current) in update_checks {
        match version_manager.check_update_available(skill_id, current) {
            Ok(has_update) => {
                if has_update {
                    println!("   ⬆️  {} (当前: {}): 有更新可用!", skill_id, current);
                } else {
                    println!("   ✅ {} (当前: {}): 已是最新版", skill_id, current);
                }
            },
            Err(e) => {
                println!("   ❌ {} (当前: {}): {}", skill_id, current, e);
            },
        }
    }
    println!();

    // 9. 依赖验证
    println!("9️⃣  依赖版本验证");
    let dependencies = vec![
        ("utils".to_string(), "^2.0.0".to_string()),
        ("logger".to_string(), "^1.0.0".to_string()),
    ];

    match version_manager.validate_dependencies("my-skill", &dependencies) {
        Ok(_) => println!("   ✅ 所有依赖版本兼容"),
        Err(e) => println!("   ❌ 依赖验证失败: {}", e),
    }
    println!();

    // 10. 不兼容依赖示例
    println!("🔟 不兼容依赖示例");
    let incompatible_deps = vec![("utils".to_string(), "^3.0.0".to_string())];

    match version_manager.validate_dependencies("my-skill", &incompatible_deps) {
        Ok(_) => println!("   ✅ 所有依赖版本兼容"),
        Err(e) => println!("   ❌ 依赖验证失败: {}", e),
    }
    println!();

    // 11. 复杂版本要求
    println!("1️⃣1️⃣  复杂版本要求示例");
    let complex_reqs = vec![
        ("1.5.0", ">=1.2.0, <2.0.0"),
        ("2.0.0", ">=1.0.0, <2.0.0"),
        ("1.2.5", "~1.2.0"),
        ("2.3.0", "^2.0.0"),
        ("1.0.0", "*"),
    ];

    println!("   复杂版本要求检查:");
    for (version, requirement) in complex_reqs {
        let result = version_manager.check_requirement(version, requirement);
        match result {
            CompatibilityResult::Compatible { .. } => {
                println!("      ✅ {} satisfies {}", version, requirement);
            },
            CompatibilityResult::Incompatible { .. } => {
                println!("      ❌ {} does NOT satisfy {}", version, requirement);
            },
            _ => {},
        }
    }
    println!();

    // 12. 版本要求说明
    println!("1️⃣2️⃣  语义化版本要求说明");
    println!("   caret (^):");
    println!("      ^1.2.3  = >=1.2.3 <2.0.0");
    println!("      ^1.2    = >=1.2.0 <2.0.0");
    println!("      ^1      = >=1.0.0 <2.0.0");
    println!();
    println!("   tilde (~):");
    println!("      ~1.2.3  = >=1.2.3 <1.3.0");
    println!("      ~1.2    = >=1.2.0 <1.3.0");
    println!();
    println!("   wildcard (*):");
    println!("      *       = 任何版本");
    println!("      1.*     = >=1.0.0 <2.0.0");
    println!("      1.2.*   = >=1.2.0 <1.3.0");
    println!();
    println!("   比较运算符:");
    println!("      >=1.2.3 : 大于或等于 1.2.3");
    println!("      >1.2.3  : 大于 1.2.3");
    println!("      <=2.0.0 : 小于或等于 2.0.0");
    println!("      <2.0.0  : 小于 2.0.0");
    println!("      ==1.2.3 : 等于 1.2.3");
    println!();

    println!("✨ 示例运行完成!");
    println!("\n💡 版本管理的关键优势:");
    println!("   1. 语义化版本管理 (遵循 SemVer 规范)");
    println!("   2. 灵活的版本要求语法 (^, ~, *, >=, <=)");
    println!("   3. 自动检测兼容性");
    println!("   4. 依赖版本验证");
    println!("   5. 更新检查和版本比较");
    println!("   6. 支持预发布版本 (alpha, beta, rc)");

    Ok(())
}
