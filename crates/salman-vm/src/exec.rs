// SPDX-License-Identifier: Apache-2.0
//! The interpreter.
//!
//! # Nothing here may panic
//!
//! This loop runs code compiled from a file salman did not write. Every index
//! is checked, every arithmetic operation that can trap is guarded, and every
//! failure becomes a [`Fault`] that stops the task and is reported — never a
//! process abort. A panic would also embed a source line number in the message,
//! which is itself unreproducible across edits and would break the determinism
//! gate.
//!
//! # Decisions salman had to make, and what it chose
//!
//! * **Integer overflow wraps.** Real controllers wrap, and IEC 61131-3 does
//!   not fix the behaviour. `DINT#2147483647 + 1` is therefore
//!   `DINT#-2147483648`, and there is a test that says so. This is a salman
//!   policy, chosen to match hardware rather than to be tidy.
//! * **Integer division by zero is a fault, not a value.** There is no answer
//!   to give, and returning zero would let a division bug reach a plant
//!   disguised as data.
//! * **Real division by zero follows IEEE 754** and yields an infinity, because
//!   IEC 61131-3 references IEEE 754 normatively for `REAL` and `LREAL`.
//! * **A scan has an instruction budget.** `WHILE TRUE DO ; END_WHILE` must
//!   stop the task and say why, not hang a test run. This is the software
//!   equivalent of the watchdog every controller has.

use salman_core::time::Duration;
use salman_core::value::{ElementaryType, Value};

use crate::bytecode::{BinOp, Op, Program, UnOp};
use crate::clock::Clock;
use crate::memory::{AddressError, Memory, SlotId};

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultKind {
    /// An integer division or remainder by zero.
    DivisionByZero,
    /// The operand stack ran out. A compiler bug if it ever happens.
    StackUnderflow,
    /// The operand stack grew past its limit.
    StackOverflow,
    /// An instruction found a value of a type it was not compiled for. A
    /// compiler bug if it ever happens.
    TypeMismatch {
        /// What the instruction expected.
        expected: ElementaryType,
        /// What it found.
        found: ElementaryType,
    },
    /// A slot index outside the program's memory.
    SlotOutOfRange(u32),
    /// A constant index outside the constant pool.
    ConstantOutOfRange(u32),
    /// A directly represented address that could not be resolved.
    Address(String),
    /// An array subscript outside the declared bounds.
    ArrayIndexOutOfRange {
        /// The index that was used.
        index: i64,
        /// The declared lower bound.
        low: i64,
        /// The declared upper bound.
        high: i64,
    },
    /// A jump target outside the routine.
    JumpOutOfRange(u32),
    /// A call to a routine that does not exist.
    RoutineOutOfRange(u32),
    /// Routines nested deeper than the limit.
    CallDepthExceeded(u32),
    /// The scan used more instructions than its budget allowed.
    ///
    /// This is salman's watchdog. A program with a loop that never ends stops
    /// the task and says so, instead of hanging the test run.
    InstructionBudgetExceeded(u64),
    /// An operation salman does not implement for these types.
    Unsupported(String),
}

impl std::fmt::Display for FaultKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DivisionByZero => f.write_str("division by zero"),
            Self::StackUnderflow => f.write_str("operand stack underflow"),
            Self::StackOverflow => f.write_str("operand stack overflow"),
            Self::TypeMismatch { expected, found } => {
                write!(f, "expected a {expected} on the stack, found a {found}")
            }
            Self::SlotOutOfRange(slot) => write!(f, "slot {slot} does not exist"),
            Self::ConstantOutOfRange(index) => write!(f, "constant {index} does not exist"),
            Self::Address(message) => f.write_str(message),
            Self::ArrayIndexOutOfRange { index, low, high } => {
                write!(
                    f,
                    "array index {index} is outside the declared bounds {low}..{high}"
                )
            }
            Self::JumpOutOfRange(target) => write!(f, "jump to {target}, outside the routine"),
            Self::RoutineOutOfRange(index) => write!(f, "routine {index} does not exist"),
            Self::CallDepthExceeded(limit) => {
                write!(f, "routines nested more than {limit} deep")
            }
            Self::InstructionBudgetExceeded(budget) => write!(
                f,
                "scan used more than {budget} instructions; salman stopped it as a watchdog would"
            ),
            Self::Unsupported(what) => write!(f, "salman does not implement {what}"),
        }
    }
}

