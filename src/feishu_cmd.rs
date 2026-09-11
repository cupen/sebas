//! `sebas feishu` — 会话外直连飞书的一次性 CLI（extract-im-service 之后的
//! 运维/测试辅助）。
//!
//! 与机器人会话完全无关：不经 core 通道、不依赖任何运行中的 sebas 实例，
//! 直接用 `[feishu]` 凭据换取 tenant token，向指定会话发文本或图片。
//! 典型用途：通知用户配合测试（如「请点一下权限卡」）、投递运维通知、
//! 发截图等。chat_id 从日志的 `open_chat_id` 取（p2p 会话也是 oc_ 开头）。

use crate::config::Config;
use anyhow::Context;
use sebas_feishu::client::{FeishuClient, FeishuConfig, TokenManager};
use sebas_feishu::events::SessionKey;

/// lib 侧参数（cli::FeishuArgs 的映射目标——cli 是 bin 模块，lib 不依赖它）。
pub struct FeishuArgs {
    pub config: String,
    pub chat: Option<String>,
    pub cmd: FeishuCmd,
}

pub enum FeishuCmd {
    Text { message: Vec<String> },
    Image { path: String },
}

pub async fn run(args: FeishuArgs) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(&args.config)
        .with_context(|| format!("读取配置失败: {}", args.config))?;
    let cfg = Config::parse(&raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    if cfg.feishu.app_id.is_empty() || cfg.feishu.app_secret.is_empty() {
        anyhow::bail!("[feishu] app_id/app_secret 为空：本命令直接使用飞书凭据，配置必须提供");
    }
    let Some(chat_id) = args.chat.clone().filter(|c| !c.is_empty()) else {
        anyhow::bail!("缺少 --chat <chat_id>（oc_ 开头；见日志 open_chat_id）");
    };

    let http = reqwest::Client::new();
    let tokens = TokenManager::new(cfg.feishu.app_id.clone(), cfg.feishu.app_secret.clone());
    let client = FeishuClient::new(FeishuConfig {
        app_id: cfg.feishu.app_id.clone(),
        app_secret: cfg.feishu.app_secret.clone(),
        owner_id: cfg.feishu.owner_id.clone(),
    });
    let key = SessionKey {
        chat_id,
        thread_id: None,
    };

    match &args.cmd {
        FeishuCmd::Text { message } => {
            let text = message.join(" ");
            if text.trim().is_empty() {
                anyhow::bail!("消息内容为空");
            }
            client.send_text(&http, &tokens, &key, &text).await?;
            println!("已发送文本到 {}: {}", key.chat_id, text);
        }
        FeishuCmd::Image { path } => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("读取图片失败: {path}"))?;
            let file_name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "image".into());
            let image_key = client
                .upload_image(&http, &tokens, &file_name, bytes)
                .await?;
            client.send_image(&http, &tokens, &key, &image_key).await?;
            println!("已发送图片 {path} 到 {}（image_key={image_key}）", key.chat_id);
        }
    }
    Ok(())
}
