// SPDX-License-Identifier: Apache-2.0
//! Dialects: the same language, spelled differently by every vendor.
//!
//! Every vendor's Structured Text differs — in what is reserved, in how
//! integers promote, in whether `-2 ** 2` is `4` or `-4`. salman models that as
//! **configuration**, not as conditional compilation, so that:
//!
//! * an engineer can point salman at existing code and say which vendor wrote
//!   it, and
//! * every diagnostic can name *which dialect rule* it applied, which is the
//!   thing somebody porting a plant between vendors actually needs to see.
//!
//! # What exists at this version
//!
//! Two profiles: [`Dialect::generic`] and [`Dialect::strict_iec`]. The vendor
//! profiles named in salman's roadmap — CODESYS, TwinCAT, Siemens SCL, Rockwell
//! ST, OpenPLC, Beremiz — are **not implemented**. [`DialectId`] does not
//! contain them, so there is nothing to select and nothing that half-works.

use std::fmt;

/// A dialect salman can actually parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum DialectId {
    /// A permissive superset of IEC 61131-3 Edition 3, and the default.
    ///
    /// Accepts constructs that are widely implemented but whose standing in the
    /// standard salman could not verify from a public source — lowercase
    /// hexadecimal digits, signed duration literals — because refusing real
    /// code that every vendor accepts helps nobody.
    #[default]
    Generic,
    /// Only what salman could verify is in IEC 61131-3 Edition 3.
    ///
    /// Rejects the constructs [`DialectId::Generic`] accepts on sufferance, and
    /// says which unverified point it is being strict about.
    StrictIec,
}

impl DialectId {
    /// The name written in a project file's `[dialect]` section.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::StrictIec => "iec61131-3:2013-strict",
        }
    }

    /// Parses a dialect name, case-insensitively.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Generic, Self::StrictIec]
            .into_iter()
            .find(|d| d.name().eq_ignore_ascii_case(name))
    }

    /// Every dialect salman implements.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Generic, Self::StrictIec]
    }
}

impl fmt::Display for DialectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How tightly a unary operator binds relative to exponentiation.
///
/// This is a real, load-bearing disagreement rather than a detail:
///
/// * IEC 61131-3 Edition 3 Table 71 lists negation, unary plus and `NOT` as
///   rows 4, 5 and 6, *above* exponentiation at row 7 — and the Edition 3
///   normative Annex A grammar agrees, making the operands of `**` unary
///   expressions. Under that reading `-2 ** 2` is `(-2) ** 2` = `4`.
/// * CODESYS and Beckhoff both publish binding-strength tables in the older
///   Edition 2 order, with exponentiation above negation. Under that reading
///   `-2 ** 2` is `-(2 ** 2)` = `-4`.
///
/// salman follows the Edition 3 reading by default and **warns on any
/// unparenthesised unary operand of `**`**, so nobody is silently bitten by a
/// four-versus-minus-four difference when code moves between tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryPowerBinding {
    /// Unary binds tighter: `-2 ** 2` is `4`. IEC Ed. 3 Table 71 and Annex A.
    UnaryTighter,
    /// Exponentiation binds tighter: `-2 ** 2` is `-4`. CODESYS, Beckhoff.
    PowerTighter,
}

/// The highest [`Dialect::max_nesting_depth`] salman will accept.
///
/// Descending one level of parentheses costs about eleven stack frames through
/// the expression precedence chain, so a bound of a few hundred is already
/// deep enough to exhaust a small thread stack. This ceiling exists because a
/// nesting bound that itself overflows the stack protects nothing.
pub const MAX_NESTING_CEILING: u32 = 256;