/// A fault, and where it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// What went wrong.
    pub kind: FaultKind,
    /// The routine it happened in.
    pub routine: String,
    /// The instruction index within that routine.
    pub pc: u32,
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} in {} at instruction {}",
            self.kind, self.routine, self.pc
        )
    }
}

impl std::error::Error for Fault {}

/// Bounds on what one execution may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecLimits {
    /// Instructions one call to [`execute`] may run before the watchdog fires.
    pub max_instructions: u64,
    /// Deepest the operand stack may get.
    pub max_stack: usize,
    /// Deepest routines may nest.
    pub max_call_depth: u32,
}

impl Default for ExecLimits {
    fn default() -> Self {
        // Ten million instructions is roughly a tenth of a second of work on a
        // modern machine — far more than any correct scan needs, and short
        // enough that a runaway loop fails a test rather than hanging it.
        Self {
            max_instructions: 10_000_000,
            max_stack: 1024,
            max_call_depth: 64,
        }
    }
}

/// What one execution did, for the cycle-time statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Executed {
    /// Instructions retired.
    pub instructions: u64,
    /// Deepest the operand stack got.
    pub peak_stack: usize,
}

/// Runs one routine to completion.
///
/// # Errors
///
/// Returns a [`Fault`] describing what stopped it. The memory is left as the
/// faulted program left it, so a debugger can show the state at the fault.
pub fn execute(
    program: &Program,
    memory: &mut Memory,
    clock: &Clock,
    routine: u32,
    limits: ExecLimits,
) -> Result<Executed, Fault> {
    let mut state = Exec {
        program,
        memory,
        clock,
        limits,
        stack: Vec::with_capacity(64),
        instructions: 0,
        peak_stack: 0,
        depth: 0,
    };
    state.run(routine)?;
    Ok(Executed {
        instructions: state.instructions,
        peak_stack: state.peak_stack,
    })
}

struct Exec<'a> {
    program: &'a Program,
    memory: &'a mut Memory,
    clock: &'a Clock,
    limits: ExecLimits,
    stack: Vec<Value>,
    instructions: u64,
    peak_stack: usize,
    depth: u32,
}

