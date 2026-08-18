// SPDX-License-Identifier: Apache-2.0
//! Signatures of the standard function blocks.
//!
//! Citation policy: salman cites IEC 61131-3:2013 (Edition 3.0) clause, table
//! and figure numbers as references only. No IEC text is reproduced. Clause
//! numbers are edition-specific; see `docs/IEC_CITATIONS.md`.
//!
//! These signatures live in the language crate rather than in the runtime,
//! because the shape of `TON` — that it has `IN` and `PT` in and `Q` and `ET`
//! out — is a fact about the language that the type checker needs long before
//! anything runs. The runtime supplies the behaviour; this supplies the shape,
//! and the two agree by construction because there is only one definition.

use salman_core::value::ElementaryType;

/// A standard function block salman implements natively rather than in ST.
///
/// The timers need the simulation clock and the counters need edge detection on
/// their parameters, neither of which is expressible in the subset of ST salman
/// compiles. Implementing them in Rust also lets each edge case named in
/// `docs/CONFORMANCE.md` be a test rather than a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NativeBlock {
    /// `SR`, set dominant. IEC 61131-3:2013 Table 43.
    Sr,
    /// `RS`, reset dominant. IEC 61131-3:2013 Table 43.
    Rs,
    /// `R_TRIG`, rising edge. IEC 61131-3:2013 Table 44.
    RTrig,
    /// `F_TRIG`, falling edge. IEC 61131-3:2013 Table 44.
    FTrig,
    /// `CTU`, count up. IEC 61131-3:2013 Table 45.
    Ctu,
    /// `CTD`, count down. IEC 61131-3:2013 Table 45.
    Ctd,
    /// `CTUD`, count up and down. IEC 61131-3:2013 Table 45.
    Ctud,
    /// `TP`, pulse. IEC 61131-3:2013 Table 46 and Figure 15.
    Tp,
    /// `TON`, on delay. IEC 61131-3:2013 Table 46 and Figure 15.
    Ton,
    /// `TOF`, off delay. IEC 61131-3:2013 Table 46 and Figure 15.
    Tof,
    /// `SEMA`. **Not an IEC 61131-3 standard function block** — see
    /// `docs/CONFORMANCE.md`. Provided for vendor compatibility only.
    Sema,
}

impl NativeBlock {
    /// The name written in Structured Text.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sr => "SR",
            Self::Rs => "RS",
            Self::RTrig => "R_TRIG",
            Self::FTrig => "F_TRIG",
            Self::Ctu => "CTU",
            Self::Ctd => "CTD",
            Self::Ctud => "CTUD",
            Self::Tp => "TP",
            Self::Ton => "TON",
            Self::Tof => "TOF",
            Self::Sema => "SEMA",
        }
    }

    /// Whether IEC 61131-3 defines this block.
    ///
    /// `SEMA` is not in Edition 2 Table 34 nor Edition 3 Table 43, which
    /// between them contain every standard bistable. salman ships it because
    /// existing code uses it, and refuses to describe it as standard.
    #[must_use]
    pub const fn is_iec_standard(self) -> bool {
        !matches!(self, Self::Sema)
    }

    /// Looks a block up by name, case-insensitively.
    #[must_use]
    pub fn lookup(name: &str) -> Option<Self> {
        Self::all()
            .iter()
            .copied()
            .find(|b| b.name().eq_ignore_ascii_case(name))
    }

    /// Every natively implemented block.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Sr,
            Self::Rs,
            Self::RTrig,
            Self::FTrig,
            Self::Ctu,
            Self::Ctd,
            Self::Ctud,
            Self::Tp,
            Self::Ton,
            Self::Tof,
            Self::Sema,
        ]
    }
}

/// What a field of a function block instance is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRole {
    /// Written by the caller, read by the block.
    Input,
    /// Written by the block, read by the caller.
    Output,
    /// The block's own state. Visible in a watch list, because a timer whose
    /// internals you cannot see is a timer you cannot debug.
    Internal,
}

