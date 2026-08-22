use std::fmt;

/// The classification of a runtime failure.
///
/// This is the `panic` effect made concrete (design §3.3): anything that
/// can go wrong at runtime is one of these kinds, and hosts receive it
/// through the C ABI as a structured error, never as an unwind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicKind {
    /// Integer overflow in an arithmetic primitive.
    Overflow,
    /// Zero divisor in integer `/` or `%`.
    DivideByZero,
    /// Input JSON rejected by type-directed decode.
    DecodeError,
    /// A value of the wrong shape reached an operation (defensive; the
    /// checker prevents this in checked code).
    TypeError,
    /// A derived operation (`eq`/`compare`/`hash`/`show`) was invoked on
    /// a type marked non-derivable, or reached a closure.
    DerivationError,
    /// The C ABI was used against its documented contract (undefined
    /// `TypeId`, wrong arity, use after teardown).
    CapiMisuse,
    /// A bug in the runtime itself, caught at the C ABI boundary.
    Internal,
}

impl PanicKind {
    pub fn name(self) -> &'static str {
        match self {
            PanicKind::Overflow => "overflow",
            PanicKind::DivideByZero => "divide-by-zero",
            PanicKind::DecodeError => "decode-error",
            PanicKind::TypeError => "type-error",
            PanicKind::DerivationError => "derivation-error",
            PanicKind::CapiMisuse => "capi-misuse",
            PanicKind::Internal => "internal",
        }
    }
}

/// One frame of a definition-level trace.
///
/// The runtime only carries these; filling them in is the interpreter's
/// and daemon's job (debug-mode instrumentation, design §4.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceFrame {
    /// The Trellis definition name, e.g. `"csvstats::median"`.
    pub definition: String,
}

/// A structured runtime error — the debug-mode payload of the host-stub
/// rule (design §3.11): kind, message, definition-level trace, and the
/// offending inputs as canonical JSON when available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoilError {
    pub kind: PanicKind,
    pub message: String,
    pub trace: Vec<TraceFrame>,
    /// Canonical-JSON rendering of the inputs that provoked the error,
    /// when the failing operation had them at hand.
    pub inputs: Option<String>,
}

impl SoilError {
    pub fn new(kind: PanicKind, message: impl Into<String>) -> Self {
        SoilError {
            kind,
            message: message.into(),
            trace: Vec::new(),
            inputs: None,
        }
    }

    pub fn with_inputs(mut self, inputs: impl Into<String>) -> Self {
        self.inputs = Some(inputs.into());
        self
    }
}

impl fmt::Display for SoilError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.name(), self.message)?;
        if let Some(inputs) = &self.inputs {
            write!(f, " (inputs: {inputs})")?;
        }
        for frame in &self.trace {
            write!(f, "\n  in {}", frame.definition)?;
        }
        Ok(())
    }
}

impl std::error::Error for SoilError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_kind_message_and_trace() {
        let mut err =
            SoilError::new(PanicKind::DivideByZero, "divisor is zero").with_inputs("[10,0]");
        err.trace.push(TraceFrame {
            definition: "csvstats::mean".to_string(),
        });
        let rendered = err.to_string();
        assert_eq!(
            rendered,
            "divide-by-zero: divisor is zero (inputs: [10,0])\n  in csvstats::mean"
        );
    }

    #[test]
    fn kind_names_are_stable() {
        assert_eq!(PanicKind::Overflow.name(), "overflow");
        assert_eq!(PanicKind::Internal.name(), "internal");
    }
}
