#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CanteenError(String);

impl CanteenError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

pub type Result<T> = std::result::Result<T, CanteenError>;

impl From<CanteenError> for String {
    fn from(error: CanteenError) -> Self {
        error.0
    }
}
