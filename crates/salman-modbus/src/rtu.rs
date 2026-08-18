// SPDX-License-Identifier: Apache-2.0
//! Modbus RTU: the serial application data unit, and what frames it.
//!
//! An RTU ADU is an address byte, the protocol data unit, and a CRC — with no
//! start delimiter, no end delimiter and no length field.
//!
//! # What frames an RTU message is silence
//!
//! MODBUS over Serial Line V1.02 §2.5.1.1 delimits frames by **time**: at
//! least 3.5 character times of silence between frames, and more than 1.5
//! character times *within* one frame means the frame is incomplete and must
//! be discarded.
//!
//! That has a consequence worth being blunt about. **A byte stream alone
//! cannot be framed as RTU.** Given `01 03 02 00 0A 38 43 01 03 …`, there is
//! no way to know from the bytes where one frame ended, because any byte could
//! be an address and any two could be a CRC. Framing needs the arrival times,
//! and those come from the transport, not from this module. [`RtuAdu::decode`]
//! therefore takes a frame that something else has already delimited, and says
//! so rather than pretending to a framer it cannot have.
//!
//! This is also why "RTU over TCP" is not a Modbus Organization protocol and
//! is never auto-detected here: TCP does not preserve the silence that RTU
//! frames with, and an RTU frame beginning `00 03` is indistinguishable from
//! an MBAP transaction identifier of zero.

use crate::crc::Crc16;
use crate::limits::{MAX_PDU, MAX_RTU_ADU};
use crate::pdu::Pdu;

/// The address that means every device on the bus. SL §2.2.
///
/// A broadcast is executed and **not** answered. There is no broadcast on
/// Modbus TCP: there, unit identifier zero means "this device".
pub const BROADCAST: u8 = 0;

/// The largest address a device may be given. SL §2.2.
pub const MAX_DEVICE_ADDRESS: u8 = 247;

/// One serial application data unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtuAdu {
    /// The device address, or [`BROADCAST`].
    pub address: u8,
    /// The protocol data unit.
    pub pdu: Pdu,
}

impl RtuAdu {
    /// Wraps a PDU for sending.
    #[must_use]
    pub const fn new(address: u8, pdu: Pdu) -> Self {
        Self { address, pdu }
    }

    /// Whether this is addressed to every device on the bus.
    #[must_use]
    pub const fn is_broadcast(&self) -> bool {
        self.address == BROADCAST
    }

    /// The whole ADU: address, PDU, then the CRC low byte first.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1 + self.pdu.len() + 2);
        bytes.push(self.address);
        bytes.extend_from_slice(self.pdu.as_bytes());
        bytes.extend_from_slice(&Crc16::of(&bytes).to_wire());
        bytes
    }

    /// Reads an ADU from a frame something else has already delimited.
    ///
    /// The frame must be exactly one ADU, CRC included. See the module
    /// documentation for why this function cannot find frame boundaries
    /// itself.
    ///
    /// # Errors
    ///
    /// Returns [`RtuError`]. A CRC failure means the frame was corrupted in
    /// transit, and SL §2.4.2 has the receiver **say nothing at all** — not
    /// answer with an exception. Returning an error here rather than a
    /// response is what makes that possible.
    pub fn decode(frame: &[u8]) -> Result<Self, RtuError> {
        // One address byte, at least one PDU byte, two CRC bytes.
        if frame.len() < 4 {
            return Err(RtuError::TooShort {
                length: frame.len(),
            });
        }
        if frame.len() > MAX_RTU_ADU {
            return Err(RtuError::TooLong {
                length: frame.len(),
            });
        }
        let split = frame.len() - 2;
        let (body, crc_bytes) = frame.split_at(split);
        let found = match crc_bytes {
            [low, high] => Crc16::from_wire([*low, *high]),
            _ => {
                return Err(RtuError::TooShort {
                    length: frame.len(),
                });
            }
        };
        let computed = Crc16::of(body);
        if computed != found {
            return Err(RtuError::CrcMismatch { computed, found });
        }
        let (address, pdu_bytes) = body.split_first().ok_or(RtuError::TooShort {
            length: frame.len(),
        })?;
        let pdu = Pdu::from_bytes(pdu_bytes).ok_or(RtuError::PduLength {
            length: pdu_bytes.len(),
        })?;
        Ok(Self {
            address: *address,
            pdu,
        })
    }
}

/// Why a serial frame could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtuError {
    /// Too short to hold an address, a function code and a CRC.
    TooShort {
        /// How many bytes arrived.
        length: usize,
    },
    /// Longer than a serial frame may be.
    TooLong {
        /// How many bytes arrived.
        length: usize,
    },
    /// The CRC did not match, so the frame was corrupted on the wire.
    CrcMismatch {
        /// What the frame's own bytes give.
        computed: Crc16,
        /// What the frame carried.
        found: Crc16,
    },
    /// The protocol data unit is not a length a PDU may have.
    PduLength {
        /// How many bytes were left for it.
        length: usize,
    },
}

