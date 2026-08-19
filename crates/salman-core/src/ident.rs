// SPDX-License-Identifier: Apache-2.0
//! IEC 61131-3 identifiers.
//!
//! Identifiers in IEC 61131-3 are **case-insensitive**: `motorRun`, `MotorRun`
//! and `MOTORRUN` name the same variable. That single fact leaks into symbol
//! tables, diagnostics, go-to-definition and the importer, so it is modelled
//! once here rather than by scattering `to_ascii_lowercase()` through the
//! codebase.
//!
//! [`Ident`] preserves the spelling the engineer wrote — it is what gets shown
//! back to them — while comparing and hashing case-insensitively.
//!
//! Case-insensitivity is deliberately **ASCII-only**. IEC identifiers are
//! defined over letters, digits and the underscore; applying Unicode case
//! folding would make identifier identity depend on the Unicode version the
//! build was compiled against, which would break the determinism gate.

use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Longest identifier salman will accept.
///
/// Identifiers come from files salman did not write. A bound keeps a hostile
/// file from turning one token into an unbounded allocation.
pub const MAX_IDENT_BYTES: usize = 1024;

/// Why a string is not a usable IEC 61131-3 identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentError {
    /// The string was empty.
    Empty,
    /// The string was longer than [`MAX_IDENT_BYTES`].
    TooLong,
    /// The first character was neither a letter nor an underscore.
    BadFirstCharacter,
    /// A character other than a letter, digit or underscore appeared.
    BadCharacter,
}

impl fmt::Display for IdentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "identifier is empty",
            Self::TooLong => "identifier is too long",
            Self::BadFirstCharacter => "identifier must start with a letter or an underscore",
            Self::BadCharacter => "identifier may contain only letters, digits and underscores",
        })
    }
}

impl std::error::Error for IdentError {}

/// An IEC 61131-3 identifier: case-preserving, case-insensitive.
#[derive(Debug, Clone)]
pub struct Ident {
    text: Box<str>,
}

impl Ident {
    /// Parses an identifier, rejecting anything outside the IEC character set.
    ///
    /// # Errors
    ///
    /// Returns [`IdentError`] describing the first violation found.
    pub fn new(text: impl Into<Box<str>>) -> Result<Self, IdentError> {
        let text: Box<str> = text.into();
        if text.is_empty() {
            return Err(IdentError::Empty);
        }
        if text.len() > MAX_IDENT_BYTES {
            return Err(IdentError::TooLong);
        }
        let mut bytes = text.bytes();
        // `bytes` is non-empty, checked above.
        let first = bytes.next().unwrap_or(b'0');
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return Err(IdentError::BadFirstCharacter);
        }
        for b in bytes {
            if !(b.is_ascii_alphanumeric() || b == b'_') {
                return Err(IdentError::BadCharacter);
            }
        }
        Ok(Self { text })
    }

    /// The spelling as written by the engineer.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether this identifier contains two consecutive underscores.
    ///
    /// IEC 61131-3 is generally understood to forbid consecutive underscores in
    /// identifiers, and several dialects reject them. salman treats this as a
    /// **lint**, not a parse error, so that importing existing code from a
    /// dialect which permits it does not fail outright.
    ///
    /// The exact clause for this rule has not been confirmed against a public
    /// source; see `docs/IEC_CITATIONS.md` for salman's citation policy.
    #[must_use]
    pub fn has_consecutive_underscores(&self) -> bool {
        self.text.as_bytes().windows(2).any(|w| w == b"__")
    }

    /// Compares against a plain string using IEC identifier rules.
    #[must_use]
    pub fn eq_str(&self, other: &str) -> bool {
        self.text.eq_ignore_ascii_case(other)
    }

    /// The lowercase form used as the lookup key in symbol tables.
    #[must_use]
    pub fn to_key(&self) -> IdentKey {
        IdentKey(self.text.to_ascii_lowercase().into_boxed_str())
    }
}

