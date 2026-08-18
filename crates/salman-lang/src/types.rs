// SPDX-License-Identifier: Apache-2.0
//! Resolved types, and the rules that govern them.
//!
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. No IEC text is reproduced. Clause
//! numbers are edition-specific; see `docs/IEC_CITATIONS.md`.
//!
//! This module is the part of the front end that decides what is legal, so it
//! is written as data — tables of permitted conversions and operator domains —
//! rather than as a cascade of `if`s. A rule you can print is a rule an
//! engineer can check against the standard.
//!
//! # The three places salman had to decide something the standard did not
//!
//! Each is marked `salman policy` here and in `docs/CONFORMANCE.md`, and each
//! is a named test:
//!
//! 1. **The type of an untyped literal.** `5` is an integer, but which one?
//!    No standard default could be verified from a public source; one vendor
//!    documents `DINT`, another "the smallest possible type". salman makes an
//!    untyped literal take the type its context requires, and fall back to
//!    `DINT` (`LREAL` for reals) when there is no context. That accepts
//!    `x : SINT := 5;` — which every vendor accepts — without inventing a
//!    standard rule.
//! 2. **Whether `BOOL` implicitly widens to the bit strings.** One vendor's
//!    rendering of the conversion figure permits it; another open
//!    implementation excludes `BOOL`. Direct contradiction, unresolved. It is
//!    a dialect setting, permitted in `generic` and refused in strict, and
//!    every diagnostic says which rule it applied.
//! 3. **Arithmetic on durations.** `T#1s * 3` is universally supported and the
//!    standard provides it through overloaded functions on the time types, but
//!    salman could not verify the table number, so the citation says so.

use std::collections::BTreeMap;

use salman_core::ident::IdentKey;
use salman_core::span::Span;
use salman_core::value::{ElementaryType, GenericType};

use crate::ast::{BinaryOp, UnaryOp};

/// Identifies one resolved type in a [`TypeArena`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeId(u32);

impl TypeId {
    /// The index this id addresses.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One field of a structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The field's name, lower-cased for lookup.
    pub name: IdentKey,
    /// Its type.
    pub ty: TypeId,
}

/// The bounds of one array dimension, both inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayBounds {
    /// Lowest index.
    pub low: i64,
    /// Highest index, inclusive.
    pub high: i64,
}

impl ArrayBounds {
    /// How many elements this dimension holds, or `None` if the bounds are
    /// inverted or the count does not fit.
    #[must_use]
    pub const fn len(self) -> Option<u64> {
        if self.high < self.low {
            return None;
        }
        // The subtraction is done in i128 because high - low can overflow i64.
        let span = (self.high as i128) - (self.low as i128) + 1;
        if span > u64::MAX as i128 {
            None
        } else {
            Some(span as u64)
        }
    }

    /// Whether the dimension holds no elements, which salman rejects.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.high < self.low
    }
}

/// What a resolved type actually is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeData {
    /// One of the elementary types.
    Elementary(ElementaryType),
    /// `STRING[n]` or `WSTRING[n]`.
    Str {
        /// `true` for `WSTRING`.
        wide: bool,
        /// The declared maximum length in characters.
        max_len: u32,
    },
    /// `ARRAY [..] OF t`.
    Array {
        /// The element type.
        element: TypeId,
        /// One entry per dimension, outermost first.
        dims: Vec<ArrayBounds>,
    },
    /// A `STRUCT`.
    Struct {
        /// The declared name.
        name: IdentKey,
        /// Its fields, in declaration order.
        fields: Vec<Field>,
    },
    /// An enumeration.
    Enum {
        /// The declared name.
        name: IdentKey,
        /// The type the values are stored in.
        base: ElementaryType,
        /// The values, in declaration order.
        values: Vec<(IdentKey, i64)>,
    },
    /// A subrange such as `INT (0..100)`.
    Subrange {
        /// The underlying integer type.
        base: ElementaryType,
        /// Lowest permitted value.
        low: i64,
        /// Highest permitted value, inclusive.
        high: i64,
    },
    /// An instance of a function block.
    FunctionBlock {
        /// The function block's name.
        name: IdentKey,
        /// Which standard block this is, when it is one.
        native: Option<crate::stdlib::NativeBlock>,
        /// The index of the declaring POU, when it is user-written.
        ///
        /// Exactly one of `native` and `pou` is set for a resolvable instance;
        /// both are `None` only when the type could not be resolved and a
        /// diagnostic has already been reported.
        pou: Option<u32>,
    },
    /// A type that could not be resolved.
    ///
    /// Poison: every rule accepts it, so one bad declaration does not produce
    /// an error at every use of the variable it declared.
    Error,
}

