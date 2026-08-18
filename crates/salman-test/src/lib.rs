// SPDX-License-Identifier: Apache-2.0
//! Unit and golden-trace tests for IEC 61131-3 code.
//!
//! This is the part of salman that makes PLC code testable the way software is
//! testable: a declarative test runs a program on salman's own runtime, on any
//! operating system, in a container, with no vendor licence and no Windows
//! virtual machine.
//!
//! PLC unit testing is not new. Every open-source framework salman's authors
//! could find requires a proprietary runtime — TwinCAT, CODESYS, Sysmac Studio
//! or TIA Portal. What is absent, and what this crate is for, is doing it
//! without one.
//!
//! # Not a safety qualification
//!
//! A green test suite here says the code does what the test says, on salman's
//! runtime, under a virtual clock. It is not evidence for a functional safety
//! argument. See `LEGAL.md`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod report;
pub mod runner;
pub mod spec;
pub mod value;

pub use report::{Summary, render_junit, render_text};
pub use runner::{Outcome, Status, run, run_all};
pub use spec::{Step, TestCase, TestFile};
pub use value::{ValueError, ValueSpec};
