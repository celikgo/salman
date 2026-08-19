// SPDX-License-Identifier: Apache-2.0
//! The classic pcap container.
//!
//! # What the four magic numbers mean
//!
//! The magic is both a format marker and a byte-order marker, and the two
//! timestamp scales double the count:
//!
//! | On disk | Byte order | Timestamps |
//! |---|---|---|
//! | `A1 B2 C3 D4` | big-endian | microseconds |
//! | `A1 B2 3C 4D` | big-endian | nanoseconds |
//! | `D4 C3 B2 A1` | little-endian | microseconds |
//! | `4D 3C B2 A1` | little-endian | nanoseconds |
//!
//! libpcap has also emitted several other magics over the years. salman
//! **refuses** them by name rather than treating them as unknown, because one
//! of them — Kuznetzov's `A1 B2 CD 34` — has a *longer per-record header*, so
//! a reader that guessed would misparse every record while producing entirely
//! plausible-looking output. That is the worst failure available here, and it
//! is the reason this list is explicit.

use core::fmt;

/// The bytes of a pcap file header.
const FILE_HEADER: usize = 24;

/// The bytes of a per-record header.
const RECORD_HEADER: usize = 16;

/// The magic as a number, for a file with microsecond timestamps.
///
/// A file whose first four bytes are `A1 B2 C3 D4` is big-endian; one whose
/// first four are `D4 C3 B2 A1` is little-endian. The magic is the byte-order
/// marker as well as the format marker, which is why it is compared both ways
/// below rather than read with a fixed order.
const MAGIC_MICROS: u32 = 0xA1B2_C3D4;

/// The same, for a file with nanosecond timestamps.
const MAGIC_NANOS: u32 = 0xA1B2_3C4D;

/// A capture salman refuses, and the reason it gives.
struct Refused {
    magic: [u8; 4],
    name: &'static str,
    why: &'static str,
}

/// The libpcap variants salman will not read.
///
/// Each is refused with its name, because "unrecognised file" tells a reader
/// nothing and one of these would silently misparse if it were guessed at.
const REFUSED: &[Refused] = &[
    Refused {
        magic: [0xA1, 0xB2, 0xCD, 0x34],
        name: "Kuznetzov's modified pcap",
        why: "its per-record header is longer than the standard one, so reading it as \
              standard pcap would misparse every record and produce plausible nonsense",
    },
    Refused {
        magic: [0x34, 0xCD, 0xB2, 0xA1],
        name: "Kuznetzov's modified pcap, byte-swapped",
        why: "its per-record header is longer than the standard one, so reading it as \
              standard pcap would misparse every record and produce plausible nonsense",
    },
    Refused {
        magic: [0xA1, 0xB2, 0x34, 0xCD],
        name: "the Alexey Kuznetzov variant",
        why: "salman has not established what its record header looks like, and guessing \
              would risk reading every record wrongly",
    },
    Refused {
        magic: [0xA1, 0x2B, 0x3C, 0x4D],
        name: "the Navtel variant",
        why: "salman has not established what its record header looks like, and guessing \
              would risk reading every record wrongly",
    },
    Refused {
        magic: [0xA1, 0xB2, 0xC3, 0xCB],
        name: "the Nokia variant",
        why: "salman has not established what its record header looks like, and guessing \
              would risk reading every record wrongly",
    },
];

/// Whether the numbers in a file are most or least significant byte first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// Least significant byte first.
    Little,
    /// Most significant byte first.
    Big,
}

impl ByteOrder {
    fn u16(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        }
    }

    fn u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }
}

/// What the second half of a timestamp counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampScale {
    /// Microseconds since the second.
    Microseconds,
    /// Nanoseconds since the second.
    Nanoseconds,
}

impl TimestampScale {
    /// Nanoseconds per unit of this scale.
    #[must_use]
    pub const fn nanos_per_unit(self) -> u64 {
        match self {
            Self::Microseconds => 1_000,
            Self::Nanoseconds => 1,
        }
    }
}

/// How precisely a capture records time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    /// What the fractional field counts.
    pub scale: TimestampScale,
}

/// The link layer a capture's frames start with.
///
/// The registry lives at <https://www.tcpdump.org/linktypes.html>. Only the
/// values salman decodes are named; anything else keeps its number, because a
/// capture salman cannot decode is still a capture whose records it can count
/// and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkType(pub u16);

