//! Agent Skills - 依赖解析示例
//!
//! 这个示例展示了如何使用 DependencyResolver 来管理技能之间的依赖关系
//!
//! 运行: cargo run --example 34_agent_skills_dependency

use claude_agent_sdk::skills::{
    Dependency, DependencyResolver, ResolutionResult, SkillMetadata, SkillPackage,
};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔗 Agent Skills - 依赖解析示例\n");

    // 1. 创建多个技能包,模拟真实的依赖关系
    println!("1️⃣  创建技能包及其依赖关系");

    let skill_data = SkillPackage {
        metadata: SkillMetadata {
            id: "data-processor".to_string(),
            name: "Data Processor".to_string(),
            description: "处理和分析数据的技能".to_string(),
            version: "1.0.0".to_string(),
            author: Some("Claude SDK Team".to_string()),
            dependencies: vec!["utils".to_string()],
            tags: vec!["data".to_string()],
        },
        instructions: "你是一个专业的数据处理助手。".to_string(),
        scripts: vec![],
        resources: Default::default(),
    };

    let skill_utils = SkillPackage {
        metadata: SkillMetadata {
            id: "utils".to_string(),
            name: "Utility Functions".to_string(),
            description: "通用工具函数".to_string(),
            version: "1.2.0".to_string(),
            author: Some("Claude SDK Team".to_string()),
            dependencies: vec!["logger".to_string()],
            tags: vec!["utility".to_string()],
        },
        instructions: "提供通用工具函数。".to_string(),
        scripts: vec![],
        resources: Default::default(),
    };

    let skill_logger = SkillPackage {
        metadata: SkillMetadata {
            id: "logger".to_string(),
            name: "Logger".to_string(),
            description: "日志记录工具".to_string(),
            version: "2.0.0".to_string(),
            author: Some("Claude SDK Team".to_string()),
            dependencies: vec![],
            tags: vec!["logging".to_string()],
        },
        instructions: "提供日志记录功能。".to_string(),
        scripts: vec![],
        resources: Default::default(),
    };

    let skill_analytics = SkillPackage {
        metadata: SkillMetadata {
            id: "analytics".to_string(),
            name: "Analytics".to_string(),
            description: "数据分析工具".to_string(),
            version: "1.5.0".to_string(),
            author: Some("Claude SDK Team".to_string()),
            dependencies: vec!["data-processor".to_string(), "utils".to_string()],
            tags: vec!["analytics".to_string()],
        },
        instructions: "提供数据分析功能。".to_string(),
        scripts: vec![],
        resources: Default::default(),
    };

    println!("   ✅ 创建了 4 个技能包:");
    println!("      - data-processor (依赖: utils)");
    println!("      - utils (依赖: logger)");
    println!("      - logger (无依赖)");
    println!("      - analytics (依赖: data-processor, utils)");
    println!();

    // 2. 创建依赖解析器并注册所有技能
    println!("2️⃣  创建依赖解析器并注册技能");
    let mut resolver = DependencyResolver::new();
    resolver.add_skill("data-processor", "1.0.0");
    resolver.add_skill("utils", "1.2.0");
    resolver.add_skill("logger", "2.0.0");
    resolver.add_skill("analytics", "1.5.0");
    println!("   ✅ 注册了 4 个技能");
    println!();

    // 3. 构建依赖关系图
    println!("3️⃣  构建依赖关系图");
    let mut skills_graph = HashMap::new();

    // data-processor -> utils
    skills_graph.insert("data-processor".to_string(), vec![Dependency::new("utils")]);

    // utils -> logger
    skills_graph.insert("utils".to_string(), vec![Dependency::new("logger")]);

    // logger -> (no dependencies)
    skills_graph.insert("logger".to_string(), vec![]);

    // analytics -> data-processor, utils
    skills_graph.insert(
        "analytics".to_string(),
        vec![Dependency::new("data-processor"), Dependency::new("utils")],
    );

    println!("   ✅ 依赖关系图构建完成");
    println!();

    // 4. 解析依赖关系
    println!("4️⃣  解析依赖关系");
    match resolver.resolve(&skills_graph) {
        ResolutionResult::Resolved { load_order } => {
            println!("   ✅ 依赖解析成功!");
            println!("\n   推荐加载顺序:");
            for (i, skill_id) in load_order.iter().enumerate() {
                println!("      {}. {}", i + 1, skill_id);
            }
            println!();
            println!("   说明: 按此顺序加载可以确保所有依赖都先于依赖它们的技能加载。");
        },
        ResolutionResult::CircularDependency { cycle } => {
            println!("   ❌ 检测到循环依赖:");
            println!("      {:?}", cycle);
        },
        ResolutionResult::MissingDependencies { missing } => {
            println!("   ❌ 缺少以下依赖:");
            for dep in &missing {
                println!("      - {}", dep);
            }
        },
    }
    println!();

    // 5. 演示循环依赖检测
    println!("5️⃣  演示循环依赖检测");
    let mut circular_graph = HashMap::new();
    circular_graph.insert("skill-a".to_string(), vec![Dependency::new("skill-b")]);
    circular_graph.insert("skill-b".to_string(), vec![Dependency::new("skill-c")]);
    circular_graph.insert("skill-c".to_string(), vec![Dependency::new("skill-a")]);

    let mut resolver_circular = DependencyResolver::new();
    resolver_circular.add_skill("skill-a", "1.0.0");
    resolver_circular.add_skill("skill-b", "1.0.0");
    resolver_circular.add_skill("skill-c", "1.0.0");

    match resolver_circular.resolve(&circular_graph) {
        ResolutionResult::CircularDependency { cycle } => {
            println!("   ✅ 成功检测到循环依赖:");
            println!("      循环路径: {}", cycle.join(" -> "));
        },
        _ => {
            println!("   ❌ 未能检测到循环依赖");
        },
    }
    println!();

    // 6. 演示缺少依赖检测
    println!("6️⃣  演示缺少依赖检测");
    let mut incomplete_graph = HashMap::new();
    incomplete_graph.insert("my-skill".to_string(), vec![Dependency::new("missing-dep")]);

    let mut resolver_incomplete = DependencyResolver::new();
    resolver_incomplete.add_skill("my-skill", "1.0.0");
    // 故意不添加 missing-dep

    match resolver_incomplete.resolve(&incomplete_graph) {
        ResolutionResult::MissingDependencies { missing } => {
            println!("   ✅ 成功检测到缺少依赖:");
            for dep in &missing {
                println!("      - {}", dep);
            }
        },
        _ => {
            println!("   ❌ 未能检测到缺少依赖");
        },
    }
    println!();

    // 7. 版本要求示例
    println!("7️⃣  带版本要求的依赖");
    let dep_with_version = Dependency::with_version("utils", "^1.0.0");
    println!("   依赖定义: {}", dep_with_version);
    println!("   说明: ^1.0.0 表示兼容 1.x.x 的任何版本");
    println!();

    // 8. 使用 SkillPackage 自动注册
    println!("8️⃣  从 SkillPackage 自动构建依赖图");
    let mut resolver_auto = DependencyResolver::new();
    let packages = vec![&skill_data, &skill_utils, &skill_logger, &skill_analytics];
    resolver_auto.add_skills(packages.iter().copied());

    let mut auto_graph = HashMap::new();
    for package in &packages {
        let deps: Vec<_> = package
            .metadata
            .dependencies
            .iter()
            .map(|d| Dependency::new(d.as_str()))
            .collect();
        auto_graph.insert(package.metadata.id.clone(), deps);
    }

    println!("   ✅ 自动构建了依赖图");
    match resolver_auto.resolve(&auto_graph) {
        ResolutionResult::Resolved { load_order } => {
            println!("   ✅ 解析成功,加载顺序:");
            for (i, skill_id) in load_order.iter().enumerate() {
                println!("      {}. {}", i + 1, skill_id);
            }
        },
        _ => {
            println!("   ❌ 解析失败");
        },
    }
    println!();

    println!("✨ 示例运行完成!");
    println!("\n💡 依赖解析的关键优势:");
    println!("   1. 自动确定正确的加载顺序");
    println!("   2. 检测循环依赖,避免无限循环");
    println!("   3. 识别缺少的依赖,提前发现问题");
    println!("   4. 支持版本要求管理");

    Ok(())
}
