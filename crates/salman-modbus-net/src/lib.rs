// SPDX-License-Identifier: Apache-2.0
//! Modbus TCP over real sockets: a client, and a simulator to point it at.
//!
//! `salman-modbus` decides what frames mean and this crate carries them. The
//! split is what makes the protocol testable without hardware, and it is why
//! this crate is small: everything difficult already happened next door.
//!
//! # The posture model is not optional here
//!
//! This is the **first code path in salman that can change a real device**.
//! Until now the posture model in `salman_core::posture` had nothing calling
//! it; it was written first precisely so that this moment could not slip past
//! it.
//!
//! * A **read** is `Effect::ReadDevice` and needs no permission.
//! * A **write to a real device** is `Effect::WriteLiveDevice`: it needs the
//!   ARMED posture *and* a human's confirmation of that specific call.
//!   [`Client::write`] takes a `UserConfirmation` **by value**, so one
//!   confirmation authorises exactly one write and cannot be kept and reused.
//!   That type has no public constructor, so no caller — agent or otherwise —
//!   can manufacture one.
//! * Running the **simulator** is `Effect::WriteSimulated`, which needs the
//!   SIMULATE posture. A simulator whose whole purpose is to accept writes
//!   should not run at a posture that forbids them.
//!
//! # Blocking sockets, and no async runtime
//!
//! This crate uses `std::net` and a thread per connection. salman has no
//! asynchronous runtime and no dependency on one; see
//! `docs/adr/ADR-0013-no-async-runtime-yet.md` for what would change that and
//! which one it would be.
//!
//! # Trademark
//!
//! MODBUS® is a registered trademark of Schneider Electric USA, Inc., used
//! under licence by Modbus Organization, Inc. salman is not certified by, not
//! conformance-tested by, and not affiliated with either. See `LEGAL.md`.

pub mod client;
pub mod server;

pub use client::{Client, ClientError};
pub use server::{Server, ServerError, ServerHandle};
