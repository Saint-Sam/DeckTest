use std::{error::Error, fmt};

/// Result alias for card compiler operations.
pub type CardcResult<T> = Result<T, CardcError>;

/// Compiler error with file and one-based source position.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CardcError {
    /// Source path or logical name.
    pub path: String,
    /// Human-readable explanation.
    pub message: String,
    /// One-based source line.
    pub line: usize,
    /// One-based source column.
    pub column: usize,
}

impl CardcError {
    /// Creates a positioned compiler error.
    pub fn new(
        path: impl Into<String>,
        line: usize,
        column: usize,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for CardcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}: {}",
            self.path, self.line, self.column, self.message
        )
    }
}

impl Error for CardcError {}