impl LinkType {
    /// BSD loopback: a four-octet protocol field in the **capturing host's**
    /// byte order, which the file's magic does not tell you.
    pub const NULL: Self = Self(0);
    /// Ethernet II.
    pub const ETHERNET: Self = Self(1);
    /// A bare IP packet with no link header at all.
    pub const RAW: Self = Self(101);
    /// Linux "cooked" capture, 16 octets. What `tcpdump -i any` writes.
    pub const LINUX_SLL: Self = Self(113);
    /// Raw IPv4.
    pub const IPV4: Self = Self(228);
    /// Raw IPv6.
    pub const IPV6: Self = Self(229);
    /// Linux "cooked" capture version 2, 20 octets.
    pub const LINUX_SLL2: Self = Self(276);

    /// The name salman knows this link type by, if it knows one.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0 => "NULL",
            1 => "ETHERNET",
            101 => "RAW",
            113 => "LINUX_SLL",
            228 => "IPV4",
            229 => "IPV6",
            276 => "LINUX_SLL2",
            _ => return None,
        })
    }
}

impl fmt::Display for LinkType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{name} ({})", self.0),
            None => write!(f, "link type {}", self.0),
        }
    }
}

/// One captured frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record<'a> {
    /// Seconds since the Unix epoch, as the capturing host recorded them.
    pub seconds: u32,
    /// The fraction of that second, in the file's scale.
    pub fraction: u32,
    /// How long the frame was on the wire.
    pub original_length: u32,
    /// The bytes that were saved, which may be fewer.
    pub data: &'a [u8],
    /// Whether the frame was cut short by the capture's snapshot length.
    ///
    /// A truncated frame is not malformed, and reporting it as malformed is a
    /// common and confusing mistake: the sender sent a complete frame and the
    /// capturing tool declined to keep all of it.
    pub truncated: bool,
    /// Which record this is, counting from zero.
    pub index: u64,
}

impl Record<'_> {
    /// The timestamp in nanoseconds since the Unix epoch.
    #[must_use]
    pub const fn nanos(&self, scale: TimestampScale) -> u64 {
        self.seconds as u64 * 1_000_000_000 + self.fraction as u64 * scale.nanos_per_unit()
    }
}

/// Reads a classic pcap file.
///
/// Borrows the bytes rather than copying them: a capture is often large, and a
/// reader that copied every frame would double the memory for no gain.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    order: ByteOrder,
    scale: TimestampScale,
    version: (u16, u16),
    link: LinkType,
    snapshot_length: u32,
    rest: &'a [u8],
    index: u64,
}

impl<'a> Reader<'a> {
    /// Opens a capture.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] naming what is wrong. A file whose magic is a
    /// libpcap variant salman does not read is refused **by name**, because
    /// "unrecognised" would leave a reader with nowhere to go and because one
    /// of those variants would misparse rather than fail if it were guessed.
    pub fn new(bytes: &'a [u8]) -> Result<Self, CaptureError> {
        let header = bytes.get(..FILE_HEADER).ok_or(CaptureError::TooShort {
            needed: FILE_HEADER,
            found: bytes.len(),
        })?;
        let magic: [u8; 4] =
            header
                .get(..4)
                .and_then(|b| b.try_into().ok())
                .ok_or(CaptureError::TooShort {
                    needed: FILE_HEADER,
                    found: bytes.len(),
                })?;

        for refused in REFUSED {
            if magic == refused.magic {
                return Err(CaptureError::UnsupportedVariant {
                    magic,
                    name: refused.name,
                    why: refused.why,
                });
            }
        }

        let (order, scale) = match u32::from_be_bytes(magic) {
            MAGIC_MICROS => (ByteOrder::Big, TimestampScale::Microseconds),
            MAGIC_NANOS => (ByteOrder::Big, TimestampScale::Nanoseconds),
            _ => match u32::from_le_bytes(magic) {
                MAGIC_MICROS => (ByteOrder::Little, TimestampScale::Microseconds),
                MAGIC_NANOS => (ByteOrder::Little, TimestampScale::Nanoseconds),
                _ => return Err(CaptureError::NotAPcap { magic }),
            },
        };

        let version = (order.u16(take2(header, 4)?), order.u16(take2(header, 6)?));
        // Bytes 8..16 are two reserved fields. The draft says writers SHOULD
        // zero them and readers MUST ignore them, so salman ignores them: real
        // files predate the formalisation and validating them would refuse
        // captures that every other tool reads.
        let snapshot_length = order.u32(take4(header, 16)?);
        let link_word = order.u32(take4(header, 20)?);

        if snapshot_length == 0 {
            return Err(CaptureError::ZeroSnapshotLength);
        }

        Ok(Self {
            order,
            scale,
            version,
            // The low sixteen bits are the link type. The rest of the word
            // carries an FCS length and two flags that only mean anything when
            // the P bit is set, and real files predate their formalisation, so
            // a non-zero remainder is not an error here.
            link: LinkType((link_word & 0xFFFF) as u16),
            snapshot_length,
            rest: bytes.get(FILE_HEADER..).unwrap_or(&[]),
            index: 0,
        })
    }

