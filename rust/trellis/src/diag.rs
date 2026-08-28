//! Daemon diagnostics. Step 2 carries the minimal shape (`code`,
//! `message`); the registry enrichment (`repair_class`, `spec_ref` —
//! contract §6) is wired in step 6, when `registry/diagnostics.json`
//! exists to source it from.

use serde::{Deserialize, Serialize};

/// Exit code discipline (mirroring soil0-cli §1): `0` success, `1` a
/// structured refusal or input diagnostic, `2` usage, environment, or
/// not-yet-implemented.
pub const EXIT_OK: i32 = 0;
pub const EXIT_DIAG: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

/// soil0-cli §3 shape: byte offsets canonical, line/col display-only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diag {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub span: Option<Span>,
    /// Registry enrichment (contract §6) — attached by
    /// `registry::enrich` at the daemon boundary.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub repair_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub spec_ref: Option<String>,
}

impl Diag {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Diag {
            code: code.to_string(),
            message: message.into(),
            file: None,
            span: None,
            repair_class: None,
            spec_ref: None,
        }
    }

    pub fn at(mut self, file: &str, span: Span) -> Self {
        self.file = Some(file.to_string());
        self.span = Some(span);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorReport {
    pub errors: Vec<Diag>,
}

impl ErrorReport {
    pub fn one(code: &str, message: impl Into<String>) -> Self {
        ErrorReport {
            errors: vec![Diag::new(code, message)],
        }
    }

    /// One compact JSON line, the stderr document.
    pub fn render(&self) -> String {
        serde_json::to_string(self).expect("error report serializes")
    }

    /// The exit code this report warrants: usage/internal-class codes
    /// are `2`, everything else is a `1`-class diagnostic.
    pub fn exit_code(&self) -> i32 {
        let usage_class = ["usage", "io", "internal", "unimplemented"];
        if self
            .errors
            .iter()
            .any(|d| usage_class.contains(&d.code.as_str()))
        {
            EXIT_USAGE
        } else {
            EXIT_DIAG
        }
    }
}
