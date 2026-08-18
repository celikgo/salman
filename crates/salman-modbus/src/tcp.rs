// SPDX-License-Identifier: Apache-2.0
//! Modbus TCP: the MBAP header, and framing a byte stream into ADUs.
//!
//! # Why a framer exists at all
//!
//! MODBUS Messaging on TCP/IP Implementation Guide V1.0b §3.1.2 says the
//! length field is there so that a receiver can find message boundaries even
//! when a message has been split across packets. MG's Implementation Rule 6
//! — that a TCP frame carries one ADU — binds the **sender**. It grants the
//! receiver nothing, because TCP is a byte stream and a receiver never sees
//! frames at all.
//!
//! So a reader that calls `read()` once and decodes what it got is wrong in
//! four ways that all occur in practice: a header split across two segments,
//! a body split across two segments, two ADUs delivered in one read, and one
//! and a half ADUs delivered in one read. [`Framer`] handles all four, and
//! each has a test.
//!
//! # There is no resynchronisation
//!
//! A Modbus TCP stream has no sync word and no checksum. If the length field
//! is wrong, there is nothing in the stream that says where the next frame
//! begins — any byte could be a plausible transaction identifier. A framer
//! that "searched forward for the next valid-looking header" would be
//! guessing, and would eventually deliver a frame assembled from the middle of
//! two others.
//!
//! salman therefore treats a bad length as **fatal to the connection**. See
//! [`FrameError::is_fatal`].

use crate::limits::{MAX_PDU, MAX_TCP_ADU, MBAP_HEADER};
use crate::pdu::Pdu;

/// The smallest MBAP length field: one unit identifier and one function code.
pub const MIN_MBAP_LENGTH: u16 = 2;

/// The largest MBAP length field: one unit identifier and a full PDU.
pub const MAX_MBAP_LENGTH: u16 = 1 + MAX_PDU as u16;

/// The MBAP header that precedes every Modbus TCP protocol data unit.
///
/// MG §3.1.3. All three 16-bit fields are big-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbapHeader {
    /// Chosen by the client, echoed by the server, and the only thing that
    /// pairs a response with its request. MG §4.4.2.5.
    pub transaction: u16,
    /// Zero for Modbus. MG §3.1.3. Anything else is not this protocol.
    pub protocol: u16,
    /// The number of bytes that follow this field: the unit identifier plus
    /// the PDU.
    pub length: u16,
    /// Meaningless when the server is the end device, and the serial slave
    /// address when it is a gateway. MG §4.4.1.2 specifies `0xFF` for the
    /// former and notes that `0x00` is also accepted. A server echoes what it
    /// received.
    pub unit: u8,
}

impl MbapHeader {
    /// A header for a PDU of `pdu_len` bytes.
    #[must_use]
    pub const fn for_pdu(transaction: u16, unit: u8, pdu_len: usize) -> Self {
        Self {
            transaction,
            protocol: 0,
            length: (pdu_len as u16) + 1,
            unit,
        }
    }

    /// The seven bytes, as they go on the wire.
    #[must_use]
    pub fn to_bytes(self) -> [u8; MBAP_HEADER] {
        let transaction = self.transaction.to_be_bytes();
        let protocol = self.protocol.to_be_bytes();
        let length = self.length.to_be_bytes();
        [
            transaction[0],
            transaction[1],
            protocol[0],
            protocol[1],
            length[0],
            length[1],
            self.unit,
        ]
    }

    /// Reads the seven bytes without judging them.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; MBAP_HEADER]) -> Self {
        Self {
            transaction: u16::from_be_bytes([bytes[0], bytes[1]]),
            protocol: u16::from_be_bytes([bytes[2], bytes[3]]),
            length: u16::from_be_bytes([bytes[4], bytes[5]]),
            unit: bytes[6],
        }
    }

    /// How many PDU bytes the length field claims follow.
    ///
    /// `None` when the length field is outside what a Modbus frame may carry,
    /// in which case the stream cannot be framed at all.
    #[must_use]
    pub const fn claimed_pdu_len(self) -> Option<usize> {
        if self.length < MIN_MBAP_LENGTH || self.length > MAX_MBAP_LENGTH {
            return None;
        }
        Some(self.length as usize - 1)
    }
}

