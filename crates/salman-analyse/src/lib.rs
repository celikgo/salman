// SPDX-License-Identifier: Apache-2.0
//! Reading a capture and saying what happened on it.
//!
//! The layers below this one produce **facts**: these bytes were at this
//! offset, this stream carried these bytes, this frame decoded to this
//! request. This layer produces **claims**, and every claim points back at the
//! facts that support it.
//!
//! That separation is deliberate and is the shape the most credible tooling in
//! this area uses. The decode path is heavily fuzzed and makes no judgements;
//! the analysis makes judgements and can be wrong without the decoders being
//! wrong. It also means a finding salman got wrong is a finding somebody can
//! argue with, because the evidence is attached to it.
//!
//! # What this deliberately does not do
//!
//! It does not try to decide whether a plant is healthy. The most prominent
//! ICS tooling from a national cyber agency stops at structured decoding and
//! adds no anomaly detection at all, and that is a defensible place to stop.
//! salman goes a little further — an exception is worth surfacing, an
//! unanswered request is worth surfacing — and stops well short of guessing.
//! A hundred low-precision findings is how a diagnostic tool loses the reader,
//! and Wireshark's mature Modbus dissectors register four expert items between
//! them.

pub mod modbus;
pub mod timeline;

pub use modbus::{Analysis, Options, analyse_capture};
pub use timeline::{Alignment, Entry, Event, Timeline};
