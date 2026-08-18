// SPDX-License-Identifier: Apache-2.0
//! Claims salman makes about decoded bytes, and the evidence for each.
//!
//! # Why this is not `salman_core::diag`
//!
//! `salman_core::diag` reports what is wrong with a **source file** salman
//! compiled. This reports what salman observed on a **wire it did not
//! control**, and the two differ in the way that matters most: a compiler
//! knows the whole input and is entitled to be certain, while a capture is a
//! partial view of something that already happened, recorded by a tool that
//! made its own decisions about what to keep.
//!
//! So everything here is built around saying **how sure salman is and why**,
//! and around being able to say "I could not tell" as a first-class answer
//! rather than as silence.
//!
//! # The shape, and the two invariants that are structural
//!
//! A finding has three independent axes, which is the idea worth borrowing
//! from Wireshark: *what kind of claim is this*, *how bad is it*, and *what
//! sort of thing was observed*. Collapsing them — as most hand-rolled models
//! do — is how a tool ends up unable to express "I checked this and it was
//! fine", which is the answer that makes the other answers trustworthy.
//!
//! Two rules could have been runtime checks and are instead impossible to
//! break:
//!
//! * **A severity belongs only to an assertion of fault.** [`Finding::fail`]
//!   takes a [`Severity`], and no other constructor does, so a `Pass` with a
//!   severity of `Error` cannot be written down.
//! * **Anything that is not an assertion of fault must say why.**
//!   [`Finding::open`], [`Finding::cannot_determine`] and
//!   [`Finding::not_applicable`] each require a [`Justification`] from a
//!   closed list. "No framing errors were detected" is not something salman
//!   can emit when the truth is "inter-frame timing is not observable from a
//!   TCP capture" — it has to pick one of the reasons, and there is no
//!   free-text escape.
//!
//! # What is deliberately not here
//!
//! **SARIF is not the model.** It is a good export and a poor internal shape
//! for this, because it has no way to say how sure a tool is: `confidence`,
//! `certainty`, `likelihood` and `evidence` appear nowhere in its schema.
//! `result.rank` is priority and `threadFlowLocation.importance` grades trace
//! steps; neither means "I am reasonably sure". That is salman's central
//! requirement, so the model keeps it and an exporter can put it in a property
//! bag and say plainly that no consumer will act on it.
//!
//! **Severity and group are not bit fields of one word.** Wireshark packs them
//! that way for reasons from 1998 and caps its groups at 255 as a result.

pub mod evidence;
pub mod finding;

pub use evidence::{Artifact, Evidence, Observed, TransactionRef};
pub use finding::{
    Confidence, Dedup, DedupScope, Finding, Group, Justification, Kind, NextCheck, Severity,
};