/// Interned resolved types.
///
/// Types are interned so that equality is an id comparison rather than a deep
/// structural walk, and so that a `BTreeMap` keyed on `TypeId` iterates in a
/// stable order — which anything reaching a trace requires.
#[derive(Debug, Clone)]
pub struct TypeArena {
    types: Vec<TypeData>,
    elementary: BTreeMap<ElementaryType, TypeId>,
    error: TypeId,
}

impl Default for TypeArena {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeArena {
    /// An arena preloaded with every elementary type and the error type.
    #[must_use]
    pub fn new() -> Self {
        let mut arena = Self {
            types: Vec::new(),
            elementary: BTreeMap::new(),
            error: TypeId(0),
        };
        arena.error = arena.push(TypeData::Error);
        for ty in ElementaryType::all() {
            let id = arena.push(TypeData::Elementary(*ty));
            arena.elementary.insert(*ty, id);
        }
        arena
    }

    fn push(&mut self, data: TypeData) -> TypeId {
        let id = TypeId(self.types.len() as u32);
        self.types.push(data);
        id
    }

    /// The id of an elementary type. Always succeeds.
    #[must_use]
    pub fn elementary(&self, ty: ElementaryType) -> TypeId {
        self.elementary.get(&ty).copied().unwrap_or(self.error)
    }

    /// The poison type.
    #[must_use]
    pub const fn error(&self) -> TypeId {
        self.error
    }

    /// Interns a type, reusing an identical one if it is already here.
    pub fn intern(&mut self, data: TypeData) -> TypeId {
        if let TypeData::Elementary(ty) = data {
            return self.elementary(ty);
        }
        if let Some(index) = self.types.iter().position(|t| *t == data) {
            return TypeId(index as u32);
        }
        self.push(data)
    }

    /// What a type is.
    #[must_use]
    pub fn get(&self, id: TypeId) -> &TypeData {
        self.types.get(id.index()).unwrap_or(&TypeData::Error)
    }

    /// The elementary type a type behaves as, if it behaves as one.
    ///
    /// A subrange behaves as its base type and an enumeration as the type its
    /// values are stored in, so that arithmetic and comparison rules do not
    /// have to know about either.
    #[must_use]
    pub fn as_elementary(&self, id: TypeId) -> Option<ElementaryType> {
        match self.get(id) {
            TypeData::Elementary(ty) => Some(*ty),
            TypeData::Subrange { base, .. } | TypeData::Enum { base, .. } => Some(*base),
            TypeData::Str { wide, .. } => Some(if *wide {
                ElementaryType::WString
            } else {
                ElementaryType::String
            }),
            _ => None,
        }
    }

    /// Whether this is the poison type.
    #[must_use]
    pub fn is_error(&self, id: TypeId) -> bool {
        matches!(self.get(id), TypeData::Error)
    }

    /// A name for the type, fit for a diagnostic.
    #[must_use]
    pub fn describe(&self, id: TypeId) -> String {
        match self.get(id) {
            TypeData::Elementary(ty) => ty.name().to_string(),
            TypeData::Str { wide, max_len } => {
                format!("{}[{max_len}]", if *wide { "WSTRING" } else { "STRING" })
            }
            TypeData::Array { element, dims } => {
                let bounds: Vec<String> = dims
                    .iter()
                    .map(|d| format!("{}..{}", d.low, d.high))
                    .collect();
                format!(
                    "ARRAY [{}] OF {}",
                    bounds.join(", "),
                    self.describe(*element)
                )
            }
            TypeData::Struct { name, .. } => name.to_string(),
            TypeData::Enum { name, .. } => name.to_string(),
            TypeData::Subrange { base, low, high } => {
                format!("{} ({low}..{high})", base.name())
            }
            TypeData::FunctionBlock { name, .. } => name.to_string(),
            TypeData::Error => "<unknown>".to_string(),
        }
    }

    /// How many types are interned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether the arena holds nothing, which cannot happen after [`new`].
    ///
    /// [`new`]: TypeArena::new
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}

/// Whether `BOOL` may implicitly widen to a bit string.
///
/// Unresolved against the standard: one vendor's rendering of
/// IEC 61131-3:2013 Figure 12 "Supported implicit type conversions" shows
/// `BOOL` widening to `BYTE`, `WORD`, `DWORD` and `LWORD`, while another open
/// implementation states that bit-string widening excludes `BOOL`. salman does
/// not resolve the contradiction; it makes it a setting and names the rule it
/// applied in every diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolWidening {
    /// `BOOL` widens to the bit strings.
    Permitted,
    /// It does not.
    Refused,
}

