//! Stable diagnostic codes.
//!
//! Codes appear in users' CI filters and lint suppressions, so once published a
//! code keeps its meaning for ever. Retiring one means never reusing the
//! number.
//!
//! * `E01xx` — lexical
//! * `E02xx` — syntactic
//! * `E03xx` — declarations and symbols
//! * `E04xx` — types
//! * `W0xxx` — warnings
//! * `U0xxx` — constructs salman does not implement yet

use salman_core::diag::DiagCode;

/// A byte that cannot begin any token.
pub const E_UNEXPECTED_CHARACTER: DiagCode = DiagCode("E0101");
/// A block comment that runs to the end of the file.
pub const E_UNTERMINATED_COMMENT: DiagCode = DiagCode("E0102");
/// A string literal that runs to the end of its line or the end of the file.
pub const E_UNTERMINATED_STRING: DiagCode = DiagCode("E0103");
/// A `$` escape that is not one of the eight defined combinations or `$xx`.
pub const E_BAD_ESCAPE: DiagCode = DiagCode("E0104");
/// A based literal whose radix is not 2, 8 or 16.
pub const E_BAD_RADIX: DiagCode = DiagCode("E0105");
/// A digit that does not exist in the literal's radix, such as `2#2`.
pub const E_BAD_DIGIT: DiagCode = DiagCode("E0106");
/// A numeric literal too large for any salman integer type.
pub const E_LITERAL_OUT_OF_RANGE: DiagCode = DiagCode("E0107");
/// A real literal that is not `digits.digits[exponent]`.
pub const E_BAD_REAL: DiagCode = DiagCode("E0108");
/// A duration literal that breaks the unit ordering or overflow rules.
pub const E_BAD_DURATION: DiagCode = DiagCode("E0109");
/// A date, time-of-day or date-and-time literal that is not a real instant.
pub const E_BAD_DATE_TIME: DiagCode = DiagCode("E0110");
/// A `%` address that is not `%` location \[size\] digits (`.` digits)\*.
pub const E_BAD_DIRECT_ADDRESS: DiagCode = DiagCode("E0111");
/// An underscore in a numeric literal that does not separate two digits.
pub const E_MISPLACED_UNDERSCORE: DiagCode = DiagCode("E0112");
/// Comments or brackets nested deeper than the dialect allows.
pub const E_NESTING_TOO_DEEP: DiagCode = DiagCode("E0113");
/// A pragma `{ ... }` that is never closed.
pub const E_UNTERMINATED_PRAGMA: DiagCode = DiagCode("E0114");
/// A construct the dialect in force does not accept.
pub const E_DIALECT_REJECTS: DiagCode = DiagCode("E0115");
/// An identifier longer than salman accepts.
pub const E_IDENT_TOO_LONG: DiagCode = DiagCode("E0116");

/// A literal prefix naming a type salman has not implemented, such as `LDT#`.
pub const U_UNSUPPORTED_LITERAL_PREFIX: DiagCode = DiagCode("U0101");

/// A duration literal finer than one nanosecond, whose tail was truncated.
pub const W_DURATION_TRUNCATED: DiagCode = DiagCode("W0101");
/// Two consecutive underscores in an identifier.
pub const W_CONSECUTIVE_UNDERSCORES: DiagCode = DiagCode("W0102");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_codes_are_unique() {
        let all = [
            E_UNEXPECTED_CHARACTER,
            E_UNTERMINATED_COMMENT,
            E_UNTERMINATED_STRING,
            E_BAD_ESCAPE,
            E_BAD_RADIX,
            E_BAD_DIGIT,
            E_LITERAL_OUT_OF_RANGE,
            E_BAD_REAL,
            E_BAD_DURATION,
            E_BAD_DATE_TIME,
            E_BAD_DIRECT_ADDRESS,
            E_MISPLACED_UNDERSCORE,
            E_NESTING_TOO_DEEP,
            E_UNTERMINATED_PRAGMA,
            E_DIALECT_REJECTS,
            E_IDENT_TOO_LONG,
            U_UNSUPPORTED_LITERAL_PREFIX,
            W_DURATION_TRUNCATED,
            W_CONSECUTIVE_UNDERSCORES,
        ];
        let mut codes: Vec<&str> = all.iter().map(|c| c.0).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count, "duplicate diagnostic code");
    }
}
