use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("TOML parse error: {0}")]
    Toml(String),
    #[error("grid parse error: {0}")]
    GridParse(String),
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("rule conflict: {0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, ConfigError>;