/// One field of a function block instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockField {
    /// The name, as written in Structured Text for inputs and outputs.
    pub name: &'static str,
    /// Its type.
    pub ty: ElementaryType,
    /// What it is for.
    pub role: FieldRole,
}

/// Builds one field descriptor.
const fn f(name: &'static str, ty: ElementaryType, role: FieldRole) -> BlockField {
    BlockField { name, ty, role }
}

use ElementaryType as E;
use FieldRole::{Input, Internal, Output};

/// IEC 61131-3:2013 Table 43 "Standard bistable function blocks".
///
/// The dominant input carries the digit: `SR` is set dominant, so its set
/// input is `S1`. That mnemonic is the one thing that prevents the classic
/// slip of wiring the pair the wrong way round.
const SR_FIELDS: &[BlockField] = &[
    f("S1", E::Bool, Input),
    f("R", E::Bool, Input),
    f("Q1", E::Bool, Output),
];

/// IEC 61131-3:2013 Table 43. `RS` is reset dominant, so its reset is `R1`.
const RS_FIELDS: &[BlockField] = &[
    f("S", E::Bool, Input),
    f("R1", E::Bool, Input),
    f("Q1", E::Bool, Output),
];

/// IEC 61131-3:2013 Table 44 "Standard edge detection function blocks".
const TRIG_FIELDS: &[BlockField] = &[
    f("CLK", E::Bool, Input),
    f("Q", E::Bool, Output),
    f("M", E::Bool, Internal),
];

/// IEC 61131-3:2013 Table 45 "Standard counter function blocks".
///
/// `CU` is declared `BOOL R_EDGE` in the standard, so edge detection is part of
/// the parameter itself: a level that was already high when the instance
/// started does not count. `CU_M` is what remembers the previous level.
const CTU_FIELDS: &[BlockField] = &[
    f("CU", E::Bool, Input),
    f("R", E::Bool, Input),
    f("PV", E::Int, Input),
    f("Q", E::Bool, Output),
    f("CV", E::Int, Output),
    f("CU_M", E::Bool, Internal),
];

/// IEC 61131-3:2013 Table 45. `CTD` has no reset input; `LD` loads the preset.
const CTD_FIELDS: &[BlockField] = &[
    f("CD", E::Bool, Input),
    f("LD", E::Bool, Input),
    f("PV", E::Int, Input),
    f("Q", E::Bool, Output),
    f("CV", E::Int, Output),
    f("CD_M", E::Bool, Internal),
];

/// IEC 61131-3:2013 Table 45.
const CTUD_FIELDS: &[BlockField] = &[
    f("CU", E::Bool, Input),
    f("CD", E::Bool, Input),
    f("R", E::Bool, Input),
    f("LD", E::Bool, Input),
    f("PV", E::Int, Input),
    f("QU", E::Bool, Output),
    f("QD", E::Bool, Output),
    f("CV", E::Int, Output),
    f("CU_M", E::Bool, Internal),
    f("CD_M", E::Bool, Internal),
];

/// IEC 61131-3:2013 Table 46 "Standard timer function blocks" and Figure 15
/// "Standard timer function blocks - timing diagrams (Rules)".
///
/// The internal phase and start instant are ordinary fields rather than hidden
/// runtime state, because a timer whose internals you cannot watch is a timer
/// you cannot debug at three in the morning.
const TIMER_FIELDS: &[BlockField] = &[
    f("IN", E::Bool, Input),
    f("PT", E::Time, Input),
    f("Q", E::Bool, Output),
    f("ET", E::Time, Output),
    f("PHASE", E::Byte, Internal),
    f("START", E::LTime, Internal),
    f("PREV_IN", E::Bool, Internal),
];

/// Not standard. See the module documentation.
const SEMA_FIELDS: &[BlockField] = &[
    f("CLAIM", E::Bool, Input),
    f("RELEASE", E::Bool, Input),
    f("BUSY", E::Bool, Output),
    f("X", E::Bool, Internal),
];