impl Exec<'_> {
    fn run(&mut self, index: u32) -> Result<(), Fault> {
        let Some(routine) = self.program.routine(index) else {
            return Err(Fault {
                kind: FaultKind::RoutineOutOfRange(index),
                routine: "<unknown>".into(),
                pc: 0,
            });
        };
        if self.depth >= self.limits.max_call_depth {
            return Err(Fault {
                kind: FaultKind::CallDepthExceeded(self.limits.max_call_depth),
                routine: routine.name.clone(),
                pc: 0,
            });
        }
        self.depth += 1;
        let result = self.run_body(index);
        self.depth -= 1;
        result
    }

    #[allow(clippy::too_many_lines)]
    fn run_body(&mut self, index: u32) -> Result<(), Fault> {
        let mut pc: u32 = 0;
        loop {
            let Some(routine) = self.program.routine(index) else {
                return Err(Fault {
                    kind: FaultKind::RoutineOutOfRange(index),
                    routine: "<unknown>".into(),
                    pc,
                });
            };
            let name = routine.name.as_str();
            let Some(op) = routine.code.get(pc as usize).copied() else {
                // Falling off the end of a routine is a normal return.
                return Ok(());
            };

            self.instructions += 1;
            if self.instructions > self.limits.max_instructions {
                return Err(Fault {
                    kind: FaultKind::InstructionBudgetExceeded(self.limits.max_instructions),
                    routine: name.to_string(),
                    pc,
                });
            }

            let fault = |kind: FaultKind| Fault {
                kind,
                routine: name.to_string(),
                pc,
            };
            let code_len = routine.code.len() as u32;
            let mut next = pc.wrapping_add(1);

            match op {
                Op::Const(i) => {
                    let value = self
                        .program
                        .constants
                        .get(i as usize)
                        .cloned()
                        .ok_or_else(|| fault(FaultKind::ConstantOutOfRange(i)))?;
                    self.push(value).map_err(fault)?;
                }
                Op::Pop => {
                    self.pop().map_err(fault)?;
                }
                Op::Dup => {
                    let top = self.peek().map_err(fault)?.clone();
                    self.push(top).map_err(fault)?;
                }
                Op::LoadSlot(slot) => {
                    let value = self
                        .memory
                        .read_slot(SlotId(slot))
                        .cloned()
                        .ok_or_else(|| fault(FaultKind::SlotOutOfRange(slot)))?;
                    self.push(value).map_err(fault)?;
                }
                Op::StoreSlot(slot) => {
                    let value = self.pop().map_err(fault)?;
                    if !self.memory.write_slot(SlotId(slot), value) {
                        return Err(fault(FaultKind::SlotOutOfRange(slot)));
                    }
                }
                Op::LoadAddress(i) => {
                    let address = self
                        .program
                        .addresses
                        .get(i as usize)
                        .ok_or_else(|| fault(FaultKind::ConstantOutOfRange(i)))?;
                    let value = self
                        .memory
                        .read_address(address)
                        .map_err(|e: AddressError| fault(FaultKind::Address(e.to_string())))?
                        .ok_or_else(|| {
                            fault(FaultKind::Address(format!(
                                "{address} is outside the process image"
                            )))
                        })?;
                    self.push(value).map_err(fault)?;
                }
                Op::StoreAddress(i) => {
                    let value = self.pop().map_err(fault)?;
                    let address = self
                        .program
                        .addresses
                        .get(i as usize)
                        .ok_or_else(|| fault(FaultKind::ConstantOutOfRange(i)))?
                        .clone();
                    let written = self
                        .memory
                        .write_address(&address, &value)
                        .map_err(|e: AddressError| fault(FaultKind::Address(e.to_string())))?;
                    if !written {
                        return Err(fault(FaultKind::Address(format!(
                            "{address} could not be written"
                        ))));
                    }
                }
                Op::LoadIndexed { base, len, low } => {
                    let slot = self.indexed_slot(base, len, low).map_err(fault)?;
                    let value = self
                        .memory
                        .read_slot(slot)
                        .cloned()
                        .ok_or_else(|| fault(FaultKind::SlotOutOfRange(slot.0)))?;
                    self.push(value).map_err(fault)?;
                }
                Op::StoreIndexed { base, len, low } => {
                    let value = self.pop().map_err(fault)?;
                    let slot = self.indexed_slot(base, len, low).map_err(fault)?;
                    if !self.memory.write_slot(slot, value) {
                        return Err(fault(FaultKind::SlotOutOfRange(slot.0)));
                    }
                }
                Op::Binary { op, ty } => {
                    let rhs = self.pop().map_err(fault)?;
                    let lhs = self.pop().map_err(fault)?;
                    let value = binary(op, ty, &lhs, &rhs).map_err(fault)?;
                    self.push(value).map_err(fault)?;
                }
                Op::Unary { op, ty } => {
                    let operand = self.pop().map_err(fault)?;
                    let value = unary(op, ty, &operand).map_err(fault)?;
                    self.push(value).map_err(fault)?;
                }
                Op::Convert { to } => {
                    let operand = self.pop().map_err(fault)?;
                    let value = convert(&operand, to).map_err(fault)?;
                    self.push(value).map_err(fault)?;
                }
                Op::Jump(target) => {
                    if target > code_len {
                        return Err(fault(FaultKind::JumpOutOfRange(target)));
                    }
                    next = target;
                }
                Op::JumpIfFalse(target) | Op::JumpIfTrue(target) => {
                    if target > code_len {
                        return Err(fault(FaultKind::JumpOutOfRange(target)));
                    }
                    let value = self.pop().map_err(fault)?;
                    let Value::Bool(condition) = value else {
                        return Err(fault(FaultKind::TypeMismatch {
                            expected: ElementaryType::Bool,
                            found: value.type_of(),
                        }));
                    };
                    let want = matches!(op, Op::JumpIfTrue(_));
                    if condition == want {
                        next = target;
                    }
                }
                Op::Call(callee) => {
                    self.run(callee)?;
                }
                Op::CallNative { block, base } => {
                    crate::stdfb::step(block, SlotId(base), self.memory, self.clock)
                        .map_err(fault)?;
                }
                Op::Return => return Ok(()),
            }

            pc = next;
        }
    }

    fn indexed_slot(&mut self, base: u32, len: u32, low: i64) -> Result<SlotId, FaultKind> {
        let index_value = self.pop()?;
        let Some(index) = index_value.as_i64() else {
            return Err(FaultKind::TypeMismatch {
                expected: ElementaryType::Dint,
                found: index_value.type_of(),
            });
        };
        let high = low.saturating_add(i64::from(len)).saturating_sub(1);
        if index < low || index > high {
            return Err(FaultKind::ArrayIndexOutOfRange { index, low, high });
        }
        let offset = index.saturating_sub(low);
        let Ok(offset) = u32::try_from(offset) else {
            return Err(FaultKind::ArrayIndexOutOfRange { index, low, high });
        };
        Ok(SlotId(base.saturating_add(offset)))
    }

    fn push(&mut self, value: Value) -> Result<(), FaultKind> {
        if self.stack.len() >= self.limits.max_stack {
            return Err(FaultKind::StackOverflow);
        }
        self.stack.push(value);
        if self.stack.len() > self.peak_stack {
            self.peak_stack = self.stack.len();
        }
        Ok(())
    }

    fn pop(&mut self) -> Result<Value, FaultKind> {
        self.stack.pop().ok_or(FaultKind::StackUnderflow)
    }

    fn peek(&self) -> Result<&Value, FaultKind> {
        self.stack.last().ok_or(FaultKind::StackUnderflow)
    }
}

