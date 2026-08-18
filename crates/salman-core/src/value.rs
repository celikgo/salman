//! IEC 61131-3 elementary types and the runtime values that inhabit them.
//!
//! # The generic type hierarchy
//!
//! IEC 61131-3 groups the elementary types into a hierarchy — `ANY_INT`,
//! `ANY_BIT`, `ANY_NUM` and so on — and the standard function signatures are
//! written in terms of it. [`GenericType`] models that hierarchy directly, so
//! that "this operator accepts `ANY_NUM`" is a statement the type checker can
//! evaluate rather than a comment.
//!
//! # Floating point and the determinism gate
//!
//! salman requires that the same project, run with the same inputs on Linux,
//! macOS and Windows, produces a byte-identical trace. Floating point is the
//! hardest part of that promise, and two decisions here are load-bearing:
//!
//! 1. **NaN is canonicalised on the way in.** The bit pattern a processor
//!    produces for a NaN is not portable: the payload and the sign bit differ
//!    between architectures and between operations, and Rust explicitly does
//!    not guarantee them. Every `REAL` and `LREAL` value entering a
//!    [`Value`] therefore passes through [`canonical_f32`] / [`canonical_f64`],
//!    which collapse every NaN to one quiet NaN with a zero payload. Two runs
//!    on different machines cannot then disagree about the bits of a NaN.
//! 2. **Negative zero is preserved.** Unlike NaN, `-0.0` is portable and
//!    semantically meaningful — `1.0 / -0.0` is `-inf` — so it is left alone.
//!
//! The remaining floating-point hazards are not this module's to fix and are
//! handled where they arise: the arithmetic operations `+ - * /` and `sqrt` are
//! exactly specified by IEEE 754 and are portable, but the transcendental
//! functions are not, and `salman-vm` uses a portable software implementation
//! rather than the platform's.

use std::fmt;
use std::fmt::Write as _;

use crate::time::{Date, DateTime, Duration, TimeOfDay};

/// Largest `STRING` or `WSTRING` salman will hold.
///
/// String values are built from files and from device traffic salman did not
/// produce. A ceiling means a malformed length field cannot become an
/// unbounded allocation.
pub const MAX_STRING_LEN: usize = 65_535;

/// The default maximum length of a `STRING` when a declaration gives none.
///
/// IEC 61131-3 leaves this implementation-defined. 80 is the value used by
/// essentially every dialect salman targets, so it is what the generic dialect
/// uses; a dialect may override it.
pub const DEFAULT_STRING_LEN: u16 = 80;

/// An IEC 61131-3 elementary data type.
///
/// The Edition 3 additions `CHAR`, `WCHAR`, `LDATE`, `LTOD` and `LDT` are not
/// here. They are not implemented, and there is nothing to select them with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElementaryType {
    /// Boolean.
    Bool,
    /// Signed 8-bit integer.
    Sint,
    /// Signed 16-bit integer.
    Int,
    /// Signed 32-bit integer.
    Dint,
    /// Signed 64-bit integer.
    Lint,
    /// Unsigned 8-bit integer.
    Usint,
    /// Unsigned 16-bit integer.
    Uint,
    /// Unsigned 32-bit integer.
    Udint,
    /// Unsigned 64-bit integer.
    Ulint,
    /// 8-bit bit string.
    Byte,
    /// 16-bit bit string.
    Word,
    /// 32-bit bit string.
    Dword,
    /// 64-bit bit string.
    Lword,
    /// 32-bit IEEE 754 binary floating point.
    Real,
    /// 64-bit IEEE 754 binary floating point.
    Lreal,
    /// Duration.
    Time,
    /// Long duration.
    LTime,
    /// Calendar date.
    Date,
    /// Time of day.
    TimeOfDay,
    /// Date and time of day.
    DateAndTime,
    /// Variable-length string of single-byte characters.
    String,
    /// Variable-length string of double-byte characters.
    WString,
}

