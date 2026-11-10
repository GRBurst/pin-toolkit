//! Secret redaction for hand-written `Debug` impls (T-D29, P-redact): the
//! one helper the token-holding types share, outside the modules that use
//! it (review N2).

use std::fmt;

/// Prints as `<redacted>` (T-D29: tokens never reach `Debug` output).
pub(crate) struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// `Some(<redacted>)` / `None` for an optional secret.
pub(crate) fn redact(secret: &Option<String>) -> Option<impl fmt::Debug> {
    secret.as_ref().map(|_| Redacted)
}
