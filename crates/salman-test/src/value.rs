// SPDX-License-Identifier: Apache-2.0
//! Turning a value written in a test file into a runtime value.
//!
//! A test file says `PT: "T#5s"` or `Count: 3` or `Running: true`. Rather than
//! inventing a second syntax for values, salman lexes a quoted string with its
//! own lexer and takes the literal out of it — so every literal form the
//! language accepts works in a test file, `16#FF` and `T#1d2h` included, and
//! there is only one definition of what a duration literal means.

use salman_core::value::{ElementaryType, Value};
use salman_lang::dialect::Dialect;
use salman_lang::lexer::lex;
use salman_lang::token::{LiteralValue, TokenKind};
use salman_lang::types::integer_fits;
use serde::Deserialize;

/// A value as written in a test file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ValueSpec {
    /// `true` or `false`.
    Bool(bool),
    /// A plain integer, such as `3` or `-1`.
    Int(i64),
    /// A plain real, such as `1.5`.
    Real(f64),
    /// Anything else, written as a string: an IEC literal such as `"T#5s"`,
    /// `"16#FF"` or `"'text'"`.
    Text(String),
}

/// Why a written value could not become a runtime value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueError {
    /// The text is not a literal salman can read.
    NotALiteral(String),
    /// The literal does not fit the variable's type.
    OutOfRange {
        /// The literal as written.
        written: String,
        /// The variable's type.
        ty: ElementaryType,
    },
    /// The literal cannot be converted to the variable's type at all.
    WrongKind {
        /// The literal as written.
        written: String,
        /// The variable's type.
        ty: ElementaryType,
    },
}

impl std::fmt::Display for ValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotALiteral(text) => {
                write!(f, "{text:?} is not a literal salman can read")
            }
            Self::OutOfRange { written, ty } => {
                write!(f, "{written} does not fit in a {ty}")
            }
            Self::WrongKind { written, ty } => {
                write!(f, "{written} cannot be written to a {ty}")
            }
        }
    }
}

impl std::error::Error for ValueError {}

