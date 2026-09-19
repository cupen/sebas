use thiserror::Error;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("router capacity {0} exceeded")]
    Capacity(usize),
    /// fix-webui-qa-defects 2.1：归档恢复遇状态矛盾（key 已有活映射，重建
    /// 会覆盖在用状态）。带人类可读原因，调用方原样透出。
    #[error("{0}")]
    Conflict(String),
}