/// Applies a binary operation.
///
/// `ty` is the type the compiler decided both operands have; a value of any
/// other type is a compiler bug and produces a type-mismatch fault rather than
/// a silent coercion.
fn binary(op: BinOp, ty: ElementaryType, lhs: &Value, rhs: &Value) -> Result<Value, FaultKind> {
    use ElementaryType as E;

    if op.is_comparison() {
        return compare(op, lhs, rhs);
    }

    match ty {
        E::Bool | E::Byte | E::Word | E::Dword | E::Lword => bitwise(op, ty, lhs, rhs),
        E::Sint | E::Int | E::Dint | E::Lint => signed_arith(op, ty, lhs, rhs),
        E::Usint | E::Uint | E::Udint | E::Ulint => unsigned_arith(op, ty, lhs, rhs),
        E::Real | E::Lreal => real_arith(op, ty, lhs, rhs),
        E::Time | E::LTime => duration_arith(op, ty, lhs, rhs),
        _ => Err(FaultKind::Unsupported(format!("{op:?} on {ty}"))),
    }
}

fn expect_i128(value: &Value, ty: ElementaryType) -> Result<i128, FaultKind> {
    value
        .as_i64()
        .map(i128::from)
        .ok_or(FaultKind::TypeMismatch {
            expected: ty,
            found: value.type_of(),
        })
}

fn expect_f64(value: &Value, ty: ElementaryType) -> Result<f64, FaultKind> {
    value.as_f64().ok_or(FaultKind::TypeMismatch {
        expected: ty,
        found: value.type_of(),
    })
}

