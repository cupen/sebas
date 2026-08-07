use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SebasError {
    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("feishu error: {0}")]
    Feishu(String),

    #[error("acp error: {0}")]
    Acp(String),

    #[error("router error: {0}")]
    Router(String),

    #[error("gateway error: {0}")]
    Gateway(String),
}

pub type Result<T> = std::result::Result<T, SebasError>;