    /// The major and minor version the file declares.
    ///
    /// Every file in circulation says 2.4, and salman does not refuse one that
    /// says otherwise: the record layout has not changed, and refusing on a
    /// version number would reject a file every other tool reads.
    #[must_use]
    pub const fn version(&self) -> (u16, u16) {
        self.version
    }

    /// The byte order the file's numbers are in.
    #[must_use]
    pub const fn byte_order(&self) -> ByteOrder {
        self.order
    }

    /// What the fractional part of each timestamp counts.
    #[must_use]
    pub const fn scale(&self) -> TimestampScale {
        self.scale
    }

    /// The link layer every frame in this file starts with.
    #[must_use]
    pub const fn link_type(&self) -> LinkType {
        self.link
    }

    /// The longest frame the capturing tool intended to keep.
    #[must_use]
    pub const fn snapshot_length(&self) -> u32 {
        self.snapshot_length
    }

    /// Reads the next record.
    ///
    /// Returns `Ok(None)` at the end of the file.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] if a record header is incomplete or a record
    /// claims more data than the file holds.
    pub fn next_record(&mut self) -> Result<Option<Record<'a>>, CaptureError> {
        if self.rest.is_empty() {
            return Ok(None);
        }
        let header = self
            .rest
            .get(..RECORD_HEADER)
            .ok_or(CaptureError::ShortRecordHeader {
                index: self.index,
                found: self.rest.len(),
            })?;
        let seconds = self.order.u32(take4(header, 0)?);
        let fraction = self.order.u32(take4(header, 4)?);
        let captured = self.order.u32(take4(header, 8)?);
        let original = self.order.u32(take4(header, 12)?);

        // Every read is bounded by the captured length and never by the
        // original length. The draft explicitly permits a record whose
        // original length is *smaller* than its captured length, and a reader
        // that sized anything from the original would either over-read or
        // silently drop data.
        let body = self
            .rest
            .get(RECORD_HEADER..)
            .and_then(|r| r.get(..captured as usize))
            .ok_or(CaptureError::RecordPastEndOfFile {
                index: self.index,
                captured,
                available: self.rest.len().saturating_sub(RECORD_HEADER),
            })?;

        self.rest = self
            .rest
            .get(RECORD_HEADER + captured as usize..)
            .unwrap_or(&[]);
        let index = self.index;
        self.index += 1;

        Ok(Some(Record {
            seconds,
            fraction,
            original_length: original,
            data: body,
            truncated: captured < original,
            index,
        }))
    }

    /// Every remaining record, and whatever stopped the reading.
    ///
    /// Both, rather than one or the other. A truncated capture is ordinary —
    /// a file still being written, a `tcpdump` that was killed — and throwing
    /// away every record because the last one is short would lose a whole
    /// capture over its final frame. The error says where it stopped, so a
    /// caller can report the gap rather than pretend the file ended tidily.
    ///
    /// An earlier version of this returned a `Result` and documented that it
    /// gave back the records read before the error, which it did not: the `?`
    /// discarded them.
    pub fn records(&mut self) -> (Vec<Record<'a>>, Option<CaptureError>) {
        let mut out = Vec::new();
        loop {
            match self.next_record() {
                Ok(Some(record)) => out.push(record),
                Ok(None) => return (out, None),
                Err(error) => return (out, Some(error)),
            }
        }
    }
}