/// Wraps an integer result into `ty`.
///
/// salman policy: integer overflow wraps, matching what controllers do. IEC
/// 61131-3 does not fix the behaviour.
fn wrap_int(value: i128, ty: ElementaryType) -> Result<Value, FaultKind> {
    use ElementaryType as E;
    Ok(match ty {
        E::Sint => Value::Sint(value as i8),
        E::Int => Value::Int(value as i16),
        E::Dint => Value::Dint(value as i32),
        E::Lint => Value::Lint(value as i64),
        E::Usint => Value::Usint(value as u8),
        E::Uint => Value::Uint(value as u16),
        E::Udint => Value::Udint(value as u32),
        E::Ulint => Value::Ulint(value as u64),
        E::Byte => Value::Byte(value as u8),
        E::Word => Value::Word(value as u16),
        E::Dword => Value::Dword(value as u32),
        E::Lword => Value::Lword(value as u64),
        E::Bool => Value::Bool(value & 1 == 1),
        _ => {
            return Err(FaultKind::Unsupported(format!(
                "integer result of type {ty}"
            )));
        }
    })
}

fn signed_arith(
    op: BinOp,
    ty: ElementaryType,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, FaultKind> {
    let a = expect_i128(lhs, ty)?;
    let b = expect_i128(rhs, ty)?;
    integer_arith(op, ty, a, b)
}

fn unsigned_arith(
    op: BinOp,
    ty: ElementaryType,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, FaultKind> {
    let a = expect_i128(lhs, ty)?;
    let b = expect_i128(rhs, ty)?;
    integer_arith(op, ty, a, b)
}

fn integer_arith(op: BinOp, ty: ElementaryType, a: i128, b: i128) -> Result<Value, FaultKind> {
    let raw = match op {
        BinOp::Add => a.wrapping_add(b),
        BinOp::Sub => a.wrapping_sub(b),
        BinOp::Mul => a.wrapping_mul(b),
        // Integer division by zero has no answer to give. Returning zero would
        // let the bug reach a plant disguised as data.
        BinOp::Div => {
            if b == 0 {
                return Err(FaultKind::DivisionByZero);
            }
            a.wrapping_div(b)
        }
        BinOp::Mod => {
            if b == 0 {
                return Err(FaultKind::DivisionByZero);
            }
            a.wrapping_rem(b)
        }
        BinOp::And | BinOp::Or | BinOp::Xor => {
            return bitwise_i128(op, ty, a, b);
        }
        BinOp::Pow => {
            return Err(FaultKind::Unsupported(
                "exponentiation on integers; the compiler converts to a real first".into(),
            ));
        }
        _ => return Err(FaultKind::Unsupported(format!("{op:?} on {ty}"))),
    };
    wrap_int(raw, ty)
}

fn bitwise(op: BinOp, ty: ElementaryType, lhs: &Value, rhs: &Value) -> Result<Value, FaultKind> {
    if ty == ElementaryType::Bool {
        let (Value::Bool(a), Value::Bool(b)) = (lhs, rhs) else {
            return Err(FaultKind::TypeMismatch {
                expected: ElementaryType::Bool,
                found: lhs.type_of(),
            });
        };
        return Ok(Value::Bool(match op {
            BinOp::And => *a && *b,
            BinOp::Or => *a || *b,
            BinOp::Xor => *a ^ *b,
            _ => return Err(FaultKind::Unsupported(format!("{op:?} on BOOL"))),
        }));
    }
    let a = expect_i128(lhs, ty)?;
    let b = expect_i128(rhs, ty)?;
    bitwise_i128(op, ty, a, b)
}

fn bitwise_i128(op: BinOp, ty: ElementaryType, a: i128, b: i128) -> Result<Value, FaultKind> {
    let raw = match op {
        BinOp::And => a & b,
        BinOp::Or => a | b,
        BinOp::Xor => a ^ b,
        _ => return Err(FaultKind::Unsupported(format!("{op:?} on {ty}"))),
    };
    wrap_int(raw, ty)
}

fn real_arith(op: BinOp, ty: ElementaryType, lhs: &Value, rhs: &Value) -> Result<Value, FaultKind> {
    let a = expect_f64(lhs, ty)?;
    let b = expect_f64(rhs, ty)?;
    // IEC 61131-3 references IEEE 754 normatively for REAL and LREAL, so
    // division by zero yields an infinity rather than a fault. Only the basic
    // operations appear here; all of them are exactly specified by IEEE 754 and
    // are therefore identical on every platform salman supports.
    let result = match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => a / b,
        BinOp::Mod => a % b,
        BinOp::Pow => {
            return Err(FaultKind::Unsupported(
                "exponentiation; salman implements no transcendental functions in this version"
                    .into(),
            ));
        }
        _ => return Err(FaultKind::Unsupported(format!("{op:?} on {ty}"))),
    };
    Ok(if ty == ElementaryType::Real {
        Value::real(result as f32)
    } else {
        Value::lreal(result)
    })
}

fn duration_arith(
    op: BinOp,
    ty: ElementaryType,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, FaultKind> {
    let make = |d: Duration| {
        if ty == ElementaryType::Time {
            Value::Time(d)
        } else {
            Value::LTime(d)
        }
    };
    match (op, lhs.as_duration(), rhs.as_duration()) {
        (BinOp::Add, Some(a), Some(b)) => Ok(make(a.saturating_add(b))),
        (BinOp::Sub, Some(a), Some(b)) => Ok(make(a.checked_sub(b).unwrap_or(if a > b {
            Duration::MAX
        } else {
            Duration::MIN
        }))),
        (BinOp::Mul, Some(a), None) => {
            let factor = rhs.as_i64().ok_or(FaultKind::TypeMismatch {
                expected: ElementaryType::Lint,
                found: rhs.type_of(),
            })?;
            Ok(make(a.checked_mul(factor).unwrap_or(Duration::MAX)))
        }
        (BinOp::Mul, None, Some(b)) => {
            let factor = lhs.as_i64().ok_or(FaultKind::TypeMismatch {
                expected: ElementaryType::Lint,
                found: lhs.type_of(),
            })?;
            Ok(make(b.checked_mul(factor).unwrap_or(Duration::MAX)))
        }
        (BinOp::Div, Some(a), None) => {
            let divisor = rhs.as_i64().ok_or(FaultKind::TypeMismatch {
                expected: ElementaryType::Lint,
                found: rhs.type_of(),
            })?;
            if divisor == 0 {
                return Err(FaultKind::DivisionByZero);
            }
            Ok(make(a.checked_div(divisor).unwrap_or(Duration::ZERO)))
        }
        _ => Err(FaultKind::Unsupported(format!("{op:?} on {ty}"))),
    }
}

/// Compares two values. The result is always `BOOL`.
fn compare(op: BinOp, lhs: &Value, rhs: &Value) -> Result<Value, FaultKind> {
    use std::cmp::Ordering;

    let ordering = match (lhs, rhs) {
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::WString(a), Value::WString(b)) => a.cmp(b),
        (Value::Date(a), Value::Date(b)) => a.cmp(b),
        (Value::TimeOfDay(a), Value::TimeOfDay(b)) => a.cmp(b),
        (Value::DateAndTime(a), Value::DateAndTime(b)) => a.cmp(b),
        _ => {
            if let (Some(a), Some(b)) = (lhs.as_duration(), rhs.as_duration()) {
                a.cmp(&b)
            } else if let (Some(a), Some(b)) = (lhs.as_i64(), rhs.as_i64()) {
                a.cmp(&b)
            } else if let (Some(a), Some(b)) = (lhs.as_f64(), rhs.as_f64()) {
                // NaN compares unequal to everything, including itself, which
                // is what IEEE 754 requires and what an engineer debugging a
                // stuck comparison needs to be true.
                match a.partial_cmp(&b) {
                    Some(o) => o,
                    None => {
                        return Ok(Value::Bool(matches!(op, BinOp::Ne)));
                    }
                }
            } else {
                return Err(FaultKind::TypeMismatch {
                    expected: lhs.type_of(),
                    found: rhs.type_of(),
                });
            }
        }
    };

    Ok(Value::Bool(match op {
        BinOp::Eq => ordering == Ordering::Equal,
        BinOp::Ne => ordering != Ordering::Equal,
        BinOp::Lt => ordering == Ordering::Less,
        BinOp::Le => ordering != Ordering::Greater,
        BinOp::Gt => ordering == Ordering::Greater,
        BinOp::Ge => ordering != Ordering::Less,
        _ => return Err(FaultKind::Unsupported(format!("{op:?} as a comparison"))),
    }))
}

fn unary(op: UnOp, ty: ElementaryType, operand: &Value) -> Result<Value, FaultKind> {
    use ElementaryType as E;
    match op {
        UnOp::Neg => match ty {
            E::Real | E::Lreal => {
                let v = expect_f64(operand, ty)?;
                Ok(if ty == E::Real {
                    Value::real(-v as f32)
                } else {
                    Value::lreal(-v)
                })
            }
            E::Time | E::LTime => {
                let d = operand.as_duration().ok_or(FaultKind::TypeMismatch {
                    expected: ty,
                    found: operand.type_of(),
                })?;
                let negated = Duration::ZERO.checked_sub(d).unwrap_or(Duration::MAX);
                Ok(if ty == E::Time {
                    Value::Time(negated)
                } else {
                    Value::LTime(negated)
                })
            }
            _ => {
                let v = expect_i128(operand, ty)?;
                wrap_int(v.wrapping_neg(), ty)
            }
        },
        UnOp::Not => match operand {
            Value::Bool(v) => Ok(Value::Bool(!v)),
            Value::Byte(v) => Ok(Value::Byte(!v)),
            Value::Word(v) => Ok(Value::Word(!v)),
            Value::Dword(v) => Ok(Value::Dword(!v)),
            Value::Lword(v) => Ok(Value::Lword(!v)),
            other => Err(FaultKind::TypeMismatch {
                expected: ElementaryType::Bool,
                found: other.type_of(),
            }),
        },
    }
}

/// Converts a value to another elementary type.
///
/// Only the conversions the compiler emits appear here. Narrowing truncates,
/// which is what the explicit `*_TO_*` functions do; the type checker is what
/// stops a narrowing happening by accident.
fn convert(value: &Value, to: ElementaryType) -> Result<Value, FaultKind> {
    use ElementaryType as E;
    if value.type_of() == to {
        return Ok(value.clone());
    }
    match to {
        E::Real | E::Lreal => {
            let v = if let Some(i) = value.as_i64() {
                i as f64
            } else if let Some(f) = value.as_f64() {
                f
            } else if let Some(d) = value.as_duration() {
                d.nanos() as f64
            } else {
                return Err(FaultKind::Unsupported(format!(
                    "converting {} to {to}",
                    value.type_of()
                )));
            };
            Ok(if to == E::Real {
                Value::real(v as f32)
            } else {
                Value::lreal(v)
            })
        }
        E::LTime => value.as_duration().map(Value::LTime).ok_or_else(|| {
            FaultKind::Unsupported(format!("converting {} to LTIME", value.type_of()))
        }),
        E::Time => value.as_duration().map(Value::Time).ok_or_else(|| {
            FaultKind::Unsupported(format!("converting {} to TIME", value.type_of()))
        }),
        E::Bool => {
            if let Some(i) = value.as_i64() {
                Ok(Value::Bool(i != 0))
            } else {
                Err(FaultKind::Unsupported(format!(
                    "converting {} to BOOL",
                    value.type_of()
                )))
            }
        }
        _ => {
            if let Some(i) = value.as_i64() {
                wrap_int(i128::from(i), to)
            } else if let Some(f) = value.as_f64() {
                // Float to integer saturates in Rust, which is specified and
                // platform-independent, rather than being undefined as it is
                // in C.
                wrap_int(f as i128, to)
            } else {
                Err(FaultKind::Unsupported(format!(
                    "converting {} to {to}",
                    value.type_of()
                )))
            }
        }
    }
}
