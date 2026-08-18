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

pub mod version;

pub use version::VERSION;