impl ElementaryType {
    /// The keyword an engineer writes, in the canonical upper case.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bool => "BOOL",
            Self::Sint => "SINT",
            Self::Int => "INT",
            Self::Dint => "DINT",
            Self::Lint => "LINT",
            Self::Usint => "USINT",
            Self::Uint => "UINT",
            Self::Udint => "UDINT",
            Self::Ulint => "ULINT",
            Self::Byte => "BYTE",
            Self::Word => "WORD",
            Self::Dword => "DWORD",
            Self::Lword => "LWORD",
            Self::Real => "REAL",
            Self::Lreal => "LREAL",
            Self::Time => "TIME",
            Self::LTime => "LTIME",
            Self::Date => "DATE",
            Self::TimeOfDay => "TIME_OF_DAY",
            Self::DateAndTime => "DATE_AND_TIME",
            Self::String => "STRING",
            Self::WString => "WSTRING",
        }
    }

    /// Every elementary type salman implements, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Bool,
            Self::Sint,
            Self::Int,
            Self::Dint,
            Self::Lint,
            Self::Usint,
            Self::Uint,
            Self::Udint,
            Self::Ulint,
            Self::Byte,
            Self::Word,
            Self::Dword,
            Self::Lword,
            Self::Real,
            Self::Lreal,
            Self::Time,
            Self::LTime,
            Self::Date,
            Self::TimeOfDay,
            Self::DateAndTime,
            Self::String,
            Self::WString,
        ]
    }

    /// Width in bits for the fixed-width types; `None` for strings, whose width
    /// depends on their declared maximum length.
    #[must_use]
    pub const fn bit_width(self) -> Option<u32> {
        Some(match self {
            Self::Bool => 1,
            Self::Sint | Self::Usint | Self::Byte => 8,
            Self::Int | Self::Uint | Self::Word => 16,
            Self::Dint | Self::Udint | Self::Dword | Self::Real | Self::Date => 32,
            Self::Lint
            | Self::Ulint
            | Self::Lword
            | Self::Lreal
            | Self::Time
            | Self::LTime
            | Self::TimeOfDay
            | Self::DateAndTime => 64,
            Self::String | Self::WString => return None,
        })
    }

    /// Whether this type is a member of `generic`.
    #[must_use]
    pub const fn is_in(self, generic: GenericType) -> bool {
        generic.contains(self)
    }

    /// The value a variable of this type has before anything assigns to it.
    ///
    /// IEC 61131-3 gives a table of default initial values; every numeric type
    /// starts at zero, `BOOL` at `FALSE`, durations at zero, and the date types
    /// at the start of their epoch.
    #[must_use]
    pub fn default_value(self) -> Value {
        match self {
            Self::Bool => Value::Bool(false),
            Self::Sint => Value::Sint(0),
            Self::Int => Value::Int(0),
            Self::Dint => Value::Dint(0),
            Self::Lint => Value::Lint(0),
            Self::Usint => Value::Usint(0),
            Self::Uint => Value::Uint(0),
            Self::Udint => Value::Udint(0),
            Self::Ulint => Value::Ulint(0),
            Self::Byte => Value::Byte(0),
            Self::Word => Value::Word(0),
            Self::Dword => Value::Dword(0),
            Self::Lword => Value::Lword(0),
            Self::Real => Value::Real(0.0),
            Self::Lreal => Value::Lreal(0.0),
            Self::Time => Value::Time(Duration::ZERO),
            Self::LTime => Value::LTime(Duration::ZERO),
            Self::Date => Value::Date(Date::EPOCH),
            Self::TimeOfDay => Value::TimeOfDay(TimeOfDay::MIDNIGHT),
            Self::DateAndTime => Value::DateAndTime(DateTime::EPOCH),
            Self::String => Value::String(Box::default()),
            Self::WString => Value::WString(Box::default()),
        }
    }
}

