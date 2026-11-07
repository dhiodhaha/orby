use std::fmt;

use crate::model::EntityKind;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    /// The entity does not exist here, is deleted, or has not fully arrived yet.
    NotFound(EntityKind, String),
    /// The local user's role in the space does not allow this write.
    PermissionDenied,
    /// The write would break the model, e.g. moving a folder inside itself.
    Invalid(&'static str),
}

pub type Result<T> = std::result::Result<T, StoreError>;

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "database: {e}"),
            StoreError::Json(e) => write!(f, "payload: {e}"),
            StoreError::NotFound(kind, id) => write!(f, "{} {id} not found", kind.as_str()),
            StoreError::PermissionDenied => f.write_str("your role in this space does not allow that"),
            StoreError::Invalid(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StoreError::Sqlite(e) => Some(e),
            StoreError::Json(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Json(e)
    }
}
