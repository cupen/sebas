//! 中立原语：路径展开与时间戳（add-domain-layer 任务 2.4 / 2.5，design D7）。
//!
//! 这些函数的复制成因是「够不着根 crate」；它们本身角色中立，落在域层是
//! **轻微异味**（design D7 已承认并被否决独立 `sebas-util` 的备选）。
//! 触发条件：本模块增长到需要自己的依赖（如 fs 事务语义）时拆出。

/// 展开 `~/` 前缀为用户主目录下的绝对路径（字符串形态）。
///
/// `~` 解析走 Known Folder（`dirs::home_dir`），**不吃 `HOME` env**——这是
/// 有意的：沙箱把 `HOME` 钉进一次性目录时，配置里的 `~/...` 仍指向操作者
/// 主目录的语义不变。其余前缀原样返回。
///
/// 唯一实现（spec「tilde expansion has one implementation」）；原根
/// `src/config.rs` / `sebas-router` / `sebas-dispatch` 三份副本已删除。
pub fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest).to_string_lossy().into();
    }
    p.to_string()
}

/// 当前 Unix 时间（秒）。时钟回拨（早于 epoch）时如实地给 0，不 panic。
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 当前 Unix 时间（毫秒，u128 形态）。仅 crud 的提交 id 拼接使用。
pub fn now_unix_millis() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_prefix_expands_and_other_paths_pass_through() {
        let home = dirs::home_dir().expect("test host must have a home dir");
        let expanded = expand_tilde("~/notes");
        assert_eq!(expanded, home.join("notes").to_string_lossy());

        // 非 `~/` 前缀（含裸 `~` 与 `~user` 形态）一律原样返回。
        assert_eq!(expand_tilde("/abs/path"), "/abs/path");
        assert_eq!(expand_tilde("relative/path"), "relative/path");
        assert_eq!(expand_tilde("~"), "~");
        assert_eq!(expand_tilde("~user/x"), "~user/x");
        assert_eq!(expand_tilde(""), "");
    }

    #[test]
    fn now_unix_is_epoch_seconds_and_millis_is_larger() {
        let secs = now_unix();
        // 2025-09-30T00:00:00Z 附近之后的下界（本仓开发期）；回拨到 0 是
        // 唯一合法的"异常"值。
        assert!(secs == 0 || secs > 1_700_000_000);
        let millis = now_unix_millis();
        assert!(millis == 0 || millis > 1_700_000_000_000);
    }
}
