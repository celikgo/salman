// SPDX-License-Identifier: Apache-2.0
//! Packet captures: reading them, writing them, and decoding what is inside.
//!
//! salman reads a capture the same way it reads a live socket, and the decoder
//! above this layer cannot tell which it is. That is the point of §7 of the
//! build specification, and it is why every protocol decoder in this workspace
//! takes bytes rather than a connection.
//!
//! # Written in-crate, and why
//!
//! Classic pcap is a 24-byte file header, a 16-byte record header, two byte
//! orders and two timestamp scales. Reading it is a few hundred lines with
//! bounds checks, and doing it here buys three things a dependency would not:
//! `unsafe_code = "forbid"` provable across the whole path from file to
//! decoded frame, a fuzz target salman owns so that every finding is
//! actionable, and errors in salman's own diagnostic vocabulary from the first
//! line rather than translated from a foreign enum.
//!
//! The expensive part of this milestone was never the file format. It is TCP
//! reassembly and the link-layer zoo, and no crate supplies correct
//! reassembly.
//!
//! # Sources
//!
//! The format is specified by `draft-ietf-opsawg-pcap`, which is an Internet
//! Draft with intended status Historic. It is **not an RFC**, and nothing here
//! cites an RFC number for it.
//!
//! **§4.1 of that draft is wrong**, and salman does not implement it. Its
//! endianness labels are inverted — a file beginning `D4 C3 B2 A1` is
//! little-endian, and the draft says otherwise — and the octet sequences it
//! gives for the nanosecond magic are the pcapng section-header byte-order
//! magic rather than a permutation of the pcap one. The field definitions in
//! §4 are correct and are what this module implements, checked against real
//! files and against libpcap's own `sf-pcap.c`.

pub mod pcap;

pub use pcap::{CaptureError, LinkType, Reader, Record, Resolution, TimestampScale, Writer};
