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
use salman_lang::stdlib::NativeBlock;

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
    /// Pushes the value in a slot, addressed absolutely.
    ///
    /// Used for globals and for anything whose home does not depend on which
    /// instance is running.
    LoadSlot(u32),
    /// Pops a value into an absolutely addressed slot.
    StoreSlot(u32),
    /// Pushes the value in a slot at `offset` from the current instance base.
    ///
    /// This is what lets a function block be compiled once and run for every
    /// instance, exactly as a controller keeps one copy of the code and one
    /// block of data per instance.
    LoadLocal(u32),
    /// Pops a value into a slot at `offset` from the current instance base.
    StoreLocal(u32),
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
    /// As [`Op::LoadIndexed`], with `base` relative to the instance base.
    LoadIndexedLocal {
        /// First slot of the array, relative to the instance base.
        base: u32,
        /// How many elements it has.
        len: u32,
        /// The declared lower bound, for the diagnostic.
        low: i64,
    },
    /// As [`Op::StoreIndexed`], with `base` relative to the instance base.
    StoreIndexedLocal {
        /// First slot of the array, relative to the instance base.
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
    /// Checks that the integer on top of the stack lies within an array
    /// dimension's declared bounds, leaving it there.
    ///
    /// Used to bounds-check each dimension of a multi-dimensional subscript
    /// before the dimensions are folded into one linear index. Checking the
    /// linear index alone would let an out-of-range subscript in one dimension
    /// silently alias into another, which is the kind of bug that reads
    /// somebody else's tag.
    BoundsCheck {
        /// The dimension's declared lower bound.
        low: i64,
        /// Its declared upper bound, inclusive.
        high: i64,
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
    /// Calls a compiled routine with an absolute instance base.
    Call {
        /// Which routine.
        routine: u32,
        /// The callee's instance base slot.
        base: u32,
    },
    /// Calls a compiled routine whose instance is nested inside this one.
    CallLocal {
        /// Which routine.
        routine: u32,
        /// The callee's base, relative to the current instance base.
        offset: u32,
    },
    /// Runs a standard function block instance implemented natively.
    CallNative {
        /// Which standard block.
        block: NativeBlock,
        /// The instance's first slot, absolute.
        base: u32,
    },
    /// Runs a standard function block instance nested inside this one.
    CallNativeLocal {
        /// Which standard block.
        block: NativeBlock,
        /// The instance's first slot, relative to the current instance base.
        offset: u32,
    },
    /// Leaves the current routine.
    Return,
}

/// A compiled routine: the body of one POU.
#[derive(Debug, Clone, PartialEq)]
pub struct Routine {
    /// The POU's name, for diagnostics and traces.
    pub name: String,
    /// Its instructions.
    pub code: Vec<Op>,
    /// The slot its result goes in, for a function, relative to its base.
    pub result_slot: Option<SlotId>,
    /// How many slots one instance of this routine's POU occupies.
    pub frame_size: u32,
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
    fn routines_and_slots_are_found_by_name_case_insensitively() {
        let program = Program {
            routines: vec![Routine {
                name: "Conveyor_Ctrl".into(),
                code: vec![Op::Return],
                result_slot: None,
                frame_size: 0,
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