/// One Modbus TCP application data unit: a header and the PDU it announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TcpAdu {
    /// The header as it arrived.
    pub header: MbapHeader,
    /// The protocol data unit.
    pub pdu: Pdu,
}

impl TcpAdu {
    /// Wraps a PDU for sending.
    #[must_use]
    pub fn new(transaction: u16, unit: u8, pdu: Pdu) -> Self {
        Self {
            header: MbapHeader::for_pdu(transaction, unit, pdu.len()),
            pdu,
        }
    }

    /// The whole ADU, header first.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MBAP_HEADER + self.pdu.len());
        bytes.extend_from_slice(&self.header.to_bytes());
        bytes.extend_from_slice(self.pdu.as_bytes());
        bytes
    }
}

/// Why a byte stream could not be framed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The protocol identifier was not zero, so these bytes are not Modbus.
    ///
    /// MG §4.4.2.5 has the server discard such a frame without answering. It
    /// is fatal here because there is no way to know where the frame that was
    /// not Modbus ends.
    ProtocolNotModbus {
        /// The identifier found.
        found: u16,
    },
    /// The length field cannot describe any Modbus frame.
    ///
    /// A frame of the length claimed could not exist, so the byte after it is
    /// not the start of anything. Continuing would be guessing.
    LengthOutOfRange {
        /// The length found.
        found: u16,
        /// The smallest a Modbus frame may claim.
        min: u16,
        /// The largest a Modbus frame may claim.
        max: u16,
    },
}

impl FrameError {
    /// Whether the connection has to be closed.
    ///
    /// Both variants are fatal, and the method exists so that a caller reads
    /// the reason rather than assuming. A Modbus TCP stream carries no sync
    /// word and no checksum: once framing is lost there is nothing in the
    /// stream that can recover it, and a framer that searched forward for a
    /// plausible header would eventually assemble a frame out of the middle of
    /// two others and report it as real.
    #[must_use]
    pub const fn is_fatal(self) -> bool {
        match self {
            Self::ProtocolNotModbus { .. } | Self::LengthOutOfRange { .. } => true,
        }
    }
}

impl core::fmt::Display for FrameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ProtocolNotModbus { found } => write!(
                f,
                "the protocol identifier is 0x{found:04X} and Modbus is 0x0000"
            ),
            Self::LengthOutOfRange { found, min, max } => write!(
                f,
                "the length field claims {found} bytes follow, and a Modbus frame \
                 claims between {min} and {max}"
            ),
        }
    }
}

impl core::error::Error for FrameError {}

/// Assembles Modbus TCP application data units from a byte stream.
///
/// One framer per **direction** of one connection. Requests and responses are
/// independent byte streams and cannot share a buffer.
///
/// The buffer is a fixed array the size of the largest ADU, so a peer that
/// claims a huge length cannot make salman reserve memory — the claim is
/// checked against [`MAX_MBAP_LENGTH`] before a byte is copied.
#[derive(Debug, Clone)]
pub struct Framer {
    buffer: [u8; MAX_TCP_ADU],
    len: usize,
    /// The error that lost the stream, kept so it can be given again.
    ///
    /// Reporting a fault once and then going quiet is worse than not
    /// reporting it: a caller that reads until it has a frame would read for
    /// ever against a peer that keeps sending, with the read timeout never
    /// firing because bytes keep arriving. So a poisoned framer answers with
    /// the same error every time it is asked.
    poisoned: Option<FrameError>,
}

impl Default for Framer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framer {
    /// The bytes needed before the length field can be read.
    const LENGTH_KNOWN_AT: usize = 6;

