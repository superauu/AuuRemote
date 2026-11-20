use std::fmt;

use serde::{Serialize, ser::SerializeStruct};

/// Stable machine-readable error codes. The frontend maps these to localized
/// messages; `detail` is a human-readable fallback for debugging.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ErrorCode {
    InvalidEndpoint,
    InvalidUsername,
    InvalidProfileName,
    InvalidFingerprint,
    InvalidMasterPassword,
    PasswordRequired,
    ConnectTimeout,
    ConnectionFailed,
    CertificateMismatch,
    SessionClosed,
    ResolutionUnsupported,
    SendTooLarge,
    NoFrame,
    ConfigCorrupt,
    VaultNotSetup,
    VaultWrongPassword,
    VaultAlreadySetup,
    VaultLocked,
    ProfileMissing,
    Internal,
    IoError,
}

impl ErrorCode {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidEndpoint => "invalidEndpoint",
            Self::InvalidUsername => "invalidUsername",
            Self::InvalidProfileName => "invalidProfileName",
            Self::InvalidFingerprint => "invalidFingerprint",
            Self::InvalidMasterPassword => "invalidMasterPassword",
            Self::PasswordRequired => "passwordRequired",
            Self::ConnectTimeout => "connectTimeout",
            Self::ConnectionFailed => "connectionFailed",
            Self::CertificateMismatch => "certificateMismatch",
            Self::SessionClosed => "sessionClosed",
            Self::ResolutionUnsupported => "resolutionUnsupported",
            Self::SendTooLarge => "sendTooLarge",
            Self::NoFrame => "noFrame",
            Self::ConfigCorrupt => "configCorrupt",
            Self::VaultNotSetup => "vaultNotSetup",
            Self::VaultWrongPassword => "vaultWrongPassword",
            Self::VaultAlreadySetup => "vaultAlreadySetup",
            Self::VaultLocked => "vaultLocked",
            Self::ProfileMissing => "profileMissing",
            Self::Internal => "internal",
            Self::IoError => "ioError",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ApiError {
    pub code: ErrorCode,
    pub detail: String,
}

impl ApiError {
    pub(crate) fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }

    /// Wraps an underlying error, keeping it as debug detail behind the code.
    pub(crate) fn chain(
        code: ErrorCode,
        error: &(impl fmt::Display + ?Sized),
        fallback: &str,
    ) -> Self {
        Self {
            code,
            detail: format!("{fallback}：{error}"),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for ApiError {}

impl Serialize for ApiError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("ApiError", 2)?;
        state.serialize_field("code", &self.code)?;
        state.serialize_field("detail", &self.detail)?;
        state.end()
    }
}