impl fmt::Display for ElementaryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A node of the IEC 61131-3 generic type hierarchy.
///
/// Used to express the domains of operators and standard functions: `+` is
/// defined on [`GenericType::AnyNum`], `AND` on [`GenericType::AnyBit`], and so
/// on. The containment relations below follow the hierarchy given in the
/// standard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GenericType {
    /// Every type salman has.
    Any,
    /// Every elementary type.
    AnyElementary,
    /// Types with an ordering and a magnitude: numbers and durations.
    AnyMagnitude,
    /// Integers and reals.
    AnyNum,
    /// `REAL` and `LREAL`.
    AnyReal,
    /// Signed and unsigned integers.
    AnyInt,
    /// `SINT`, `INT`, `DINT`, `LINT`.
    AnySigned,
    /// `USINT`, `UINT`, `UDINT`, `ULINT`.
    AnyUnsigned,
    /// `TIME` and `LTIME`.
    AnyDuration,
    /// `BOOL` and the bit-string types.
    AnyBit,
    /// String and character types.
    AnyChars,
    /// `STRING` and `WSTRING`.
    AnyString,
    /// The calendar and clock types.
    AnyDate,
}

impl GenericType {
    /// Whether `ty` belongs to this generic type.
    #[must_use]
    pub const fn contains(self, ty: ElementaryType) -> bool {
        use ElementaryType as E;
        match self {
            Self::Any | Self::AnyElementary => true,
            Self::AnyMagnitude => Self::AnyNum.contains(ty) || Self::AnyDuration.contains(ty),
            Self::AnyNum => Self::AnyInt.contains(ty) || Self::AnyReal.contains(ty),
            Self::AnyReal => matches!(ty, E::Real | E::Lreal),
            Self::AnyInt => Self::AnySigned.contains(ty) || Self::AnyUnsigned.contains(ty),
            Self::AnySigned => matches!(ty, E::Sint | E::Int | E::Dint | E::Lint),
            Self::AnyUnsigned => matches!(ty, E::Usint | E::Uint | E::Udint | E::Ulint),
            Self::AnyDuration => matches!(ty, E::Time | E::LTime),
            Self::AnyBit => {
                matches!(ty, E::Bool | E::Byte | E::Word | E::Dword | E::Lword)
            }
            Self::AnyChars => Self::AnyString.contains(ty),
            Self::AnyString => matches!(ty, E::String | E::WString),
            Self::AnyDate => matches!(ty, E::Date | E::TimeOfDay | E::DateAndTime),
        }
    }

    /// The name used in diagnostics.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Any => "ANY",
            Self::AnyElementary => "ANY_ELEMENTARY",
            Self::AnyMagnitude => "ANY_MAGNITUDE",
            Self::AnyNum => "ANY_NUM",
            Self::AnyReal => "ANY_REAL",
            Self::AnyInt => "ANY_INT",
            Self::AnySigned => "ANY_SIGNED",
            Self::AnyUnsigned => "ANY_UNSIGNED",
            Self::AnyDuration => "ANY_DURATION",
            Self::AnyBit => "ANY_BIT",
            Self::AnyChars => "ANY_CHARS",
            Self::AnyString => "ANY_STRING",
            Self::AnyDate => "ANY_DATE",
        }
    }
}

impl fmt::Display for GenericType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The single quiet NaN salman uses for every `REAL` NaN.
const CANONICAL_NAN_F32_BITS: u32 = 0x7fc0_0000;
/// The single quiet NaN salman uses for every `LREAL` NaN.
const CANONICAL_NAN_F64_BITS: u64 = 0x7ff8_0000_0000_0000;

/// Collapses every `f32` NaN to one bit pattern, leaving other values alone.
///
/// See the module documentation: NaN bit patterns are not portable, and salman
/// hashes traces that contain floating-point values.
#[must_use]
pub fn canonical_f32(x: f32) -> f32 {
    if x.is_nan() {
        f32::from_bits(CANONICAL_NAN_F32_BITS)
    } else {
        x
    }
}

