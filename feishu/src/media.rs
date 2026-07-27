use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct MediaMeta {
    pub file_key: String,
    pub file_name: String,
    #[serde(default)]
    pub mime: Option<String>,
}

pub mod download_file {
    use super::*;

    pub fn compose_dest(dir: PathBuf, meta: &MediaMeta) -> PathBuf {
        dir.join(&meta.file_name)
    }

    /// Downloads a media file from Feishu to `dest`. Network-dependent.
    pub async fn download(
        http: &reqwest::Client,
        token: &str,
        file_key: &str,
        dest: &Path,
    ) -> anyhow::Result<()> {
        // 1. GET /im/v1/messages/{message_id}/resources/{file_key} → redirect URL or stream
        let url =
            format!("https://open.feishu.cn/open-apis/im/v1/messages/msg/resources/{file_key}");
        let bytes = http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(dest, &bytes).await?;
        Ok(())
    }
}