/// Writes a classic pcap file.
///
/// salman writes little-endian with microsecond timestamps, which is what
/// `tcpdump -w` produces and what every tool reads.
#[derive(Debug, Clone)]
pub struct Writer {
    bytes: Vec<u8>,
    scale: TimestampScale,
}

impl Writer {
    /// Starts a capture of `link` frames.
    #[must_use]
    pub fn new(link: LinkType, snapshot_length: u32) -> Self {
        let mut bytes = Vec::with_capacity(FILE_HEADER);
        bytes.extend_from_slice(&MAGIC_MICROS.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&snapshot_length.max(1).to_le_bytes());
        bytes.extend_from_slice(&u32::from(link.0).to_le_bytes());
        Self {
            bytes,
            scale: TimestampScale::Microseconds,
        }
    }

    /// Appends a frame captured at `nanos` since the Unix epoch.
    pub fn write(&mut self, nanos: u64, original_length: u32, data: &[u8]) {
        let seconds = (nanos / 1_000_000_000) as u32;
        let fraction = ((nanos % 1_000_000_000) / self.scale.nanos_per_unit()) as u32;
        self.bytes.extend_from_slice(&seconds.to_le_bytes());
        self.bytes.extend_from_slice(&fraction.to_le_bytes());
        self.bytes
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.bytes
            .extend_from_slice(&original_length.max(data.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(data);
    }

    /// The file.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// Why a capture could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// The file is shorter than a pcap file header.
    TooShort {
        /// How many bytes a header needs.
        needed: usize,
        /// How many the file has.
        found: usize,
    },
    /// The magic is not one of the four pcap magics.
    NotAPcap {
        /// The first four bytes.
        magic: [u8; 4],
    },
    /// The magic names a libpcap variant salman will not read.
    UnsupportedVariant {
        /// The first four bytes.
        magic: [u8; 4],
        /// What the variant is called.
        name: &'static str,
        /// Why salman refuses rather than guessing.
        why: &'static str,
    },
    /// The header says no bytes of any frame were kept.
    ZeroSnapshotLength,
    /// A record header ran off the end of the file.
    ShortRecordHeader {
        /// Which record.
        index: u64,
        /// How many bytes were left.
        found: usize,
    },
    /// A record claims more data than the file holds.
    RecordPastEndOfFile {
        /// Which record.
        index: u64,
        /// How many bytes it claims.
        captured: u32,
        /// How many are left.
        available: usize,
    },
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { needed, found } => write!(
                f,
                "a pcap file header is {needed} bytes and this file is {found}"
            ),
            Self::NotAPcap { magic } => write!(
                f,
                "this does not begin with a pcap magic: it begins {magic:02X?}. \
                 The four are A1B2C3D4, A1B23C4D and those two byte-swapped"
            ),
            Self::UnsupportedVariant { magic, name, why } => write!(
                f,
                "this is {name} ({magic:02X?}), which salman does not read: {why}"
            ),
            Self::ZeroSnapshotLength => f.write_str(
                "the header says the snapshot length is zero, so no frame in this file \
                 could hold anything",
            ),
            Self::ShortRecordHeader { index, found } => write!(
                f,
                "record {index} has {found} bytes left and a record header is {RECORD_HEADER}"
            ),
            Self::RecordPastEndOfFile {
                index,
                captured,
                available,
            } => write!(
                f,
                "record {index} claims {captured} bytes and {available} are left in the file"
            ),
        }
    }
}

impl core::error::Error for CaptureError {}

/// Two bytes at `offset`, or a short-file error.
fn take2(bytes: &[u8], offset: usize) -> Result<[u8; 2], CaptureError> {
    bytes
        .get(offset..offset + 2)
        .and_then(|b| b.try_into().ok())
        .ok_or(CaptureError::TooShort {
            needed: offset + 2,
            found: bytes.len(),
        })
}

/// Four bytes at `offset`, or a short-file error.
fn take4(bytes: &[u8], offset: usize) -> Result<[u8; 4], CaptureError> {
    bytes
        .get(offset..offset + 4)
        .and_then(|b| b.try_into().ok())
        .ok_or(CaptureError::TooShort {
            needed: offset + 4,
            found: bytes.len(),
        })
}