/// Collapses every `f64` NaN to one bit pattern, leaving other values alone.
#[must_use]
pub fn canonical_f64(x: f64) -> f64 {
    if x.is_nan() {
        f64::from_bits(CANONICAL_NAN_F64_BITS)
    } else {
        x
    }
}

/// A runtime value of an elementary type.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `BOOL`.
    Bool(bool),
    /// `SINT`.
    Sint(i8),
    /// `INT`.
    Int(i16),
    /// `DINT`.
    Dint(i32),
    /// `LINT`.
    Lint(i64),
    /// `USINT`.
    Usint(u8),
    /// `UINT`.
    Uint(u16),
    /// `UDINT`.
    Udint(u32),
    /// `ULINT`.
    Ulint(u64),
    /// `BYTE`.
    Byte(u8),
    /// `WORD`.
    Word(u16),
    /// `DWORD`.
    Dword(u32),
    /// `LWORD`.
    Lword(u64),
    /// `REAL`. Any NaN stored here is the canonical one.
    Real(f32),
    /// `LREAL`. Any NaN stored here is the canonical one.
    Lreal(f64),
    /// `TIME`.
    Time(Duration),
    /// `LTIME`.
    LTime(Duration),
    /// `DATE`.
    Date(Date),
    /// `TIME_OF_DAY`.
    TimeOfDay(TimeOfDay),
    /// `DATE_AND_TIME`.
    DateAndTime(DateTime),
    /// `STRING`, as bytes.
    ///
    /// Held as bytes rather than as a Rust `String` because IEC `STRING` is a
    /// sequence of single-byte characters whose encoding is set by the system,
    /// and real projects contain bytes that are not valid UTF-8. Turning those
    /// into replacement characters on import would silently corrupt data.
    String(Box<[u8]>),
    /// `WSTRING`, as 16-bit code units.
    WString(Box<[u16]>),
}

impl Value {
    /// Builds a `REAL`, canonicalising NaN.
    #[must_use]
    pub fn real(x: f32) -> Self {
        Self::Real(canonical_f32(x))
    }

    /// Builds an `LREAL`, canonicalising NaN.
    #[must_use]
    pub fn lreal(x: f64) -> Self {
        Self::Lreal(canonical_f64(x))
    }

    /// Builds a `STRING`, truncating at [`MAX_STRING_LEN`].
    #[must_use]
    pub fn string(bytes: impl AsRef<[u8]>) -> Self {
        let bytes = bytes.as_ref();
        let end = bytes.len().min(MAX_STRING_LEN);
        Self::String(bytes.get(..end).unwrap_or(&[]).into())
    }

    /// Builds a `WSTRING`, truncating at [`MAX_STRING_LEN`] code units.
    #[must_use]
    pub fn wstring(units: impl AsRef<[u16]>) -> Self {
        let units = units.as_ref();
        let end = units.len().min(MAX_STRING_LEN);
        Self::WString(units.get(..end).unwrap_or(&[]).into())
    }

    /// The elementary type of this value.
    #[must_use]
    pub const fn type_of(&self) -> ElementaryType {
        match self {
            Self::Bool(_) => ElementaryType::Bool,
            Self::Sint(_) => ElementaryType::Sint,
            Self::Int(_) => ElementaryType::Int,
            Self::Dint(_) => ElementaryType::Dint,
            Self::Lint(_) => ElementaryType::Lint,
            Self::Usint(_) => ElementaryType::Usint,
            Self::Uint(_) => ElementaryType::Uint,
            Self::Udint(_) => ElementaryType::Udint,
            Self::Ulint(_) => ElementaryType::Ulint,
            Self::Byte(_) => ElementaryType::Byte,
            Self::Word(_) => ElementaryType::Word,
            Self::Dword(_) => ElementaryType::Dword,
            Self::Lword(_) => ElementaryType::Lword,
            Self::Real(_) => ElementaryType::Real,
            Self::Lreal(_) => ElementaryType::Lreal,
            Self::Time(_) => ElementaryType::Time,
            Self::LTime(_) => ElementaryType::LTime,
            Self::Date(_) => ElementaryType::Date,
            Self::TimeOfDay(_) => ElementaryType::TimeOfDay,
            Self::DateAndTime(_) => ElementaryType::DateAndTime,
            Self::String(_) => ElementaryType::String,
            Self::WString(_) => ElementaryType::WString,
        }
    }

