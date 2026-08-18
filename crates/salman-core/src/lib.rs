//! Core types shared by every other salman crate.
//!
//! This crate holds the things that must mean exactly one thing across the
//! whole platform: the project version, the safety posture, the value and
//! time models, and the capability registry that generates the status table
//! in the README.
//!
//! # Safety boundary
//!
//! salman is an engineering and diagnostic tool. It is **not** a safety PLC,
//! not certified under IEC 61508 / IEC 62061 / ISO 13849, and must never be
//! used to design, validate or replace a safety function. See `README.md`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod capability;
pub mod clause;
pub mod diag;
pub mod hash;
pub mod ident;
pub mod posture;
pub mod rng;
pub mod span;
pub mod time;
pub mod value;
pub mod version;

pub use capability::{Capability, Status};
pub use clause::ClauseRef;
pub use diag::{DiagCode, Diagnostic, Diagnostics, Severity};
pub use hash::{Sha256, sha256, to_hex};
pub use ident::{Ident, IdentKey};
pub use posture::{Effect, Permit, Posture, PostureState};
pub use rng::Rng;
pub use span::{FileId, SourceMap, Span};
pub use time::{Date, DateTime, Duration, TimeOfDay};
pub use value::{ElementaryType, GenericType, Value};
pub use version::VERSION;