impl RtuError {
    /// Whether a server must answer nothing at all.
    ///
    /// SL §2.4.2: a frame that fails its check is discarded in silence, and
    /// the client is left to time out. Answering an exception to a bad CRC is
    /// the single most common defect in a hand-written Modbus server — it
    /// tells the client that a device at that address received something,
    /// which is exactly what a corrupted frame does not establish.
    #[must_use]
    pub const fn requires_silence(self) -> bool {
        // Every variant means the frame could not be trusted, and a frame that
        // could not be trusted was not necessarily addressed to this device.
        match self {
            Self::CrcMismatch { .. }
            | Self::TooShort { .. }
            | Self::TooLong { .. }
            | Self::PduLength { .. } => true,
        }
    }
}

impl core::fmt::Display for RtuError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort { length } => write!(
                f,
                "a serial frame of {length} bytes cannot hold an address, a function code and a CRC"
            ),
            Self::TooLong { length } => write!(
                f,
                "a serial frame of {length} bytes is longer than the {MAX_RTU_ADU} a Modbus frame may be"
            ),
            Self::CrcMismatch { computed, found } => write!(
                f,
                "the frame carries CRC 0x{:04X} and its bytes give 0x{:04X}",
                found.0, computed.0
            ),
            Self::PduLength { length } => write!(
                f,
                "{length} bytes is not a length a protocol data unit may have, which is 1 to {MAX_PDU}"
            ),
        }
    }
}

impl core::error::Error for RtuError {}

/// What an address means. SL §2.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressKind {
    /// Zero: every device executes and none answers.
    Broadcast,
    /// 1 to 247: one device.
    Device,
    /// 248 to 255: reserved by the specification.
    Reserved,
}

/// Classifies a serial address.
#[must_use]
pub const fn classify_address(address: u8) -> AddressKind {
    match address {
        BROADCAST => AddressKind::Broadcast,
        1..=MAX_DEVICE_ADDRESS => AddressKind::Device,
        _ => AddressKind::Reserved,
    }
}

/// The inter-frame and inter-character silences RTU uses to delimit frames.
///
/// # What is the specification's and what is salman's
///
/// SL §2.5.1.1 gives the **character format** — eleven bits in RTU: one start
/// bit, eight data bits, one parity bit and one stop bit, or two stop bits
/// when there is no parity, which is still eleven — and the **rule**: 3.5
/// character times between frames, 1.5 within one.
///
/// Multiplying the two is salman's arithmetic, not a published table.
///
/// Above 19200 baud SL gives fixed values instead, and states them as a
/// recommendation rather than a requirement. salman follows the
/// recommendation and [`Timing::is_recommended_rather_than_required`] says
/// when it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Nanoseconds of silence that separate two frames.
    pub inter_frame_ns: u64,
    /// Nanoseconds of silence that mean a frame is incomplete.
    pub inter_character_ns: u64,
    /// The baud rate these were computed for.
    pub baud: u32,
}

impl Timing {
    /// Bits per character on an RTU line. SL §2.5.1.
    pub const BITS_PER_CHARACTER: u64 = 11;

    /// The baud rate above which SL recommends fixed times instead.
    pub const FIXED_ABOVE_BAUD: u32 = 19_200;

    /// The recommended inter-frame silence above 19200 baud: 1.750 ms.
    pub const FIXED_INTER_FRAME_NS: u64 = 1_750_000;

    /// The recommended inter-character silence above 19200 baud: 750 µs.
    pub const FIXED_INTER_CHARACTER_NS: u64 = 750_000;

    /// The two silences for a baud rate.
    ///
    /// Returns `None` for a baud rate of zero, which has no character time.
    #[must_use]
    pub const fn for_baud(baud: u32) -> Option<Self> {
        const NS_PER_SECOND: u64 = 1_000_000_000;
        if baud == 0 {
            return None;
        }
        if baud > Self::FIXED_ABOVE_BAUD {
            return Some(Self {
                inter_frame_ns: Self::FIXED_INTER_FRAME_NS,
                inter_character_ns: Self::FIXED_INTER_CHARACTER_NS,
                baud,
            });
        }
        // 3.5 and 1.5 characters, in nanoseconds, without floating point.
        // 3.5 x 11 = 38.5 bits and 1.5 x 11 = 16.5, held as tenths so the
        // fraction survives. The multiplication comes before the division:
        // dividing first would truncate the bit time and then scale the error
        // up by 38.5, which at 9600 baud is a whole microsecond adrift.
        let bit_rate = baud as u64 * 10;
        Some(Self {
            inter_frame_ns: NS_PER_SECOND * 385 / bit_rate,
            inter_character_ns: NS_PER_SECOND * 165 / bit_rate,
            baud,
        })
    }

    /// Whether these times come from SL's recommendation rather than from the
    /// character-time rule.
    #[must_use]
    pub const fn is_recommended_rather_than_required(&self) -> bool {
        self.baud > Self::FIXED_ABOVE_BAUD
    }
}