    /// A framer with an empty buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: [0; MAX_TCP_ADU],
            len: 0,
            poisoned: None,
        }
    }

    /// How many bytes are held from an incomplete frame.
    #[must_use]
    pub const fn buffered(&self) -> usize {
        self.len
    }

    /// Whether framing has been lost and the connection must be closed.
    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned.is_some()
    }

    /// Takes bytes from the stream until one ADU is complete.
    ///
    /// Returns how many bytes of `input` were consumed, and the frame if one
    /// finished. Call it in a loop, advancing `input` by the count returned,
    /// until it consumes nothing:
    ///
    /// ```
    /// # use salman_modbus::tcp::Framer;
    /// # let mut framer = Framer::new();
    /// # let received: &[u8] = &[];
    /// let mut rest = received;
    /// loop {
    ///     let (used, frame) = framer.advance(rest);
    ///     rest = &rest[used..];
    ///     match frame {
    ///         Ok(Some(adu)) => { let _ = adu; }
    ///         Ok(None) => break,
    ///         Err(error) => { assert!(error.is_fatal()); break }
    ///     }
    /// }
    /// ```
    ///
    /// Returning one frame per call rather than a list is what keeps the
    /// buffer at one ADU: bytes are taken from `input` only as far as the
    /// frame being assembled needs them, and the rest stays where it was.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError`] when the stream cannot be framed. Every variant
    /// is fatal to the connection; see [`FrameError::is_fatal`].
    pub fn advance(&mut self, input: &[u8]) -> (usize, Result<Option<TcpAdu>, FrameError>) {
        // Once the stream is lost it stays lost, and salman keeps saying so
        // rather than falling silent. A framer that answered "need more bytes"
        // for ever would turn a caller's read loop into a spin against any
        // peer that keeps sending — no error, no timeout, no progress.
        if let Some(error) = self.poisoned {
            return (0, Err(error));
        }
        let mut consumed = 0;

        // Step 1: enough bytes to read the length field.
        consumed += self.take(input, Self::LENGTH_KNOWN_AT);
        if self.len < Self::LENGTH_KNOWN_AT {
            return (consumed, Ok(None));
        }

        // Step 2 and 3: the header's two claims, checked before anything is
        // sized by them.
        let protocol = u16::from_be_bytes([self.byte(2), self.byte(3)]);
        if protocol != 0 {
            let error = FrameError::ProtocolNotModbus { found: protocol };
            self.poisoned = Some(error);
            return (consumed, Err(error));
        }
        let length = u16::from_be_bytes([self.byte(4), self.byte(5)]);
        if !(MIN_MBAP_LENGTH..=MAX_MBAP_LENGTH).contains(&length) {
            let error = FrameError::LengthOutOfRange {
                found: length,
                min: MIN_MBAP_LENGTH,
                max: MAX_MBAP_LENGTH,
            };
            self.poisoned = Some(error);
            return (consumed, Err(error));
        }

        // Step 4: the whole frame is six bytes plus what the length claims.
        let total = Self::LENGTH_KNOWN_AT + length as usize;
        consumed += self.take(input.get(consumed..).unwrap_or(&[]), total);
        if self.len < total {
            return (consumed, Ok(None));
        }

        // Step 5: deliver it and start the next one. Consuming exactly this
        // frame is what makes two ADUs in one read work: the second is still
        // in `input`, and the caller's loop brings it back.
        let header = MbapHeader::from_bytes([
            self.byte(0),
            self.byte(1),
            self.byte(2),
            self.byte(3),
            self.byte(4),
            self.byte(5),
            self.byte(6),
        ]);
        let pdu = self
            .buffer
            .get(MBAP_HEADER..total)
            .and_then(Pdu::from_bytes);
        self.len = 0;
        let Some(pdu) = pdu else {
            // `length >= MIN_MBAP_LENGTH` guarantees at least one PDU byte, so
            // this cannot be reached. Reporting it as a length fault beats a
            // panic in a decoder that faces the network.
            let error = FrameError::LengthOutOfRange {
                found: length,
                min: MIN_MBAP_LENGTH,
                max: MAX_MBAP_LENGTH,
            };
            self.poisoned = Some(error);
            return (consumed, Err(error));
        };
        (consumed, Ok(Some(TcpAdu { header, pdu })))
    }

    /// Copies from `input` until the buffer holds `target` bytes, and says how
    /// many it took.
    fn take(&mut self, input: &[u8], target: usize) -> usize {
        if self.len >= target {
            return 0;
        }
        let wanted = target - self.len;
        let taken = wanted.min(input.len());
        for offset in 0..taken {
            let Some(byte) = input.get(offset) else { break };
            if let Some(slot) = self.buffer.get_mut(self.len) {
                *slot = *byte;
                self.len += 1;
            }
        }
        taken
    }

    /// A buffered byte, or zero past the end. Only called where the length has
    /// already been checked.
    fn byte(&self, index: usize) -> u8 {
        self.buffer.get(index).copied().unwrap_or(0)
    }
}