/// Every implicit conversion salman permits, as an explicit adjacency table.
///
/// IEC 61131-3:2013 Figure 12 "Supported implicit type conversions". This is
/// written out rather than computed from a width rule, because the exceptions
/// are the interesting part: `INT` widens to `REAL` but `DINT` does **not**,
/// since a 24-bit significand cannot hold every 32-bit integer, and `LINT`
/// does not widen to `LREAL` for the same reason one size up.
///
/// `LREAL`, `LINT`, `ULINT`, `LTIME` and `LWORD` are the source of no implicit
/// conversion at all: there is nothing wider to go to.
///
/// **The transcription is from a vendor's rendering of Figure 12, not from the
/// figure itself.** That rendering omits the date, character and string types
/// entirely, so salman permits no implicit conversion for those, and says so
/// rather than claiming the figure is empty there.
const IMPLICIT_CONVERSIONS: &[(ElementaryType, &[ElementaryType])] = {
    use ElementaryType as E;
    &[
        // Reals.
        (E::Real, &[E::Lreal]),
        // Signed integers.
        (E::Sint, &[E::Int, E::Dint, E::Lint, E::Real, E::Lreal]),
        (E::Int, &[E::Dint, E::Lint, E::Real, E::Lreal]),
        (E::Dint, &[E::Lint, E::Lreal]),
        // Unsigned integers. Unsigned widens to signed only when the signed
        // type is strictly wider, so USINT reaches INT but UINT does not.
        (
            E::Usint,
            &[
                E::Uint,
                E::Udint,
                E::Ulint,
                E::Int,
                E::Dint,
                E::Lint,
                E::Real,
                E::Lreal,
            ],
        ),
        (
            E::Uint,
            &[E::Udint, E::Ulint, E::Dint, E::Lint, E::Real, E::Lreal],
        ),
        (E::Udint, &[E::Ulint, E::Lint, E::Lreal]),
        // Durations.
        (E::Time, &[E::LTime]),
        // Bit strings. BOOL's row is gated by BoolWidening.
        (E::Bool, &[E::Byte, E::Word, E::Dword, E::Lword]),
        (E::Byte, &[E::Word, E::Dword, E::Lword]),
        (E::Word, &[E::Dword, E::Lword]),
        (E::Dword, &[E::Lword]),
    ]
};

/// Whether `from` may become `to` without an explicit conversion function.
///
/// A type always converts to itself.
#[must_use]
pub fn implicit_conversion_allowed(
    from: ElementaryType,
    to: ElementaryType,
    bool_widening: BoolWidening,
) -> bool {
    if from == to {
        return true;
    }
    if from == ElementaryType::Bool && bool_widening == BoolWidening::Refused {
        return false;
    }
    IMPLICIT_CONVERSIONS
        .iter()
        .find(|(source, _)| *source == from)
        .is_some_and(|(_, targets)| targets.contains(&to))
}

/// The type two operands must both become for a binary operation, if there is
/// one.
///
/// This is the operand type, not necessarily the result type: a comparison
/// yields `BOOL` whatever its operands were.
#[must_use]
pub fn common_type(
    left: ElementaryType,
    right: ElementaryType,
    bool_widening: BoolWidening,
) -> Option<ElementaryType> {
    if left == right {
        return Some(left);
    }
    if implicit_conversion_allowed(left, right, bool_widening) {
        return Some(right);
    }
    if implicit_conversion_allowed(right, left, bool_widening) {
        return Some(left);
    }
    None
}

/// What a binary operator accepts and what it produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandDomain {
    /// Both operands must be in this generic type.
    Generic(GenericType),
    /// Either both operands are in the generic type, or the operation is one
    /// of the duration forms handled separately.
    NumericOrDuration,
}

/// The result of type-checking an operator application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpResult {
    /// The operation is legal and yields this type.
    Ok(ElementaryType),
    /// The operands have no common type.
    NoCommonType,
    /// The operands are outside the operator's domain; carries what the
    /// operator does accept, for the diagnostic.
    OutsideDomain(GenericType),
}

/// The generic type a binary operator's operands must belong to.
///
/// IEC 61131-3:2013 Table 71 "Operators of the ST language" gives the operator
/// set; the domains come from the standard function tables the operators map
/// onto.
#[must_use]
pub const fn binary_operand_domain(op: BinaryOp) -> GenericType {
    match op {
        // Arithmetic. Durations are handled as a separate case: salman also
        // accepts TIME + TIME, TIME - TIME, TIME * ANY_NUM and TIME / ANY_NUM,
        // which the standard provides through overloaded functions on the time
        // data types whose table number salman could not verify.
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Pow => {
            GenericType::AnyNum
        }
        BinaryOp::Mod => GenericType::AnyInt,
        // AND, OR, XOR operate on bit strings, and BOOL is a bit string.
        BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => GenericType::AnyBit,
        // Comparison and equality are defined across the elementary types.
        BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge | BinaryOp::Eq | BinaryOp::Ne => {
            GenericType::AnyElementary
        }
    }
}

