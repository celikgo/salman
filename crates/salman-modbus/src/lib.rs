// SPDX-License-Identifier: Apache-2.0
//! Modbus protocol data units, framing and checksums.
//!
//! This crate is **pure**: it opens no socket, reads no file and starts no
//! thread. Bytes go in and typed frames come out, which is what makes the same
//! decoder usable on a live socket and on a capture file. A decoder that could
//! only be exercised against real equipment could not be tested at all.
//!
//! # Sources
//!
//! Every constant here is transcribed from a document salman fetched and can
//! cite. No IEC or Modbus Organization text is reproduced.
//!
//! * **APS** — MODBUS Application Protocol Specification V1.1b3, 26 April 2012
//! * **SL** — MODBUS over Serial Line Specification and Implementation Guide
//!   V1.02, 20 December 2006
//! * **MG** — MODBUS Messaging on TCP/IP Implementation Guide V1.0b,
//!   24 October 2006
//!
//! Where those documents are silent or disagree with themselves, salman makes
//! a decision, marks it as salman's, and never presents it as the
//! specification's. Those places are listed in `docs/CONFORMANCE.md`.
//!
//! # Trademark
//!
//! MODBUS® is a registered trademark of Schneider Electric USA, Inc., used
//! under licence by Modbus Organization, Inc. salman is not certified by, not
//! conformance-tested by, and not affiliated with either. See `LEGAL.md`.

pub mod crc;
pub mod device;
pub mod function;
pub mod limits;
pub mod pdu;
pub mod rtu;
pub mod tcp;

pub use crc::Crc16;
pub use device::{BitTable, Device, Table, WordTable};
pub use function::{ExceptionCode, FunctionCode};
pub use pdu::{Bits, DecodeError, Pdu, Request, Response, Words};
pub use rtu::{RtuAdu, RtuError};
pub use tcp::{Framer, MbapHeader, TcpAdu};