/// The fields of every natively implemented block, in slot order.
///
/// The compiler allocates exactly this many consecutive slots for each
/// instance, so this list is the contract between the compiler and the runtime.
#[must_use]
pub const fn layout(block: NativeBlock) -> &'static [BlockField] {
    match block {
        NativeBlock::Sr => SR_FIELDS,
        NativeBlock::Rs => RS_FIELDS,
        NativeBlock::RTrig | NativeBlock::FTrig => TRIG_FIELDS,
        NativeBlock::Ctu => CTU_FIELDS,
        NativeBlock::Ctd => CTD_FIELDS,
        NativeBlock::Ctud => CTUD_FIELDS,
        NativeBlock::Tp | NativeBlock::Ton | NativeBlock::Tof => TIMER_FIELDS,
        NativeBlock::Sema => SEMA_FIELDS,
    }
}

/// How many slots one instance of a block occupies.
#[must_use]
pub fn slot_count(block: NativeBlock) -> u32 {
    layout(block).len() as u32
}

/// The slot offset of a named field, for the compiler and for tests.
#[must_use]
pub fn field_offset(block: NativeBlock, name: &str) -> Option<u32> {
    layout(block)
        .iter()
        .position(|f| f.name.eq_ignore_ascii_case(name))
        .and_then(|i| u32::try_from(i).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_block_has_a_distinct_name_and_looks_itself_up() {
        let mut names: Vec<&str> = NativeBlock::all().iter().map(|b| b.name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        for block in NativeBlock::all() {
            assert_eq!(NativeBlock::lookup(block.name()), Some(*block));
            assert_eq!(
                NativeBlock::lookup(&block.name().to_lowercase()),
                Some(*block)
            );
        }
        assert_eq!(NativeBlock::lookup("Conveyor_Ctrl"), None);
    }

    #[test]
    fn sema_is_the_only_block_salman_does_not_claim_is_standard() {
        // SEMA is in neither Edition 2 Table 34 nor Edition 3 Table 43, which
        // between them hold every standard bistable. salman ships it because
        // existing code uses it, and says plainly that it is not standard.
        for block in NativeBlock::all() {
            assert_eq!(
                block.is_iec_standard(),
                *block != NativeBlock::Sema,
                "{} is misclassified",
                block.name()
            );
        }
    }

    #[test]
    fn all_ten_standard_blocks_are_present() {
        let standard: Vec<&str> = NativeBlock::all()
            .iter()
            .filter(|b| b.is_iec_standard())
            .map(|b| b.name())
            .collect();
        for expected in [
            "SR", "RS", "R_TRIG", "F_TRIG", "CTU", "CTD", "CTUD", "TP", "TON", "TOF",
        ] {
            assert!(standard.contains(&expected), "{expected} is missing");
        }
        assert_eq!(standard.len(), 10);
    }

    #[test]
    fn every_block_signature_has_unique_field_names_and_at_least_one_output() {
        for block in NativeBlock::all() {
            let fields = layout(*block);
            let mut names: Vec<&str> = fields.iter().map(|f| f.name).collect();
            let count = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), count, "{} has a duplicate field", block.name());
            assert!(
                fields.iter().any(|f| f.role == FieldRole::Output),
                "{} has no output",
                block.name()
            );
            assert_eq!(slot_count(*block), fields.len() as u32);
        }
    }

    #[test]
    fn field_offsets_are_found_case_insensitively() {
        assert_eq!(field_offset(NativeBlock::Ton, "IN"), Some(0));
        assert_eq!(field_offset(NativeBlock::Ton, "pt"), Some(1));
        assert_eq!(field_offset(NativeBlock::Ton, "Q"), Some(2));
        assert_eq!(field_offset(NativeBlock::Ton, "ET"), Some(3));
        assert_eq!(field_offset(NativeBlock::Ton, "nope"), None);
    }
}