/// Type-checks a binary operation between two elementary types.
#[must_use]
pub fn check_binary(
    op: BinaryOp,
    left: ElementaryType,
    right: ElementaryType,
    bool_widening: BoolWidening,
) -> OpResult {
    use ElementaryType as E;

    // Duration arithmetic, which is not in ANY_NUM.
    if let Some(result) = duration_arithmetic(op, left, right) {
        return OpResult::Ok(result);
    }

    let domain = binary_operand_domain(op);

    // Comparison and equality: operands must have a common type, and the
    // result is BOOL whatever that type was.
    if op.is_comparison() {
        return match common_type(left, right, bool_widening) {
            Some(_) => OpResult::Ok(E::Bool),
            None => OpResult::NoCommonType,
        };
    }

    if !domain.contains(left) || !domain.contains(right) {
        return OpResult::OutsideDomain(domain);
    }

    let Some(common) = common_type(left, right, bool_widening) else {
        return OpResult::NoCommonType;
    };

    // Exponentiation yields a real. IEC provides it through a function whose
    // result is ANY_REAL, so an integer base is promoted rather than producing
    // an integer power.
    //
    // salman policy: the result is LREAL when either operand is LREAL or does
    // not fit REAL, and REAL otherwise. The standard's exact result type for
    // an integer base could not be verified from a public source.
    if op == BinaryOp::Pow {
        let wide = matches!(left, E::Lreal | E::Lint | E::Ulint | E::Dint | E::Udint)
            || matches!(right, E::Lreal);
        return OpResult::Ok(if wide { E::Lreal } else { E::Real });
    }

    OpResult::Ok(common)
}

/// Arithmetic that involves a duration, which lives outside `ANY_NUM`.
fn duration_arithmetic(
    op: BinaryOp,
    left: ElementaryType,
    right: ElementaryType,
) -> Option<ElementaryType> {
    use ElementaryType as E;
    let is_duration = |t: E| matches!(t, E::Time | E::LTime);
    let widest = |a: E, b: E| {
        if a == E::LTime || b == E::LTime {
            E::LTime
        } else {
            E::Time
        }
    };

    match op {
        // TIME + TIME and TIME - TIME.
        BinaryOp::Add | BinaryOp::Sub if is_duration(left) && is_duration(right) => {
            Some(widest(left, right))
        }
        // TIME * number, number * TIME, TIME / number. Dividing a number by a
        // duration is not defined and is deliberately absent.
        BinaryOp::Mul if is_duration(left) && GenericType::AnyNum.contains(right) => Some(left),
        BinaryOp::Mul if is_duration(right) && GenericType::AnyNum.contains(left) => Some(right),
        BinaryOp::Div if is_duration(left) && GenericType::AnyNum.contains(right) => Some(left),
        _ => None,
    }
}

/// Type-checks a unary operation.
#[must_use]
pub fn check_unary(op: UnaryOp, operand: ElementaryType) -> OpResult {
    use ElementaryType as E;
    match op {
        // Negating a duration is meaningful and IEC duration literals may
        // themselves be negative, so ANY_DURATION is in the domain here even
        // though it is not in ANY_NUM.
        UnaryOp::Neg | UnaryOp::Plus => {
            if GenericType::AnyNum.contains(operand) || GenericType::AnyDuration.contains(operand) {
                // Negating an unsigned type cannot produce an unsigned result.
                // salman promotes to the signed type of the same width rather
                // than wrapping, and refuses when there is no such type.
                if op == UnaryOp::Neg && GenericType::AnyUnsigned.contains(operand) {
                    return match operand {
                        E::Usint => OpResult::Ok(E::Int),
                        E::Uint => OpResult::Ok(E::Dint),
                        E::Udint => OpResult::Ok(E::Lint),
                        // There is no signed type wider than LINT to hold
                        // -ULINT, so this is refused rather than wrapped.
                        _ => OpResult::OutsideDomain(GenericType::AnySigned),
                    };
                }
                OpResult::Ok(operand)
            } else {
                OpResult::OutsideDomain(GenericType::AnyNum)
            }
        }
        UnaryOp::Not => {
            if GenericType::AnyBit.contains(operand) {
                OpResult::Ok(operand)
            } else {
                OpResult::OutsideDomain(GenericType::AnyBit)
            }
        }
    }
}

