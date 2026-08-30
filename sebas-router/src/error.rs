use thiserror::Error;

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("router capacity {0} exceeded")]
    Capacity(usize),
}
