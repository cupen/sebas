use thiserror::Error;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("router capacity {0} exceeded")]
    Capacity(usize),
}