/// The inclusive range of values an integer type can hold.
///
/// `None` for types that are not integers.
#[must_use]
pub const fn integer_range(ty: ElementaryType) -> Option<(i128, i128)> {
    use ElementaryType as E;
    Some(match ty {
        E::Sint => (i8::MIN as i128, i8::MAX as i128),
        E::Int => (i16::MIN as i128, i16::MAX as i128),
        E::Dint => (i32::MIN as i128, i32::MAX as i128),
        E::Lint => (i64::MIN as i128, i64::MAX as i128),
        E::Usint => (0, u8::MAX as i128),
        E::Uint => (0, u16::MAX as i128),
        E::Udint => (0, u32::MAX as i128),
        E::Ulint => (0, u64::MAX as i128),
        E::Byte => (0, u8::MAX as i128),
        E::Word => (0, u16::MAX as i128),
        E::Dword => (0, u32::MAX as i128),
        E::Lword => (0, u64::MAX as i128),
        E::Bool => (0, 1),
        _ => return None,
    })
}

/// Whether `value` fits in `ty`.
#[must_use]
pub const fn integer_fits(value: i128, ty: ElementaryType) -> bool {
    match integer_range(ty) {
        Some((low, high)) => value >= low && value <= high,
        None => false,
    }
}

/// The type an untyped literal takes when nothing in its context requires one.
///
/// **salman policy.** No standard default could be verified from a public
/// source: one vendor documents `DINT` for integers and `LREAL` for reals,
/// another "the smallest possible data type" for integers. salman chooses the
/// widely-documented pair and says it is a choice.
#[must_use]
pub const fn default_literal_type(is_real: bool) -> ElementaryType {
    if is_real {
        ElementaryType::Lreal
    } else {
        ElementaryType::Dint
    }
}