impl PartialEq for Ident {
    fn eq(&self, other: &Self) -> bool {
        self.text.eq_ignore_ascii_case(&other.text)
    }
}

impl Eq for Ident {}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ident {
    /// Orders case-insensitively, so that any output produced by sorting
    /// identifiers is stable regardless of how they were spelled.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let a = self.text.bytes().map(|b| b.to_ascii_lowercase());
        let b = other.text.bytes().map(|b| b.to_ascii_lowercase());
        a.cmp(b)
    }
}

impl Hash for Ident {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for b in self.text.bytes() {
            state.write_u8(b.to_ascii_lowercase());
        }
        state.write_u8(0xff);
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// The canonical lookup key for an identifier: its ASCII-lowercase form.
///
/// Symbol tables are keyed on this so that iteration order is a function of the
/// canonical spelling only, which keeps generated output deterministic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdentKey(Box<str>);

impl IdentKey {
    /// The lowercase key text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for IdentKey {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn id(s: &str) -> Ident {
        Ident::new(s).unwrap()
    }

    #[test]
    fn identifiers_compare_case_insensitively() {
        assert_eq!(id("MotorRun"), id("motorrun"));
        assert_eq!(id("MOTOR_RUN"), id("motor_run"));
        assert_ne!(id("MotorRun"), id("MotorStop"));
    }

    #[test]
    fn identifiers_preserve_the_spelling_that_was_written() {
        assert_eq!(id("MotorRun").as_str(), "MotorRun");
        assert_eq!(id("MotorRun").to_string(), "MotorRun");
    }

    #[test]
    fn identifiers_hash_case_insensitively() {
        // A HashSet is the point of this test: it is what exercises the `Hash`
        // impl. Nothing here is iterated and nothing leaves the test, so the
        // non-deterministic order the clippy.toml ban exists to prevent cannot
        // reach a trace or a generated file.
        #[allow(clippy::disallowed_types, reason = "testing Hash; order never escapes")]
        let mut set = std::collections::HashSet::new();
        set.insert(id("Valve_Open"));
        assert!(set.contains(&id("VALVE_OPEN")));
        assert_eq!(set.len(), 1);
        set.insert(id("valve_open"));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn identifier_ordering_ignores_case_so_generated_output_is_stable() {
        let mut map = BTreeMap::new();
        for name in ["beta", "Alpha", "GAMMA"] {
            map.insert(id(name).to_key(), name);
        }
        let order: Vec<&str> = map.values().copied().collect();
        assert_eq!(order, ["Alpha", "beta", "GAMMA"]);
    }

    #[test]
    fn case_folding_is_ascii_only_so_it_cannot_drift_with_unicode_versions() {
        // Non-ASCII is rejected outright, so there is no Unicode case folding
        // anywhere in identifier identity.
        assert_eq!(Ident::new("Motörn"), Err(IdentError::BadCharacter));
    }

    #[test]
    fn an_identifier_may_not_start_with_a_digit() {
        assert_eq!(Ident::new("1Motor"), Err(IdentError::BadFirstCharacter));
    }

    #[test]
    fn an_identifier_may_start_with_an_underscore() {
        assert_eq!(id("_internal").as_str(), "_internal");
    }

    #[test]
    fn empty_and_oversized_identifiers_are_rejected() {
        assert_eq!(Ident::new(""), Err(IdentError::Empty));
        let long = "a".repeat(MAX_IDENT_BYTES + 1);
        assert_eq!(Ident::new(long), Err(IdentError::TooLong));
    }

    #[test]
    fn consecutive_underscores_are_detected_but_do_not_prevent_parsing() {
        let i = id("Motor__Run");
        assert!(i.has_consecutive_underscores());
        assert!(!id("Motor_Run").has_consecutive_underscores());
    }

    #[test]
    fn eq_str_uses_iec_case_rules() {
        assert!(id("TON").eq_str("ton"));
        assert!(!id("TON").eq_str("tof"));
    }
}
