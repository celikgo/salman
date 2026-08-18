// SPDX-License-Identifier: Apache-2.0
//! The instruction set, and the compiled program it describes.
//!
//! # Static addressing, and why that is not a shortcut
//!
//! Every slot reference in the bytecode is an **absolute** index. There are no
//! stack frames and no dynamic allocation, because IEC 61131-3 does not permit
//! a program organization unit to invoke itself, directly or through a cycle.
//! Every function and every function block instance can therefore have one
//! permanent home in memory, which is how a real controller works and is what
//! makes a scan's memory cost knowable in advance.
//!
//! salman **rejects recursion statically** rather than relying on it not
//! happening; that check is what makes the single-frame layout sound. The
//! prohibition itself is well attested across dialect documentation, but salman
//! could not confirm the governing clause from a public source, so the
//! diagnostic says so.
//!
//! # Operand types are decided at compile time
//!
//! An instruction carries the elementary type it operates on. The runtime does
//! not infer the operation from the values it finds on the stack, because that
//! would make the arithmetic of a program depend on data rather than on the
//! declaration the engineer wrote — and would put the overflow behaviour of
//! `DINT` addition beyond the reach of a test.

use salman_core::value::{ElementaryType, Value};
use salman_lang::address::DirectAddress;

use crate::memory::SlotId;

/// A binary machine operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// Addition.
    Add,
    /// Subtraction.
    Sub,
    /// Multiplication.
    Mul,
    /// Division. Integer division truncates toward zero.
    Div,
    /// Remainder, integers only, with the sign of the dividend.
    Mod,
    /// Exponentiation, on reals.
    Pow,
    /// Bitwise or logical and.
    And,
    /// Bitwise or logical or.
    Or,
    /// Bitwise or logical exclusive or.
    Xor,
    /// Equality.
    Eq,
    /// Inequality.
    Ne,
    /// Less than.
    Lt,
    /// Less than or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Ge,
}

impl BinOp {
    /// Whether the operation yields `BOOL` whatever its operands are.
    #[must_use]
    pub const fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge
        )
    }
}

/// A unary machine operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// Arithmetic negation.
    Neg,
    /// Bitwise or logical complement.
    Not,
}

/// One instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// Pushes a constant from the program's constant pool.
    Const(u32),
    /// Discards the top of the stack.
    Pop,
    /// Duplicates the top of the stack.
    Dup,
    /// Pushes the value in a slot.
    LoadSlot(u32),
    /// Pops a value into a slot.
    StoreSlot(u32),
    /// Pushes the value at a directly represented address.
    LoadAddress(u32),
    /// Pops a value to a directly represented address.
    StoreAddress(u32),
    /// Pushes an element of an array slot; pops the index first.
    ///
    /// Carries the base slot, the number of elements, and the declared lower
    /// bound, so a bounds check can name what went wrong.
    LoadIndexed {
        /// First slot of the array.
        base: u32,
        /// How many elements it has.
        len: u32,
        /// The declared lower bound, for the diagnostic.
        low: i64,
    },
    /// Pops a value and an index and stores into an array slot.
    StoreIndexed {
        /// First slot of the array.
        base: u32,
        /// How many elements it has.
        len: u32,
        /// The declared lower bound, for the diagnostic.
        low: i64,
    },
    /// Applies a binary operation to the two values on top of the stack.
    Binary {
        /// Which operation.
        op: BinOp,
        /// The type both operands have.
        ty: ElementaryType,
    },
    /// Applies a unary operation to the value on top of the stack.
    Unary {
        /// Which operation.
        op: UnOp,
        /// The operand's type.
        ty: ElementaryType,
    },
    /// Converts the value on top of the stack to another elementary type.
    Convert {
        /// The target type.
        to: ElementaryType,
    },
    /// Jumps unconditionally.
    Jump(u32),
    /// Pops a `BOOL` and jumps if it is false.
    JumpIfFalse(u32),
    /// Pops a `BOOL` and jumps if it is true.
    JumpIfTrue(u32),
    /// Calls a compiled routine. Arguments are already in its input slots.
    Call(u32),
    /// Runs a standard function block instance implemented natively.
    CallNative {
        /// Which standard block.
        block: NativeBlock,
        /// The instance's first slot.
        base: u32,
    },
    /// Leaves the current routine.
    Return,
}

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

/// A compiled routine: the body of one POU.
#[derive(Debug, Clone, PartialEq)]
pub struct Routine {
    /// The POU's name, for diagnostics and traces.
    pub name: String,
    /// Its instructions.
    pub code: Vec<Op>,
    /// The slot its result goes in, for a function.
    pub result_slot: Option<SlotId>,
    /// Deepest the operand stack gets, computed at compile time so the
    /// interpreter can reserve once and check against a real bound.
    pub max_stack: u32,
}

/// Everything needed to run a program.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    /// The compiled routines, indexed by the operand of [`Op::Call`].
    pub routines: Vec<Routine>,
    /// Constants referenced by [`Op::Const`].
    pub constants: Vec<Value>,
    /// Directly represented addresses referenced by the address instructions.
    pub addresses: Vec<DirectAddress>,
    /// The type of every slot, in slot order.
    pub slot_types: Vec<ElementaryType>,
    /// The name of every slot, for traces and the watch list.
    pub slot_names: Vec<String>,
    /// How many bytes each process image area needs.
    pub image_bytes: usize,
}

impl Program {
    /// An empty program.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            routines: Vec::new(),
            constants: Vec::new(),
            addresses: Vec::new(),
            slot_types: Vec::new(),
            slot_names: Vec::new(),
            image_bytes: 0,
        }
    }

    /// A routine by index.
    #[must_use]
    pub fn routine(&self, index: u32) -> Option<&Routine> {
        self.routines.get(index as usize)
    }

    /// The index of a routine by name, compared case-insensitively.
    #[must_use]
    pub fn routine_index(&self, name: &str) -> Option<u32> {
        self.routines
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(name))
            .and_then(|i| u32::try_from(i).ok())
    }

    /// The index of a slot by name, compared case-insensitively.
    #[must_use]
    pub fn slot_index(&self, name: &str) -> Option<SlotId> {
        self.slot_names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(name))
            .and_then(|i| u32::try_from(i).ok())
            .map(SlotId)
    }
}

impl Default for Program {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_operations_are_the_ones_that_yield_bool() {
        for op in [
            BinOp::Eq,
            BinOp::Ne,
            BinOp::Lt,
            BinOp::Le,
            BinOp::Gt,
            BinOp::Ge,
        ] {
            assert!(op.is_comparison());
        }
        for op in [BinOp::Add, BinOp::And, BinOp::Pow, BinOp::Mod] {
            assert!(!op.is_comparison());
        }
    }

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
    fn routines_and_slots_are_found_by_name_case_insensitively() {
        let program = Program {
            routines: vec![Routine {
                name: "Conveyor_Ctrl".into(),
                code: vec![Op::Return],
                result_slot: None,
                max_stack: 0,
            }],
            slot_names: vec!["Motor_Run".into()],
            slot_types: vec![ElementaryType::Bool],
            ..Program::new()
        };
        assert_eq!(program.routine_index("conveyor_ctrl"), Some(0));
        assert_eq!(program.routine_index("nothing"), None);
        assert_eq!(program.slot_index("MOTOR_RUN"), Some(SlotId(0)));
        assert!(program.routine(0).is_some());
        assert!(program.routine(1).is_none());
    }
}