/// Where a type came from, for a diagnostic that has to explain a mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedSpan {
    /// The type.
    pub ty: TypeId,
    /// Where it was written or inferred from.
    pub span: Span,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ElementaryType as E;

    const PERMIT: BoolWidening = BoolWidening::Permitted;

    // --- implicit conversions -------------------------------------------

    #[test]
    fn signed_integers_widen_through_the_whole_chain() {
        assert!(implicit_conversion_allowed(E::Sint, E::Int, PERMIT));
        assert!(implicit_conversion_allowed(E::Sint, E::Dint, PERMIT));
        assert!(implicit_conversion_allowed(E::Sint, E::Lint, PERMIT));
        assert!(implicit_conversion_allowed(E::Int, E::Dint, PERMIT));
        assert!(implicit_conversion_allowed(E::Dint, E::Lint, PERMIT));
    }

    #[test]
    fn int_widens_to_real_but_dint_does_not() {
        // This is the trap in the conversion figure and the reason the table
        // is written out rather than computed: a 24-bit significand holds
        // every 16-bit integer and not every 32-bit one.
        assert!(implicit_conversion_allowed(E::Int, E::Real, PERMIT));
        assert!(!implicit_conversion_allowed(E::Dint, E::Real, PERMIT));
        assert!(implicit_conversion_allowed(E::Dint, E::Lreal, PERMIT));
    }

    #[test]
    fn the_widest_integers_do_not_widen_to_lreal() {
        // Same reason, one size up: a 53-bit significand cannot hold every
        // 64-bit integer.
        assert!(!implicit_conversion_allowed(E::Lint, E::Lreal, PERMIT));
        assert!(!implicit_conversion_allowed(E::Ulint, E::Lreal, PERMIT));
    }

    #[test]
    fn unsigned_widens_to_signed_only_when_the_signed_type_is_strictly_wider() {
        assert!(implicit_conversion_allowed(E::Usint, E::Int, PERMIT));
        assert!(!implicit_conversion_allowed(E::Uint, E::Int, PERMIT));
        assert!(implicit_conversion_allowed(E::Uint, E::Dint, PERMIT));
        assert!(!implicit_conversion_allowed(E::Udint, E::Dint, PERMIT));
        assert!(implicit_conversion_allowed(E::Udint, E::Lint, PERMIT));
    }

    #[test]
    fn nothing_narrows_implicitly() {
        for (from, to) in [
            (E::Lint, E::Dint),
            (E::Dint, E::Int),
            (E::Int, E::Sint),
            (E::Lreal, E::Real),
            (E::Ulint, E::Udint),
            (E::Lword, E::Dword),
        ] {
            assert!(
                !implicit_conversion_allowed(from, to, PERMIT),
                "{} must not narrow to {} implicitly",
                from.name(),
                to.name()
            );
        }
    }

    #[test]
    fn no_real_converts_implicitly_to_an_integer() {
        for real in [E::Real, E::Lreal] {
            for int in [
                E::Sint,
                E::Int,
                E::Dint,
                E::Lint,
                E::Usint,
                E::Uint,
                E::Udint,
                E::Ulint,
            ] {
                assert!(
                    !implicit_conversion_allowed(real, int, PERMIT),
                    "{} must not convert to {}",
                    real.name(),
                    int.name()
                );
            }
        }
    }

    #[test]
    fn numbers_and_bit_strings_do_not_mix_implicitly() {
        assert!(!implicit_conversion_allowed(E::Dint, E::Dword, PERMIT));
        assert!(!implicit_conversion_allowed(E::Dword, E::Dint, PERMIT));
    }

    #[test]
    fn bit_strings_widen_among_themselves() {
        assert!(implicit_conversion_allowed(E::Byte, E::Word, PERMIT));
        assert!(implicit_conversion_allowed(E::Word, E::Dword, PERMIT));
        assert!(implicit_conversion_allowed(E::Dword, E::Lword, PERMIT));
        assert!(!implicit_conversion_allowed(E::Word, E::Byte, PERMIT));
    }

    #[test]
    fn bool_widening_is_a_setting_because_the_sources_contradict_each_other() {
        assert!(implicit_conversion_allowed(
            E::Bool,
            E::Byte,
            BoolWidening::Permitted
        ));
        assert!(!implicit_conversion_allowed(
            E::Bool,
            E::Byte,
            BoolWidening::Refused
        ));
        // BOOL to BOOL is identity and is unaffected by the setting.
        assert!(implicit_conversion_allowed(
            E::Bool,
            E::Bool,
            BoolWidening::Refused
        ));
    }

    #[test]
    fn time_widens_only_to_ltime() {
        assert!(implicit_conversion_allowed(E::Time, E::LTime, PERMIT));
        assert!(!implicit_conversion_allowed(E::LTime, E::Time, PERMIT));
        assert!(!implicit_conversion_allowed(E::Time, E::Lint, PERMIT));
        assert!(!implicit_conversion_allowed(E::Lint, E::Time, PERMIT));
    }

    #[test]
    fn dates_and_strings_have_no_implicit_conversions_at_all() {
        for from in [E::Date, E::TimeOfDay, E::DateAndTime, E::String, E::WString] {
            for to in ElementaryType::all() {
                if from == *to {
                    continue;
                }
                assert!(
                    !implicit_conversion_allowed(from, *to, PERMIT),
                    "{} must not convert to {}",
                    from.name(),
                    to.name()
                );
            }
        }
    }

    #[test]
    fn every_type_converts_to_itself() {
        for ty in ElementaryType::all() {
            assert!(implicit_conversion_allowed(*ty, *ty, BoolWidening::Refused));
        }
    }

    #[test]
    fn implicit_conversion_is_antisymmetric_so_there_is_no_conversion_cycle() {
        // If A widened to B and B widened to A, `common_type` would depend on
        // argument order and the same expression could get two types.
        for a in ElementaryType::all() {
            for b in ElementaryType::all() {
                if a == b {
                    continue;
                }
                let forward = implicit_conversion_allowed(*a, *b, PERMIT);
                let back = implicit_conversion_allowed(*b, *a, PERMIT);
                assert!(
                    !(forward && back),
                    "{} and {} convert to each other",
                    a.name(),
                    b.name()
                );
            }
        }
    }

    #[test]
    fn common_type_is_order_independent() {
        for a in ElementaryType::all() {
            for b in ElementaryType::all() {
                assert_eq!(
                    common_type(*a, *b, PERMIT),
                    common_type(*b, *a, PERMIT),
                    "{} and {} disagree depending on order",
                    a.name(),
                    b.name()
                );
            }
        }
    }

    // --- operators -------------------------------------------------------

    #[test]
    fn arithmetic_takes_numbers_and_produces_their_common_type() {
        assert_eq!(
            check_binary(BinaryOp::Add, E::Int, E::Dint, PERMIT),
            OpResult::Ok(E::Dint)
        );
        assert_eq!(
            check_binary(BinaryOp::Mul, E::Real, E::Real, PERMIT),
            OpResult::Ok(E::Real)
        );
        assert_eq!(
            check_binary(BinaryOp::Sub, E::Int, E::Real, PERMIT),
            OpResult::Ok(E::Real)
        );
    }

    #[test]
    fn arithmetic_refuses_bit_strings_and_strings() {
        assert_eq!(
            check_binary(BinaryOp::Add, E::Word, E::Word, PERMIT),
            OpResult::OutsideDomain(GenericType::AnyNum)
        );
        assert_eq!(
            check_binary(BinaryOp::Add, E::String, E::String, PERMIT),
            OpResult::OutsideDomain(GenericType::AnyNum)
        );
    }

    #[test]
    fn mod_takes_integers_only() {
        assert_eq!(
            check_binary(BinaryOp::Mod, E::Dint, E::Dint, PERMIT),
            OpResult::Ok(E::Dint)
        );
        assert_eq!(
            check_binary(BinaryOp::Mod, E::Real, E::Real, PERMIT),
            OpResult::OutsideDomain(GenericType::AnyInt)
        );
    }

    #[test]
    fn boolean_operators_take_bit_strings_and_keep_their_width() {
        assert_eq!(
            check_binary(BinaryOp::And, E::Bool, E::Bool, PERMIT),
            OpResult::Ok(E::Bool)
        );
        assert_eq!(
            check_binary(BinaryOp::Or, E::Word, E::Word, PERMIT),
            OpResult::Ok(E::Word)
        );
        assert_eq!(
            check_binary(BinaryOp::Xor, E::Byte, E::Word, PERMIT),
            OpResult::Ok(E::Word)
        );
        assert_eq!(
            check_binary(BinaryOp::And, E::Dint, E::Dint, PERMIT),
            OpResult::OutsideDomain(GenericType::AnyBit)
        );
    }

    #[test]
    fn comparisons_yield_bool_whatever_their_operands_were() {
        for op in [
            BinaryOp::Lt,
            BinaryOp::Gt,
            BinaryOp::Le,
            BinaryOp::Ge,
            BinaryOp::Eq,
            BinaryOp::Ne,
        ] {
            assert_eq!(
                check_binary(op, E::Dint, E::Int, PERMIT),
                OpResult::Ok(E::Bool)
            );
            assert_eq!(
                check_binary(op, E::Time, E::Time, PERMIT),
                OpResult::Ok(E::Bool)
            );
            assert_eq!(
                check_binary(op, E::String, E::String, PERMIT),
                OpResult::Ok(E::Bool)
            );
        }
    }

    #[test]
    fn comparing_types_with_no_common_type_is_an_error_not_a_silent_false() {
        assert_eq!(
            check_binary(BinaryOp::Eq, E::Dint, E::String, PERMIT),
            OpResult::NoCommonType
        );
        assert_eq!(
            check_binary(BinaryOp::Lt, E::Time, E::Date, PERMIT),
            OpResult::NoCommonType
        );
    }

    #[test]
    fn duration_arithmetic_is_accepted_because_every_real_program_needs_it() {
        assert_eq!(
            check_binary(BinaryOp::Add, E::Time, E::Time, PERMIT),
            OpResult::Ok(E::Time)
        );
        assert_eq!(
            check_binary(BinaryOp::Sub, E::Time, E::LTime, PERMIT),
            OpResult::Ok(E::LTime)
        );
        assert_eq!(
            check_binary(BinaryOp::Mul, E::Time, E::Dint, PERMIT),
            OpResult::Ok(E::Time)
        );
        assert_eq!(
            check_binary(BinaryOp::Mul, E::Dint, E::Time, PERMIT),
            OpResult::Ok(E::Time)
        );
        assert_eq!(
            check_binary(BinaryOp::Div, E::Time, E::Dint, PERMIT),
            OpResult::Ok(E::Time)
        );
    }

    #[test]
    fn dividing_a_number_by_a_duration_is_not_defined() {
        assert_eq!(
            check_binary(BinaryOp::Div, E::Dint, E::Time, PERMIT),
            OpResult::OutsideDomain(GenericType::AnyNum)
        );
    }

    #[test]
    fn exponentiation_yields_a_real_even_for_integer_operands() {
        assert_eq!(
            check_binary(BinaryOp::Pow, E::Int, E::Int, PERMIT),
            OpResult::Ok(E::Real)
        );
        assert_eq!(
            check_binary(BinaryOp::Pow, E::Real, E::Int, PERMIT),
            OpResult::Ok(E::Real)
        );
        assert_eq!(
            check_binary(BinaryOp::Pow, E::Lreal, E::Int, PERMIT),
            OpResult::Ok(E::Lreal)
        );
        assert_eq!(
            check_binary(BinaryOp::Pow, E::Dint, E::Int, PERMIT),
            OpResult::Ok(E::Lreal)
        );
    }

    #[test]
    fn not_takes_bit_strings_and_keeps_the_width() {
        assert_eq!(check_unary(UnaryOp::Not, E::Bool), OpResult::Ok(E::Bool));
        assert_eq!(check_unary(UnaryOp::Not, E::Word), OpResult::Ok(E::Word));
        assert_eq!(
            check_unary(UnaryOp::Not, E::Dint),
            OpResult::OutsideDomain(GenericType::AnyBit)
        );
    }

    #[test]
    fn negating_an_unsigned_value_promotes_rather_than_wrapping() {
        // -USINT#200 is -200, which no unsigned type holds. Producing USINT#56
        // by wrapping would be a silent wrong answer in a motion calculation.
        assert_eq!(check_unary(UnaryOp::Neg, E::Usint), OpResult::Ok(E::Int));
        assert_eq!(check_unary(UnaryOp::Neg, E::Uint), OpResult::Ok(E::Dint));
        assert_eq!(check_unary(UnaryOp::Neg, E::Udint), OpResult::Ok(E::Lint));
        // Nothing is wider than LINT, so negating a ULINT is refused.
        assert_eq!(
            check_unary(UnaryOp::Neg, E::Ulint),
            OpResult::OutsideDomain(GenericType::AnySigned)
        );
    }

    #[test]
    fn negating_a_duration_is_allowed() {
        assert_eq!(check_unary(UnaryOp::Neg, E::Time), OpResult::Ok(E::Time));
    }

    // --- ranges ----------------------------------------------------------

    #[test]
    fn integer_ranges_match_the_widths_in_the_elementary_type_table() {
        assert_eq!(integer_range(E::Sint), Some((-128, 127)));
        assert_eq!(integer_range(E::Usint), Some((0, 255)));
        assert_eq!(integer_range(E::Int), Some((-32_768, 32_767)));
        assert_eq!(
            integer_range(E::Lint),
            Some((i128::from(i64::MIN), i128::from(i64::MAX)))
        );
        assert_eq!(integer_range(E::Ulint), Some((0, i128::from(u64::MAX))));
        assert_eq!(integer_range(E::Bool), Some((0, 1)));
        assert_eq!(integer_range(E::Real), None);
        assert_eq!(integer_range(E::String), None);
    }

    #[test]
    fn range_checking_catches_the_off_by_one_at_each_bound() {
        assert!(integer_fits(127, E::Sint));
        assert!(!integer_fits(128, E::Sint));
        assert!(integer_fits(-128, E::Sint));
        assert!(!integer_fits(-129, E::Sint));
        assert!(!integer_fits(-1, E::Usint));
        assert!(integer_fits(255, E::Usint));
        assert!(!integer_fits(256, E::Usint));
    }

    // --- the arena --------------------------------------------------------

    #[test]
    fn every_elementary_type_is_preloaded_and_interning_it_again_is_the_same_id() {
        let mut arena = TypeArena::new();
        for ty in ElementaryType::all() {
            let a = arena.elementary(*ty);
            let b = arena.intern(TypeData::Elementary(*ty));
            assert_eq!(a, b, "{} interned to two ids", ty.name());
            assert_eq!(arena.as_elementary(a), Some(*ty));
        }
    }

    #[test]
    fn structurally_identical_types_intern_to_one_id() {
        let mut arena = TypeArena::new();
        let dint = arena.elementary(E::Dint);
        let dims = vec![ArrayBounds { low: 0, high: 9 }];
        let a = arena.intern(TypeData::Array {
            element: dint,
            dims: dims.clone(),
        });
        let b = arena.intern(TypeData::Array {
            element: dint,
            dims,
        });
        assert_eq!(a, b);
        let c = arena.intern(TypeData::Array {
            element: dint,
            dims: vec![ArrayBounds { low: 1, high: 9 }],
        });
        assert_ne!(a, c);
    }

    #[test]
    fn a_subrange_behaves_as_its_base_type_for_the_operator_rules() {
        let mut arena = TypeArena::new();
        let id = arena.intern(TypeData::Subrange {
            base: E::Int,
            low: 0,
            high: 100,
        });
        assert_eq!(arena.as_elementary(id), Some(E::Int));
        assert_eq!(arena.describe(id), "INT (0..100)");
    }

    #[test]
    fn the_error_type_describes_itself_rather_than_pretending_to_be_something() {
        let arena = TypeArena::new();
        assert!(arena.is_error(arena.error()));
        assert_eq!(arena.describe(arena.error()), "<unknown>");
        // An out-of-range id is the error type, not a panic.
        assert!(arena.is_error(TypeId(u32::MAX)));
    }

    #[test]
    fn array_bounds_report_their_length_and_reject_inverted_ranges() {
        assert_eq!(ArrayBounds { low: 0, high: 9 }.len(), Some(10));
        assert_eq!(ArrayBounds { low: -5, high: 5 }.len(), Some(11));
        assert_eq!(ArrayBounds { low: 1, high: 0 }.len(), None);
        assert!(ArrayBounds { low: 1, high: 0 }.is_empty());
        // The whole i64 range does not fit in a u64 count.
        assert_eq!(
            ArrayBounds {
                low: i64::MIN,
                high: i64::MAX
            }
            .len(),
            None
        );
    }

    #[test]
    fn describing_a_type_produces_something_an_engineer_recognises() {
        let mut arena = TypeArena::new();
        let real = arena.elementary(E::Real);
        let array = arena.intern(TypeData::Array {
            element: real,
            dims: vec![
                ArrayBounds { low: 0, high: 9 },
                ArrayBounds { low: 1, high: 4 },
            ],
        });
        assert_eq!(arena.describe(array), "ARRAY [0..9, 1..4] OF REAL");
        let s = arena.intern(TypeData::Str {
            wide: false,
            max_len: 80,
        });
        assert_eq!(arena.describe(s), "STRING[80]");
    }
}
