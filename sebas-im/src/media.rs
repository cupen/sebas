//! 媒体解析（extract-im-service 4.2）：im 持飞书凭据，把入站媒体 file_key
//! 解析为本地可用的附件（下载到 `[media] download_dir`）。超限与失败如实
//! 报错，调用方以纯文本反馈（spec: im-service「媒体解析与图片上传」）。

use std::path::{Path, PathBuf};

/// 单文件大小上限兜底（`[media] max_file_size` 为 0 时仍生效的硬顶）。
pub const HARD_SIZE_CAP: u64 = 50 * 1024 * 1024;

/// 下载一个媒体文件到 `dir`，返回本地路径。`message_id` 是携带该文件的消息
/// id（飞书 media API 的路径参数）。流式落盘，超过 `max_file_size` 拒绝。
pub async fn resolve(
    http: &reqwest::Client,
    bearer: &str,
    message_id: &str,
    file_key: &str,
    dir: &Path,
    max_file_size: u64,
) -> Result<PathBuf, String> {
    if file_key.is_empty() || message_id.is_empty() {
        return Err("媒体引用不完整（缺 file_key 或 message_id）".into());
    }
    let cap = if max_file_size == 0 { HARD_SIZE_CAP } else { max_file_size };
    let url = format!(
        "https://open.feishu.cn/open-apis/im/v1/messages/{message_id}/resources/{file_key}?type=file"
    );
    let resp = http
        .get(&url)
        .bearer_auth(bearer)
        .send()
        .await
        .map_err(|e| format!("媒体下载失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("媒体下载失败：HTTP {}", resp.status()));
    }
    let total = resp.content_length();
    if let Some(n) = total
        && n > cap
    {
        return Err(format!("附件超过大小上限（{n} > {cap} 字节），已拒绝"));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("媒体读取失败：{e}"))?;
    if bytes.len() as u64 > cap {
        return Err(format!("附件超过大小上限（{} > {cap} 字节），已拒绝", bytes.len()));
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| format!("创建媒体目录失败：{e}"))?;
    let dest = dir.join(file_key);
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| format!("媒体落盘失败：{e}"))?;
    Ok(dest)
}
