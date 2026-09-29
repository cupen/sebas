//! Agent Skills - 热加载功能示例
//!
//! 这个示例展示了如何使用热加载功能自动监控技能文件变化
//!
//! 运行: cargo run --example 38_agent_skills_hot_reload --features hot-reload

use claude_agent_sdk::skills::{
    HotReloadConfig, HotReloadManager, HotReloadWatcher, SkillMetadata,
    SkillPackage,
};
use std::fs;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔥 Agent Skills - 热加载功能示例\n");

    // 1. 创建临时目录用于演示
    println!("1️⃣  创建临时目录");
    let temp_dir = std::env::temp_dir().join("skills_hot_reload_demo");
    fs::create_dir_all(&temp_dir)?;
    println!("   ✅ 临时目录: {:?}", temp_dir);
    println!();

    // 2. 创建初始技能文件
    println!("2️⃣  创建初始技能文件");
    let skill1_path = temp_dir.join("skill1.json");
    let skill1 = SkillPackage {
        metadata: SkillMetadata {
            id: "data-processor".to_string(),
            name: "Data Processor".to_string(),
            description: "Processes data efficiently".to_string(),
            version: "1.0.0".to_string(),
            author: Some("Demo Team".to_string()),
            dependencies: vec![],
            tags: vec!["data".to_string(), "processing".to_string()],
        },
        instructions: "Process the data efficiently".to_string(),
        scripts: vec![],
        resources: Default::default(),
    };
    skill1.save_to_file(&skill1_path)?;
    println!("   ✅ 创建技能: {}", skill1.metadata.name);
    println!();

    // 3. 设置热加载
    println!("3️⃣  设置热加载监控");
    let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();

    let config = HotReloadConfig {
        debounce_duration: Duration::from_millis(100),
        recursive: true,
        file_patterns: vec!["*.json".to_string(), "*.yaml".to_string()],
    };

    // 启动监控器
    let _watcher = HotReloadWatcher::new(&temp_dir, config, event_sender)?;
    println!("   ✅ 监控器已启动");
    println!("   📁 监控路径: {:?}", temp_dir);
    println!("   🎯 监控模式: *.json, *.yaml");
    println!();

    // 4. 创建管理器
    println!("4️⃣  创建热加载管理器");
    let mut manager = HotReloadManager::new(event_receiver);

    // 处理初始文件创建事件（watcher会自动检测并发送事件）
    sleep(Duration::from_millis(200)).await;
    let event_count = manager.process_events();

    println!("   ✅ 管理器已创建");
    println!("   📦 处理了 {} 个初始事件", event_count);
    println!("   📦 当前技能数: {}", manager.get_skills().len());
    println!();

    // 5. 演示热加载场景
    println!("5️⃣  演示热加载场景");
    println!("   等待 3 秒后创建新技能文件...");
    sleep(Duration::from_secs(3)).await;

    // 创建第二个技能
    let skill2_path = temp_dir.join("skill2.json");
    let skill2 = SkillPackage {
        metadata: SkillMetadata {
            id: "text-analyzer".to_string(),
            name: "Text Analyzer".to_string(),
            description: "Analyzes text patterns".to_string(),
            version: "1.0.0".to_string(),
            author: Some("Demo Team".to_string()),
            dependencies: vec![],
            tags: vec!["text".to_string(), "analysis".to_string()],
        },
        instructions: "Analyze text patterns".to_string(),
        scripts: vec![],
        resources: Default::default(),
    };
    skill2.save_to_file(&skill2_path)?;
    println!("   ✅ 创建新技能: {}", skill2.metadata.name);

    // 处理事件
    sleep(Duration::from_millis(200)).await;
    let event_count = manager.process_events();
    println!("   📊 处理了 {} 个事件", event_count);
    println!("   📦 当前技能数: {}", manager.get_skills().len());
    println!();

    // 6. 演示技能修改
    println!("6️⃣  演示技能修改检测");
    println!("   等待 3 秒后修改技能文件...");
    sleep(Duration::from_secs(3)).await;

    // 修改第一个技能
    let mut modified_skill = SkillPackage::load_from_file(&skill1_path)?;
    modified_skill.metadata.version = "1.1.0".to_string();
    modified_skill.metadata.description = "Processes data efficiently (updated)".to_string();
    modified_skill.save_to_file(&skill1_path)?;
    println!(
        "   ✅ 修改技能: {} -> v{}",
        modified_skill.metadata.name, modified_skill.metadata.version
    );

    // 处理事件
    sleep(Duration::from_millis(200)).await;
    let event_count = manager.process_events();
    println!("   📊 处理了 {} 个事件", event_count);

    // 验证更新
    if let Some(updated_skill) = manager.get_skill(&skill1_path) {
        println!("   ✅ 技能已更新: v{}", updated_skill.metadata.version);
        println!("   📝 描述: {}", updated_skill.metadata.description);
    }
    println!();

    // 7. 演示技能删除
    println!("7️⃣  演示技能删除检测");
    println!("   等待 3 秒后删除技能文件...");
    sleep(Duration::from_secs(3)).await;

    // 删除第二个技能
    fs::remove_file(&skill2_path)?;
    println!("   ✅ 删除技能: {}", skill2.metadata.name);

    // 处理事件
    sleep(Duration::from_millis(200)).await;
    let event_count = manager.process_events();
    println!("   📊 处理了 {} 个事件", event_count);
    println!("   📦 当前技能数: {}", manager.get_skills().len());
    println!();

    // 8. 列出所有技能
    println!("8️⃣  当前加载的技能");
    for skill in manager.get_skills() {
        println!(
            "   📦 {} (v{}) - {}",
            skill.metadata.name, skill.metadata.version, skill.metadata.description
        );
    }
    println!();

    // 9. 清理
    println!("9️⃣  清理临时文件");
    fs::remove_file(&skill1_path)?;
    fs::remove_dir(&temp_dir)?;
    println!("   ✅ 已清理临时文件");
    println!();

    println!("✨ 示例运行完成!");
    println!("\n💡 热加载的关键特性:");
    println!("   1. 自动监控文件变化 (创建/修改/删除)");
    println!("   2. 实时更新内存中的技能");
    println!("   3. 支持多种文件格式 (JSON, YAML)");
    println!("   4. 可配置的防抖延迟");
    println!("   5. 事件驱动的异步架构");
    println!("   6. 详细的日志和错误处理");
    println!("\n📚 使用场景:");
    println!("   - 开发环境: 修改技能后无需重启");
    println!("   - 生产环境: 动态更新技能配置");
    println!("   - 多技能管理: 统一监控和加载");
    println!("   - 技能发现: 自动发现新技能");

    Ok(())
}
