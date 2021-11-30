use std::path::PathBuf;

use thiserror::Error;

pub type WatcherError = notify::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("Error when loading config file: {0}")]
    Figment(#[from] figment::Error),
    #[error("Missing rule: {0}")]
    Rule(String),
    #[error("Specified path does not exist or is a file: {0}")]
    NotADirectory(PathBuf),
}
