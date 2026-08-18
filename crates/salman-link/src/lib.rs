// SPDX-License-Identifier: Apache-2.0
//! Running a project's IO mappings against a device, at the scan boundaries.
//!
//! A scan is: latch the inputs, run the program, publish the outputs. A
//! mapping hooks either side of that — inputs are read from the device
//! *before* the latch, outputs are written to it *after* the publish — so the
//! program sees one frozen picture of the world for the whole scan, which is
//! the property the process image exists to give.
//!
//! # salman does not drive a plant, and this is where that is enforced
//!
//! An engineering write and a control loop are different things, and the
//! difference matters more here than anywhere else in salman.
//!
//! An **engineering write** is one value, to one register, once, because a
//! person decided to. `salman_modbus_net::Client::write` handles it: ARMED
//! posture, and a confirmation of that specific call.
//!
//! A **control loop** writes its outputs every scan, for ever. Confirming each
//! one is not possible, and a tool that asked once and then wrote ten thousand
//! times would have turned a per-call confirmation into a session-wide licence
//! to drive a plant.
//!
//! So salman does not do it. **Output mappings run against a simulated device
//! only.** Against a live device a link may read, and salman refuses to write —
//! not because it is difficult, but because a tool that drives real outputs is
//! a controller, and salman is not one. It has no watchdog, no failsafe state,
//! no redundancy and no functional-safety assessment, and anything with those
//! properties driving a plant is a category of software this project is not.
//!
//! That refusal is [`LinkError::WouldDriveALiveDevice`], and it is a
//! categorical refusal in the sense `docs/adr/ADR-0002-read-only-by-default.md`
//! uses: there is no posture, flag or configuration key that enables it.

pub mod link;

pub use link::{Link, LinkError, Peer};