impl ValueSpec {
    /// How the value was written, for a message.
    #[must_use]
    pub fn written(&self) -> String {
        match self {
            Self::Bool(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Real(v) => v.to_string(),
            Self::Text(v) => v.clone(),
        }
    }

    /// Converts to a runtime value of `ty`.
    ///
    /// # Errors
    ///
    /// Returns [`ValueError`] when the text is not a literal, does not fit, or
    /// is of the wrong kind entirely.
    pub fn to_value(&self, ty: ElementaryType) -> Result<Value, ValueError> {
        let literal = match self {
            Self::Bool(v) => LiteralValue::Bool(*v),
            Self::Int(v) => LiteralValue::Int {
                magnitude: u128::from(v.unsigned_abs()),
                negative: *v < 0,
                declared: None,
            },
            Self::Real(v) => LiteralValue::Real { value: *v, declared: None },
            Self::Text(text) => parse_literal(text)
                .ok_or_else(|| ValueError::NotALiteral(text.clone()))?,
        };
        convert(&literal, ty).ok_or_else(|| match &literal {
            LiteralValue::Int { magnitude, negative, .. } => {
                let signed = i128::try_from(*magnitude).unwrap_or(i128::MAX);
                let value = if *negative { -signed } else { signed };
                if integer_fits(value, ty) {
                    ValueError::WrongKind { written: self.written(), ty }
                } else {
                    ValueError::OutOfRange { written: self.written(), ty }
                }
            }
            _ => ValueError::WrongKind { written: self.written(), ty },
        })
    }
}

/// Lexes one literal out of a string, or returns `None`.
fn parse_literal(text: &str) -> Option<LiteralValue> {
    let mut map = salman_core::span::SourceMap::new();
    let file = map.add("<test value>", text).ok()?;
    let (stream, diagnostics) = lex(file, text, &Dialect::generic());
    if diagnostics.has_errors() {
        return None;
    }
    let tokens = stream.tokens();
    // Exactly one literal, then end of input. Anything else is not a value.
    let first = tokens.first()?;
    if tokens.len() != 2 {
        return None;
    }
    match first.kind {
        TokenKind::Literal(index) => stream.literal(index).cloned(),
        _ => None,
    }
}

/// Fits a literal to a variable's type, or fails.
fn convert(literal: &LiteralValue, ty: ElementaryType) -> Option<Value> {
    use ElementaryType as E;
    match literal {
        LiteralValue::Bool(v) => (ty == E::Bool).then_some(Value::Bool(*v)),
        LiteralValue::Int { magnitude, negative, .. } => {
            let signed = i128::try_from(*magnitude).ok()?;
            let value = if *negative { -signed } else { signed };
            match ty {
                E::Real => Some(Value::real(value as f32)),
                E::Lreal => Some(Value::lreal(value as f64)),
                E::Time => Some(Value::Time(salman_core::time::Duration::from_nanos(
                    i64::try_from(value).ok()?,
                ))),
                E::LTime => Some(Value::LTime(salman_core::time::Duration::from_nanos(
                    i64::try_from(value).ok()?,
                ))),
                _ => integer_fits(value, ty).then(|| integer_value(value, ty)),
            }
        }
        LiteralValue::Real { value, .. } => match ty {
            E::Real => Some(Value::real(*value as f32)),
            E::Lreal => Some(Value::lreal(*value)),
            _ => None,
        },
        LiteralValue::Duration { value, .. } => match ty {
            E::Time => Some(Value::Time(*value)),
            E::LTime => Some(Value::LTime(*value)),
            _ => None,
        },
        LiteralValue::Date(d) => (ty == E::Date).then_some(Value::Date(*d)),
        LiteralValue::TimeOfDay(t) => (ty == E::TimeOfDay).then_some(Value::TimeOfDay(*t)),
        LiteralValue::DateAndTime(d) => {
            (ty == E::DateAndTime).then_some(Value::DateAndTime(*d))
        }
        LiteralValue::String(bytes) => (ty == E::String).then(|| Value::string(bytes)),
        LiteralValue::WString(units) => (ty == E::WString).then(|| Value::wstring(units)),
    }
}

fn integer_value(value: i128, ty: ElementaryType) -> Value {
    use ElementaryType as E;
    match ty {
        E::Sint => Value::Sint(value as i8),
        E::Int => Value::Int(value as i16),
        E::Lint => Value::Lint(value as i64),
        E::Usint => Value::Usint(value as u8),
        E::Uint => Value::Uint(value as u16),
        E::Udint => Value::Udint(value as u32),
        E::Ulint => Value::Ulint(value as u64),
        E::Byte => Value::Byte(value as u8),
        E::Word => Value::Word(value as u16),
        E::Dword => Value::Dword(value as u32),
        E::Lword => Value::Lword(value as u64),
        E::Bool => Value::Bool(value != 0),
        _ => Value::Dint(value as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ElementaryType as E;

    #[test]
    fn a_plain_boolean_becomes_a_bool() {
        assert_eq!(ValueSpec::Bool(true).to_value(E::Bool), Ok(Value::Bool(true)));
    }

    #[test]
    fn a_plain_integer_takes_the_variables_type() {
        assert_eq!(ValueSpec::Int(3).to_value(E::Int), Ok(Value::Int(3)));
        assert_eq!(ValueSpec::Int(3).to_value(E::Dint), Ok(Value::Dint(3)));
        assert_eq!(ValueSpec::Int(-3).to_value(E::Sint), Ok(Value::Sint(-3)));
    }

    #[test]
    fn an_integer_that_does_not_fit_says_so_rather_than_wrapping() {
        // Silently writing SINT#44 where the test said 300 would make the test
        // pass for the wrong reason, which is worse than failing.
        let err = ValueSpec::Int(300).to_value(E::Sint).unwrap_err();
        assert!(matches!(err, ValueError::OutOfRange { .. }), "{err}");
        assert!(err.to_string().contains("does not fit"), "{err}");
    }

    #[test]
    fn a_duration_is_written_as_an_iec_literal() {
        let value = ValueSpec::Text("T#5s".into()).to_value(E::Time).unwrap();
        assert_eq!(value.to_trace_string(), "T#5s");
        let long = ValueSpec::Text("T#4s999ms".into()).to_value(E::Time).unwrap();
        assert_eq!(long.as_duration().map(|d| d.nanos()), Some(4_999_000_000));
    }

    #[test]
    fn every_literal_form_the_language_accepts_works_in_a_test_file() {
        // The point of lexing with salman's own lexer: there is one definition
        // of what these mean, not two.
        assert_eq!(
            ValueSpec::Text("16#FF".into()).to_value(E::Byte),
            Ok(Value::Byte(255))
        );
        assert_eq!(
            ValueSpec::Text("2#1010".into()).to_value(E::Int),
            Ok(Value::Int(10))
        );
        assert_eq!(
            ValueSpec::Text("D#2024-02-29".into()).to_value(E::Date).map(|v| v.to_trace_string()),
            Ok("D#2024-02-29".to_string())
        );
        assert_eq!(
            ValueSpec::Text("'hello'".into()).to_value(E::String),
            Ok(Value::string(b"hello"))
        );
        assert_eq!(ValueSpec::Text("TRUE".into()).to_value(E::Bool), Ok(Value::Bool(true)));
    }

    #[test]
    fn something_that_is_not_a_literal_is_refused_with_the_text_that_was_written() {
        let err = ValueSpec::Text("Motor_Run".into()).to_value(E::Bool).unwrap_err();
        assert!(matches!(err, ValueError::NotALiteral(_)), "{err}");
        assert!(err.to_string().contains("Motor_Run"), "{err}");
        assert!(ValueSpec::Text("T#5s + 1".into()).to_value(E::Time).is_err());
        assert!(ValueSpec::Text(String::new()).to_value(E::Bool).is_err());
    }

    #[test]
    fn a_literal_of_the_wrong_kind_is_refused_rather_than_coerced() {
        // Writing a duration into a BOOL is a mistake in the test, not
        // something to guess at.
        let err = ValueSpec::Text("T#5s".into()).to_value(E::Bool).unwrap_err();
        assert!(matches!(err, ValueError::WrongKind { .. }), "{err}");
        assert!(ValueSpec::Bool(true).to_value(E::Dint).is_err());
    }

    #[test]
    fn integers_widen_into_reals_because_writing_0_for_a_real_is_natural() {
        assert_eq!(ValueSpec::Int(0).to_value(E::Real), Ok(Value::real(0.0)));
        assert_eq!(ValueSpec::Real(1.5).to_value(E::Lreal), Ok(Value::lreal(1.5)));
    }
}
