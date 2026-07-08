/// Errors produced by this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV parse error: {0}")]
    Csv(#[from] csv::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("file contains no data rows")]
    Empty,
    #[error("could not identify a description column")]
    NoDescriptionColumn,
    #[error("profile format version {found} is newer than the version this build supports ({current})")]
    UnsupportedVersion { found: u32, current: u32 },
    #[error("unknown tag: {0}")]
    UnknownTag(String),
}
