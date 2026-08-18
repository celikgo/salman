// SPDX-License-Identifier: Apache-2.0
//! The salman project file.
//!
//! A project says three things: which source files make up the program, which
//! devices it talks to, and how those devices' registers reach the process
//! image. The third is the one that has no good home anywhere else — putting
//! an IO mapping in code means it cannot be reviewed by the person who knows
//! the plant, and putting it in a vendor tool means it cannot be reviewed at
//! all.
//!
//! ```yaml
//! dialect: generic
//! sources:
//!   - conveyor.st
//! devices:
//!   - name: press
//!     protocol: modbus-tcp
//!     address: "10.4.2.7:502"
//!     unit: 1
//!     map:
//!       - table: input-registers
//!         from: 0
//!         count: 4
//!         to: "%IW0"
//!       - table: coils
//!         from: 0
//!         count: 8
//!         to: "%QX0.0"
//! ```
//!
//! # Unknown keys are refused
//!
//! A misspelled key that was ignored would leave a mapping silently absent,
//! and the program would read zeros from an input that looked configured. The
//! same rule the declarative test format follows, for the same reason.
//!
//! # The direction is not a field
//!
//! Which way data flows is decided by the image area the mapping names: `%I`
//! is read from the device, `%Q` is written to it. It is not a key the file
//! can set, because a file that could say `direction: output` against a `%I`
//! address could say something that has no meaning, and salman would have to
//! decide which half to believe.
//!
//! # Trademark
//!
//! MODBUS® is a registered trademark of Schneider Electric USA, Inc., used
//! under licence by Modbus Organization, Inc. See `LEGAL.md`.

pub mod map;
pub mod spec;

pub use map::{Direction, Flow, Mapping, MappingError};
pub use spec::{Device, Project, ProjectError, Protocol};
