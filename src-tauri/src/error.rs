// SOT: app-error, error-kinds, ipc-error-shape, error-mapping

use serde::Serialize;
use std::fmt;
use ts_rs::TS;

// WHAT:  The single error type every layer returns and the UI receives.
// WHY:   One shape means the frontend can switch on `kind` exhaustively; a new
//        variant here is a compile error in TS after `pnpm bindings`.
// HOW:   Vendor errors (sqlx, rusqlite, keyring, aes-gcm) are mapped into it
//        inside their own adapter layer — never leaked past it.
// WHERE: src/lib/bindings/AppError.ts (generated), src/lib/ipc.ts (consumer)
#[derive(Debug, Clone, Serialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum AppError {
    NotFound { message: String },
    NotConnected { message: String },
    ReadOnly { message: String },
    DestructiveConfirmationRequired { message: String, statements: Vec<String> },
    InvalidInput { message: String },
    Timeout { message: String },
    /// The user pressed Stop (`cancel_query`) while the run was in flight.
    Cancelled { message: String },
    /// A failure the database itself reported. `position` is the 1-based
    /// character offset into the statement that was sent, `hint` and `detail`
    /// are the server's own follow-up lines. Adapters that can say more than
    /// the message (Postgres, MySQL, ClickHouse) fill them; the editor draws
    /// a squiggle at `position` and prints the other two under the message.
    Driver { message: String, position: Option<u32>, hint: Option<String>, detail: Option<String> },
    Store { message: String },
    Crypto { message: String },
    Keyring { message: String },
    Internal { message: String },
}

impl AppError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound { message: message.into() }
    }
    pub fn not_connected(message: impl Into<String>) -> Self {
        Self::NotConnected { message: message.into() }
    }
    pub fn read_only(message: impl Into<String>) -> Self {
        Self::ReadOnly { message: message.into() }
    }
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput { message: message.into() }
    }
    pub fn invalid_input_from(err: impl fmt::Display) -> Self {
        Self::InvalidInput { message: err.to_string() }
    }
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::Timeout { message: message.into() }
    }
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::Cancelled { message: message.into() }
    }
    pub fn driver(message: impl fmt::Display) -> Self {
        Self::Driver { message: message.to_string(), position: None, hint: None, detail: None }
    }
    /// A database failure with the server's own pointer into the statement.
    /// Blank hints and details are dropped, so the UI never prints an empty line.
    pub fn driver_with(message: impl fmt::Display, position: Option<u32>, hint: Option<String>, detail: Option<String>) -> Self {
        let keep = |text: Option<String>| text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        Self::Driver { message: message.to_string(), position: position.filter(|p| *p > 0), hint: keep(hint), detail: keep(detail) }
    }
    pub fn store(message: impl fmt::Display) -> Self {
        Self::Store { message: message.to_string() }
    }
    pub fn crypto(message: impl fmt::Display) -> Self {
        Self::Crypto { message: message.to_string() }
    }
    pub fn keyring(message: impl fmt::Display) -> Self {
        Self::Keyring { message: message.to_string() }
    }
    pub fn internal(message: impl fmt::Display) -> Self {
        Self::Internal { message: message.to_string() }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::NotFound { message }
            | Self::NotConnected { message }
            | Self::ReadOnly { message }
            | Self::DestructiveConfirmationRequired { message, .. }
            | Self::InvalidInput { message }
            | Self::Timeout { message }
            | Self::Cancelled { message }
            | Self::Driver { message, .. }
            | Self::Store { message }
            | Self::Crypto { message }
            | Self::Keyring { message }
            | Self::Internal { message } => message,
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for AppError {}

pub type AppResult<T> = Result<T, AppError>;