/// One dialect's rules.
///
/// Every field is consulted by the lexer, parser or type checker. There are no
/// settings here that do nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialect {
    /// Which dialect this is.
    pub id: DialectId,
    /// Whether `16#ff` is accepted as well as `16#FF`.
    ///
    /// matiec restricts hexadecimal digits to uppercase, citing the standard;
    /// every vendor salman looked at accepts lowercase. Unverified either way.
    pub lowercase_hex_digits: bool,
    /// Whether `T#-5s` is accepted.
    ///
    /// matiec quotes an Edition 3 committee-draft grammar permitting a sign;
    /// CODESYS and Beckhoff both state signs are not permitted. Unresolved.
    pub signed_duration_literals: bool,
    /// Whether `(* ... (* ... *) ... *)` nests.
    ///
    /// Nesting is normative in Edition 3 (Table 3, rows 3a and 3b).
    pub nested_comments: bool,
    /// Whether `//` starts a comment. Normative in Edition 3 (Table 3, row 1).
    pub line_comments: bool,
    /// Whether `/* ... */` is a comment. Normative (Table 3, row 2b).
    pub c_style_block_comments: bool,
    /// How unary operators bind against `**`.
    pub unary_power_binding: UnaryPowerBinding,
    /// Whether `BOOL` implicitly widens to `BYTE`, `WORD`, `DWORD`, `LWORD`.
    ///
    /// Unresolved: a vendor rendering of Edition 3 Figure 12 shows this as
    /// permitted; another open implementation excludes `BOOL` from bit-string
    /// widening. salman permits it in the generic dialect and refuses it in the
    /// strict one, and says which rule it used either way.
    pub bool_widens_to_bit_strings: bool,
    /// Maximum length of a `STRING` whose declaration gives none.
    ///
    /// Implementation-defined by the standard; 80 in practice everywhere.
    pub default_string_length: u16,
    /// Deepest nesting of comments, parentheses and statements accepted.
    ///
    /// A bound, not a preference: source text arrives from files salman did not
    /// write, and an unbounded recursive-descent parser is a stack overflow
    /// waiting for a hostile input.
    ///
    /// The ceiling is not arbitrary. Descending one level of parentheses costs
    /// roughly eleven stack frames through the precedence chain, and a bound of
    /// 4096 was measured overflowing a test thread's stack at around 500 levels
    /// — that is, the bound itself would have been the bug. [`MAX_NESTING_CEILING`]
    /// is the highest value salman accepts, and a test enforces it.
    pub max_nesting_depth: u32,
}

impl Default for Dialect {
    fn default() -> Self {
        Self::generic()
    }
}

impl Dialect {
    /// The permissive default.
    #[must_use]
    pub const fn generic() -> Self {
        Self {
            id: DialectId::Generic,
            lowercase_hex_digits: true,
            signed_duration_literals: true,
            nested_comments: true,
            line_comments: true,
            c_style_block_comments: true,
            unary_power_binding: UnaryPowerBinding::UnaryTighter,
            bool_widens_to_bit_strings: true,
            default_string_length: salman_core::value::DEFAULT_STRING_LEN,
            max_nesting_depth: 128,
        }
    }

    /// Only what salman could verify is in the standard.
    #[must_use]
    pub const fn strict_iec() -> Self {
        Self {
            id: DialectId::StrictIec,
            lowercase_hex_digits: false,
            signed_duration_literals: false,
            bool_widens_to_bit_strings: false,
            ..Self::generic()
        }
    }

    /// The dialect for an id.
    #[must_use]
    pub const fn for_id(id: DialectId) -> Self {
        match id {
            DialectId::Generic => Self::generic(),
            DialectId::StrictIec => Self::strict_iec(),
        }
    }

    /// A sentence naming this dialect's rule on some point, for a diagnostic.
    ///
    /// Every diagnostic that depends on a dialect setting carries one of these,
    /// so the reader learns not only that salman objected but under whose rule.
    #[must_use]
    pub fn rule(&self, rule: &str, detail: &str) -> String {
        format!("{}: {rule} — {detail}", self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_dialect_is_generic() {
        assert_eq!(Dialect::default().id, DialectId::Generic);
        assert_eq!(DialectId::default(), DialectId::Generic);
    }

    #[test]
    fn dialect_names_round_trip_case_insensitively() {
        for id in DialectId::all() {
            assert_eq!(DialectId::from_name(id.name()), Some(*id));
            assert_eq!(DialectId::from_name(&id.name().to_uppercase()), Some(*id));
        }
        assert_eq!(
            DialectId::from_name("codesys"),
            None,
            "unimplemented dialects must not resolve"
        );
    }

    #[test]
    fn the_strict_dialect_differs_from_generic_on_the_unverified_points() {
        let generic = Dialect::generic();
        let strict = Dialect::strict_iec();
        assert!(generic.lowercase_hex_digits && !strict.lowercase_hex_digits);
        assert!(generic.signed_duration_literals && !strict.signed_duration_literals);
        assert!(generic.bool_widens_to_bit_strings && !strict.bool_widens_to_bit_strings);
    }

    #[test]
    fn both_dialects_follow_the_edition_3_unary_power_binding() {
        for id in DialectId::all() {
            assert_eq!(
                Dialect::for_id(*id).unary_power_binding,
                UnaryPowerBinding::UnaryTighter
            );
        }
    }

    #[test]
    fn nesting_depth_is_bounded_in_every_dialect() {
        for id in DialectId::all() {
            let d = Dialect::for_id(*id);
            assert!(
                d.max_nesting_depth > 0 && d.max_nesting_depth <= MAX_NESTING_CEILING,
                "{id} allows a nesting depth that could overflow the stack it is meant to protect"
            );
        }
    }

    #[test]
    fn a_dialect_rule_names_the_dialect_that_produced_it() {
        let text = Dialect::strict_iec().rule("lowercase hex digits", "not accepted");
        assert!(text.starts_with("iec61131-3:2013-strict:"), "{text}");
    }
}