    /// The boolean this value carries, if it is a `BOOL`.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The value as a signed 64-bit integer, if it is an integer or bit string.
    ///
    /// Bit strings widen as unsigned, which is what they are; `LWORD` values
    /// above `i64::MAX` therefore do not fit and yield `None`.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Self::Sint(v) => i64::from(*v),
            Self::Int(v) => i64::from(*v),
            Self::Dint(v) => i64::from(*v),
            Self::Lint(v) => *v,
            Self::Usint(v) | Self::Byte(v) => i64::from(*v),
            Self::Uint(v) | Self::Word(v) => i64::from(*v),
            Self::Udint(v) | Self::Dword(v) => i64::from(*v),
            Self::Ulint(v) | Self::Lword(v) => i64::try_from(*v).ok()?,
            _ => return None,
        })
    }

    /// The value as an `f64`, if it is a `REAL` or `LREAL`.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Real(v) => Some(f64::from(*v)),
            Self::Lreal(v) => Some(*v),
            _ => None,
        }
    }

    /// The duration this value carries, if it is `TIME` or `LTIME`.
    #[must_use]
    pub const fn as_duration(&self) -> Option<Duration> {
        match self {
            Self::Time(d) | Self::LTime(d) => Some(*d),
            _ => None,
        }
    }

    /// Appends a canonical byte encoding of this value.
    ///
    /// This encoding is what trace fingerprints are computed over, so it must be
    /// total, injective within a type, and free of any platform dependence.
    /// Integers are little-endian; floats are their IEEE bit patterns, with NaN
    /// already canonicalised by construction.
    pub fn write_canonical_bytes(&self, out: &mut Vec<u8>) {
        // A type tag first, so that `Value::Int(1)` and `Value::Dint(1)` cannot
        // hash the same.
        out.push(self.type_of() as u8);
        match self {
            Self::Bool(v) => out.push(u8::from(*v)),
            Self::Sint(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::Int(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::Dint(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::Lint(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::Usint(v) | Self::Byte(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::Uint(v) | Self::Word(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::Udint(v) | Self::Dword(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::Ulint(v) | Self::Lword(v) => out.extend_from_slice(&v.to_le_bytes()),
            Self::Real(v) => out.extend_from_slice(&canonical_f32(*v).to_bits().to_le_bytes()),
            Self::Lreal(v) => out.extend_from_slice(&canonical_f64(*v).to_bits().to_le_bytes()),
            Self::Time(d) | Self::LTime(d) => out.extend_from_slice(&d.nanos().to_le_bytes()),
            Self::Date(d) => out.extend_from_slice(&d.days_since_epoch().to_le_bytes()),
            Self::TimeOfDay(t) => {
                out.extend_from_slice(&t.nanos_since_midnight().to_le_bytes());
            }
            Self::DateAndTime(d) => out.extend_from_slice(&d.nanos_since_epoch().to_le_bytes()),
            Self::String(bytes) => {
                out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                out.extend_from_slice(bytes);
            }
            Self::WString(units) => {
                out.extend_from_slice(&(units.len() as u32).to_le_bytes());
                out.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
            }
        }
    }

    /// Renders the value the way it appears in a trace file.
    ///
    /// Deterministic across platforms. Reals are rendered with Rust's
    /// shortest-round-trip formatting, which is a pure-Rust algorithm rather
    /// than the platform's `printf`, and always carry a decimal point or an
    /// exponent so that a `REAL` cannot be mistaken for an integer.
    #[must_use]
    pub fn to_trace_string(&self) -> String {
        match self {
            Self::Bool(v) => (if *v { "TRUE" } else { "FALSE" }).to_string(),
            Self::Sint(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::Dint(v) => v.to_string(),
            Self::Lint(v) => v.to_string(),
            Self::Usint(v) => v.to_string(),
            Self::Uint(v) => v.to_string(),
            Self::Udint(v) => v.to_string(),
            Self::Ulint(v) => v.to_string(),
            Self::Byte(v) => format!("16#{v:02X}"),
            Self::Word(v) => format!("16#{v:04X}"),
            Self::Dword(v) => format!("16#{v:08X}"),
            Self::Lword(v) => format!("16#{v:016X}"),
            Self::Real(v) => format_float(f64::from(canonical_f32(*v))),
            Self::Lreal(v) => format_float(canonical_f64(*v)),
            Self::Time(d) | Self::LTime(d) => d.to_iec_literal(),
            Self::Date(d) => d.to_iec_literal(),
            Self::TimeOfDay(t) => t.to_iec_literal(),
            Self::DateAndTime(d) => d.to_iec_literal(),
            Self::String(bytes) => format_string_literal(bytes.iter().map(|b| u32::from(*b)), '\''),
            Self::WString(units) => format_string_literal(units.iter().map(|u| u32::from(*u)), '"'),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_trace_string())
    }
}

/// Formats a float so that it always reads as a float.
///
/// Rust's `{}` renders `1.0f64` as `1`, which in a trace of PLC values would be
/// indistinguishable from `DINT#1`.
fn format_float(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    let s = x.to_string();
    if s.contains(['.', 'e', 'E']) {
        s
    } else {
        s + ".0"
    }
}

/// Renders a string value as an IEC literal with `$` escapes.
///
/// Bytes outside printable ASCII become `$xx`, which keeps trace files pure
/// ASCII and therefore byte-comparable regardless of the encoding a `STRING`
/// happens to hold.
fn format_string_literal(units: impl Iterator<Item = u32>, quote: char) -> String {
    let mut out = String::new();
    out.push(quote);
    for unit in units {
        match unit {
            0x24 => out.push_str("$$"),
            u if u == quote as u32 => {
                out.push('$');
                out.push(quote);
            }
            0x20..=0x7e => {
                if let Some(c) = char::from_u32(unit) {
                    out.push(c);
                }
            }
            u if u <= 0xff => {
                let _ = write!(out, "${u:02X}");
            }
            u => {
                let _ = write!(out, "${u:04X}");
            }
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_elementary_type_has_a_distinct_keyword() {
        let mut names: Vec<&str> = ElementaryType::all().iter().map(|t| t.name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn every_elementary_type_has_a_default_value_of_its_own_type() {
        for ty in ElementaryType::all() {
            assert_eq!(
                ty.default_value().type_of(),
                *ty,
                "{ty} default has the wrong type"
            );
        }
    }

    #[test]
    fn default_values_follow_the_iec_initial_value_table() {
        assert_eq!(ElementaryType::Bool.default_value(), Value::Bool(false));
        assert_eq!(ElementaryType::Dint.default_value(), Value::Dint(0));
        assert_eq!(ElementaryType::Lreal.default_value(), Value::Lreal(0.0));
        assert_eq!(
            ElementaryType::Time.default_value(),
            Value::Time(Duration::ZERO)
        );
        assert_eq!(
            ElementaryType::Date.default_value().to_trace_string(),
            "D#1970-01-01"
        );
        assert_eq!(
            ElementaryType::String.default_value().to_trace_string(),
            "''"
        );
    }

    #[test]
    fn the_generic_hierarchy_matches_the_standard_groupings() {
        use ElementaryType as E;
        use GenericType as G;

        assert!(G::AnySigned.contains(E::Dint));
        assert!(!G::AnySigned.contains(E::Udint));
        assert!(G::AnyUnsigned.contains(E::Udint));
        assert!(G::AnyInt.contains(E::Dint));
        assert!(G::AnyInt.contains(E::Udint));
        assert!(!G::AnyInt.contains(E::Real));
        assert!(G::AnyReal.contains(E::Real));
        assert!(G::AnyReal.contains(E::Lreal));
        assert!(G::AnyNum.contains(E::Dint));
        assert!(G::AnyNum.contains(E::Lreal));
        assert!(!G::AnyNum.contains(E::Time));
        assert!(G::AnyDuration.contains(E::Time));
        assert!(G::AnyMagnitude.contains(E::Time));
        assert!(G::AnyMagnitude.contains(E::Dint));
        assert!(!G::AnyMagnitude.contains(E::Bool));
        // BOOL is a bit string in IEC 61131-3, not a category of its own.
        assert!(G::AnyBit.contains(E::Bool));
        assert!(G::AnyBit.contains(E::Lword));
        assert!(!G::AnyBit.contains(E::Dint));
        assert!(G::AnyDate.contains(E::Date));
        assert!(G::AnyDate.contains(E::DateAndTime));
        assert!(G::AnyString.contains(E::WString));
        assert!(G::AnyChars.contains(E::String));
    }

    #[test]
    fn any_contains_every_elementary_type() {
        for ty in ElementaryType::all() {
            assert!(GenericType::Any.contains(*ty));
            assert!(GenericType::AnyElementary.contains(*ty));
        }
    }

    #[test]
    fn every_elementary_type_belongs_to_at_least_one_narrow_generic() {
        // A type that is in no narrow generic group could not appear in any
        // standard function signature, which would mean salman had modelled it
        // but given it nothing to do.
        let narrow = [
            GenericType::AnyNum,
            GenericType::AnyDuration,
            GenericType::AnyBit,
            GenericType::AnyChars,
            GenericType::AnyDate,
        ];
        for ty in ElementaryType::all() {
            assert!(
                narrow.iter().any(|g| g.contains(*ty)),
                "{ty} is in no narrow generic group"
            );
        }
    }

    #[test]
    fn nan_is_canonicalised_so_traces_cannot_differ_between_architectures() {
        // Two different NaN bit patterns, one with a payload and a sign bit
        // that a processor is free to produce.
        let odd_nan = f32::from_bits(0xffff_dead);
        let other_nan = f32::from_bits(0x7fc0_1234);
        assert!(odd_nan.is_nan() && other_nan.is_nan());

        let a = Value::real(odd_nan);
        let b = Value::real(other_nan);
        let (mut ba, mut bb) = (Vec::new(), Vec::new());
        a.write_canonical_bytes(&mut ba);
        b.write_canonical_bytes(&mut bb);
        assert_eq!(ba, bb, "two NaNs produced different trace bytes");

        let odd = f64::from_bits(0xffff_ffff_dead_beef);
        assert!(odd.is_nan());
        let mut bytes = Vec::new();
        Value::lreal(odd).write_canonical_bytes(&mut bytes);
        let mut expected = Vec::new();
        Value::lreal(f64::NAN).write_canonical_bytes(&mut expected);
        assert_eq!(bytes, expected);
    }

    #[test]
    fn negative_zero_is_preserved_because_it_is_portable_and_meaningful() {
        let neg = Value::lreal(-0.0);
        let pos = Value::lreal(0.0);
        let (mut bn, mut bp) = (Vec::new(), Vec::new());
        neg.write_canonical_bytes(&mut bn);
        pos.write_canonical_bytes(&mut bp);
        assert_ne!(bn, bp, "-0.0 and 0.0 must remain distinguishable");
        assert_eq!(neg.as_f64().map(f64::is_sign_negative), Some(true));
    }

    #[test]
    fn canonical_bytes_are_tagged_so_two_types_holding_one_cannot_collide() {
        let (mut a, mut b) = (Vec::new(), Vec::new());
        Value::Int(1).write_canonical_bytes(&mut a);
        Value::Dint(1).write_canonical_bytes(&mut b);
        assert_ne!(a, b);

        let (mut c, mut d) = (Vec::new(), Vec::new());
        Value::Byte(1).write_canonical_bytes(&mut c);
        Value::Usint(1).write_canonical_bytes(&mut d);
        assert_ne!(
            c, d,
            "BYTE and USINT hold the same bits but are different types"
        );
    }

    #[test]
    fn floats_render_so_they_cannot_be_mistaken_for_integers() {
        assert_eq!(Value::lreal(1.0).to_trace_string(), "1.0");
        assert_eq!(Value::lreal(-0.0).to_trace_string(), "-0.0");
        assert_eq!(Value::lreal(0.5).to_trace_string(), "0.5");
        assert_eq!(Value::real(1.0).to_trace_string(), "1.0");
        assert_eq!(Value::lreal(f64::INFINITY).to_trace_string(), "inf");
        assert_eq!(Value::lreal(f64::NEG_INFINITY).to_trace_string(), "-inf");
        assert_eq!(Value::lreal(f64::NAN).to_trace_string(), "NaN");
    }

    #[test]
    fn bit_strings_render_in_hexadecimal_at_their_declared_width() {
        assert_eq!(Value::Byte(0x0f).to_trace_string(), "16#0F");
        assert_eq!(Value::Word(0x0f).to_trace_string(), "16#000F");
        assert_eq!(Value::Dword(0x0f).to_trace_string(), "16#0000000F");
        assert_eq!(Value::Lword(0x0f).to_trace_string(), "16#000000000000000F");
    }

    #[test]
    fn strings_hold_arbitrary_bytes_without_corrupting_them() {
        // 0x80 is not valid UTF-8. It must survive a round trip.
        let v = Value::string([0x41, 0x80, 0x42]);
        let Value::String(bytes) = &v else {
            panic!("not a string")
        };
        assert_eq!(&**bytes, &[0x41, 0x80, 0x42]);
        assert_eq!(v.to_trace_string(), "'A$80B'");
    }

    #[test]
    fn string_literals_escape_the_dollar_sign_and_the_quote() {
        assert_eq!(Value::string(b"it's $5").to_trace_string(), "'it$'s $$5'");
        assert_eq!(
            Value::wstring([u16::from(b'a'), u16::from(b'"')]).to_trace_string(),
            "\"a$\"\""
        );
    }

    #[test]
    fn strings_are_capped_so_a_bad_length_field_cannot_exhaust_memory() {
        let huge = vec![b'x'; MAX_STRING_LEN + 100];
        let Value::String(bytes) = Value::string(&huge) else {
            panic!("not a string")
        };
        assert_eq!(bytes.len(), MAX_STRING_LEN);
    }

    #[test]
    fn as_i64_refuses_unsigned_values_that_do_not_fit_rather_than_wrapping() {
        assert_eq!(Value::Ulint(u64::MAX).as_i64(), None);
        assert_eq!(Value::Lword(u64::MAX).as_i64(), None);
        assert_eq!(Value::Ulint(7).as_i64(), Some(7));
        assert_eq!(Value::Sint(-3).as_i64(), Some(-3));
        assert_eq!(Value::Real(1.0).as_i64(), None);
    }

    #[test]
    fn bit_widths_match_the_standard() {
        use ElementaryType as E;
        assert_eq!(E::Bool.bit_width(), Some(1));
        assert_eq!(E::Sint.bit_width(), Some(8));
        assert_eq!(E::Int.bit_width(), Some(16));
        assert_eq!(E::Dint.bit_width(), Some(32));
        assert_eq!(E::Lint.bit_width(), Some(64));
        assert_eq!(E::Real.bit_width(), Some(32));
        assert_eq!(E::Lreal.bit_width(), Some(64));
        assert_eq!(E::String.bit_width(), None);
    }
}
