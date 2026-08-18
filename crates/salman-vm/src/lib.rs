// SPDX-License-Identifier: Apache-2.0
//! The salman runtime: a bytecode compiler and a deterministic scan VM.
//!
//! # Not a certified runtime
//!
//! This runtime is for development, testing and simulation. It is **not**
//! certified, assessed or qualified under IEC 61508, IEC 62061, ISO 13849 or
//! any other functional safety standard, and it is not for controlling
//! machinery. See `LEGAL.md`.
//!
//! # Why a bytecode VM
//!
//! Not an AST interpreter, because walking a tree per scan makes the scan cost
//! depend on source shape in ways that are hard to reason about and hard to
//! budget. Not a transpiler, because compiling to another language puts that
//! language's arithmetic, its optimiser and its floating-point behaviour
//! between salman and the determinism promise. A bytecode VM is the only one of
//! the three where salman decides, and can state, exactly what every operation
//! does.
//!
//! # Determinism
//!
//! Single-threaded by design. Floating-point addition is not associative, so
//! any parallel reduction over reals reassociates according to thread
//! scheduling and cannot produce a reproducible answer. Nothing in the
//! evaluation path reads a clock, iterates a hash map, or calls a standard
//! library transcendental function.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod bytecode;
pub mod clock;
pub mod compile;
pub mod exec;
pub mod memory;
// pub mod project;  <- enabled as soon as sema::check lands; see project.rs
pub mod stdfb;
pub mod task;
pub mod trace;

pub use bytecode::{Op, Program, Routine};
pub use clock::{Clock, ClockMode};
pub use compile::{Compiled, compile};
pub use exec::{ExecLimits, Fault, FaultKind, execute};
pub use memory::{Memory, Persistence, ProcessImage, Restart, SlotId};
pub use salman_lang::stdlib::NativeBlock;
pub use task::{ProgramBinding, Runtime, StepOutcome, TaskConfig, TaskTrigger};
pub use trace::{Sample, Signal, Trace};
