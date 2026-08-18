// SPDX-License-Identifier: Apache-2.0
//! Decoding one captured frame down to a TCP payload.
//!
//! # The trap that matters most
//!
//! **Trim the payload with the IP header's length field, never with the length
//! of the frame.**
//!
//! An Ethernet frame has a minimum size, so a bare acknowledgement — a TCP
//! segment carrying no data at all — arrives padded to sixty bytes. A decoder
//! that took "everything after the TCP header" as payload hands six bytes of
//! padding to whatever is above it. Fed to a Modbus TCP framer those six bytes
//! read as transaction 0, protocol 0, length 0: a phantom frame, on every
//! acknowledgement, in a capture where nothing is wrong.
//!
//! It is not a rare case. Measured on one real Modbus capture, **58 of 118
//! frames** had a frame length that disagreed with the IP total length. So the
//! payload here is always bounded by IPv4's Total Length or IPv6's Payload
//! Length, and the frame's own length is used only as the outer bound that
//! stops a lying header reading past the buffer.
//!
//! # Everything else that is easy to get wrong
//!
//! * **The EtherType is a loop, not an `if`.** VLAN tags stack — a C-Tag
//!   inside an S-Tag is ordinary in carrier networks — so the type field is
//!   followed until it is not a tag. The loop is bounded, because a crafted
//!   frame can otherwise be all tags.
//! * **Header lengths are variable.** IPv4's IHL and TCP's data offset both
//!   count 32-bit words with a minimum of five. Twenty bytes is the common
//!   case and not the rule; a frame with IP options decodes wrongly for anyone
//!   who hardcodes it.
//! * **`LINKTYPE_NULL`'s protocol field is in the capturing host's byte
//!   order**, which the file's own magic does not tell you. Both orders are
//!   tried and the one that names a protocol wins. It looks like a bug and is
//!   the documented behaviour.

use core::fmt;

use crate::pcap::LinkType;

/// How deep a stack of VLAN tags may go before salman stops following it.
///
/// Two is ordinary (QinQ). A frame with a hundred is not a frame, it is
/// someone seeing how long the loop runs for.
const MAX_VLAN_TAGS: usize = 4;

/// EtherType for IPv4.
const ETHERTYPE_IPV4: u16 = 0x0800;
/// EtherType for IPv6.
const ETHERTYPE_IPV6: u16 = 0x86DD;
/// EtherType for an 802.1Q customer VLAN tag.
const ETHERTYPE_C_TAG: u16 = 0x8100;
/// EtherType for an 802.1ad service VLAN tag.
const ETHERTYPE_S_TAG: u16 = 0x88A8;
/// A second service-tag value seen in the wild.
const ETHERTYPE_S_TAG_ALT: u16 = 0x9100;

/// IP protocol number for TCP.
const PROTOCOL_TCP: u8 = 6;
/// IP protocol number for UDP.
const PROTOCOL_UDP: u8 = 17;

/// One end of a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Address {
    /// An IPv4 address.
    V4([u8; 4]),
    /// An IPv6 address.
    V6([u8; 16]),
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V4(octets) => {
                for (index, octet) in octets.iter().enumerate() {
                    if index > 0 {
                        f.write_str(".")?;
                    }
                    write!(f, "{octet}")?;
                }
                Ok(())
            }
            Self::V6(bytes) => {
                // Plain eight groups. Not the shortened form: this is for a
                // diagnostic, and an address a reader can compare with the one
                // in their configuration beats a prettier one they cannot.
                for group in 0..8 {
                    if group > 0 {
                        f.write_str(":")?;
                    }
                    let high = bytes.get(group * 2).copied().unwrap_or(0);
                    let low = bytes.get(group * 2 + 1).copied().unwrap_or(0);
                    write!(f, "{:x}", u16::from(high) << 8 | u16::from(low))?;
                }
                Ok(())
            }
        }
    }
}

/// One end of a TCP conversation: an address and a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Endpoint {
    /// The address.
    pub address: Address,
    /// The port.
    pub port: u16,
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.address {
            Address::V4(_) => write!(f, "{}:{}", self.address, self.port),
            // A bracketed form, because an IPv6 address contains colons and
            // `::1:502` is ambiguous to a reader as well as to a parser.
            Address::V6(_) => write!(f, "[{}]:{}", self.address, self.port),
        }
    }
}

/// One TCP segment, decoded out of a captured frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment<'a> {
    /// Where it came from.
    pub source: Endpoint,
    /// Where it was going.
    pub destination: Endpoint,
    /// The sequence number of its first payload byte.
    pub sequence: u32,
    /// The acknowledgement number, meaningful when [`Segment::ack`] is set.
    pub acknowledgement: u32,
    /// The payload, bounded by the IP header's length field.
    pub payload: &'a [u8],
    /// The SYN flag: this segment opens a connection.
    pub syn: bool,
    /// The ACK flag.
    pub ack: bool,
    /// The FIN flag: this end has finished sending.
    pub fin: bool,
    /// The RST flag: the connection was aborted.
    pub rst: bool,
    /// Whether the payload was cut short by the capture's snapshot length.
    ///
    /// A short payload is not a malformed one, and treating it as malformed is
    /// how a decoder blames a device for a decision the capturing tool made.
    pub truncated: bool,
}

impl Segment<'_> {
    /// The two ends, in a fixed order, so both directions of one conversation
    /// share a key.
    #[must_use]
    pub fn connection(&self) -> (Endpoint, Endpoint) {
        if self.source <= self.destination {
            (self.source, self.destination)
        } else {
            (self.destination, self.source)
        }
    }
}

/// What a frame turned out to hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decoded<'a> {
    /// A TCP segment.
    Tcp(Segment<'a>),
    /// Something salman deliberately does not decode, named rather than
    /// discarded.
    ///
    /// A capture is full of frames that are not the protocol being looked for,
    /// and a decoder that returned an error for each would drown the real
    /// findings. This is not an error; it is an answer.
    NotDecoded {
        /// What it was, as far as salman got.
        what: NotDecoded,
    },
}

/// Why a frame was not decoded further, when that is not a fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotDecoded {
    /// The link type is one salman does not decode.
    LinkType(LinkType),
    /// The EtherType is not IPv4 or IPv6 — ARP, for instance.
    EtherType(u16),
    /// The IP protocol is not TCP.
    Protocol(u8),
    /// The frame was cut so short by the snapshot length that its headers are
    /// incomplete. Nothing is wrong with the frame; the capture kept too
    /// little of it.
    TruncatedBeforeHeaders,
    /// The packet is a fragment, and salman does not reassemble fragments.
    ///
    /// Both halves of this matter and both are wrong to decode.
    ///
    /// A fragment with a non-zero offset carries **no transport header at
    /// all** — it is a continuation of one, and the bytes where a TCP header
    /// would be are application data. Decoding it produces a segment with
    /// invented ports, an invented sequence number and a payload cut from the
    /// middle of something: entirely plausible and entirely false.
    ///
    /// A first fragment does carry a TCP header, and carries only part of the
    /// payload. Handing that to a stream reassembler puts a hole in the
    /// middle of the byte stream that the sequence numbers do not account
    /// for.
    ///
    /// Reassembling IP fragments is a real piece of work with an overlap
    /// policy of its own — it is what evasion techniques target — and salman
    /// does not do it. Saying so beats half-doing it.
    Fragmented {
        /// Where in the original packet this fragment starts, in bytes.
        offset: u32,
        /// Whether more fragments follow.
        more_fragments: bool,
    },
}

impl fmt::Display for NotDecoded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LinkType(link) => write!(f, "salman does not decode {link}"),
            Self::EtherType(ty) => write!(f, "EtherType 0x{ty:04X} is not IPv4 or IPv6"),
            Self::Protocol(PROTOCOL_UDP) => f.write_str("UDP, which salman does not decode"),
            Self::Protocol(protocol) => write!(f, "IP protocol {protocol} is not TCP"),
            Self::TruncatedBeforeHeaders => {
                f.write_str("the capture kept too few bytes to reach the headers")
            }
            Self::Fragmented {
                offset,
                more_fragments,
            } => {
                if *offset == 0 {
                    f.write_str(
                        "this is the first of several IP fragments and carries only part of \
                         its payload; salman does not reassemble IP fragments",
                    )
                } else {
                    write!(
                        f,
                        "this is an IP fragment starting {offset} bytes into its packet, so \
                         it carries no transport header at all{}; salman does not reassemble \
                         IP fragments",
                        if *more_fragments {
                            " and more follow"
                        } else {
                            ""
                        }
                    )
                }
            }
        }
    }
}

/// Why a frame could not be decoded, when something is wrong with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// A header field says the header is shorter than its own minimum.
    HeaderTooShort {
        /// Which header.
        layer: &'static str,
        /// What the field said, in bytes.
        declared: usize,
        /// The smallest that header may be.
        minimum: usize,
    },
    /// A length field claims more than the frame holds.
    LengthPastEndOfFrame {
        /// Which header.
        layer: &'static str,
        /// What the field claims, in bytes.
        declared: usize,
        /// What is there.
        available: usize,
    },
    /// The VLAN tags are stacked deeper than salman follows.
    TooManyVlanTags {
        /// How deep salman looks.
        limit: usize,
    },
    /// The IP version nibble is neither 4 nor 6.
    NotAnIpVersion {
        /// The nibble found.
        version: u8,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderTooShort {
                layer,
                declared,
                minimum,
            } => write!(
                f,
                "the {layer} header says it is {declared} bytes and the smallest one is {minimum}"
            ),
            Self::LengthPastEndOfFrame {
                layer,
                declared,
                available,
            } => write!(
                f,
                "the {layer} header claims {declared} bytes and the frame holds {available}"
            ),
            Self::TooManyVlanTags { limit } => write!(
                f,
                "the VLAN tags are stacked more than {limit} deep, which is not a frame \
                 anyone sent"
            ),
            Self::NotAnIpVersion { version } => {
                write!(f, "the IP version nibble is {version} and IP is 4 or 6")
            }
        }
    }
}

impl core::error::Error for FrameError {}

/// Decodes one captured frame.
///
/// `truncated` says the capture kept fewer bytes than were on the wire, which
/// changes how a short payload is reported: cut short by the capturing tool,
/// rather than malformed.
///
/// # Errors
///
/// Returns [`FrameError`] only when a header contradicts itself or the frame.
/// A frame that is simply not TCP over IP comes back as
/// [`Decoded::NotDecoded`], which is an answer rather than a failure.
pub fn decode(link: LinkType, frame: &[u8], truncated: bool) -> Result<Decoded<'_>, FrameError> {
    let (ether_type, rest) = match link {
        LinkType::ETHERNET => {
            // Six octets of destination, six of source, then the type.
            let Some(rest) = frame.get(14..) else {
                return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
            };
            let ether_type = be16(frame, 12).ok_or(FrameError::LengthPastEndOfFrame {
                layer: "Ethernet",
                declared: 14,
                available: frame.len(),
            })?;
            follow_vlan_tags(ether_type, rest)?
        }
        LinkType::LINUX_SLL => {
            let Some(rest) = frame.get(16..) else {
                return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
            };
            // The last two octets of the 16-octet header carry an EtherType.
            let Some(ether_type) = be16(frame, 14) else {
                return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
            };
            follow_vlan_tags(ether_type, rest)?
        }
        LinkType::LINUX_SLL2 => {
            let Some(rest) = frame.get(20..) else {
                return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
            };
            // SLL2 puts the protocol first.
            let Some(ether_type) = be16(frame, 0) else {
                return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
            };
            follow_vlan_tags(ether_type, rest)?
        }
        LinkType::RAW => {
            // No link header at all: the IP version nibble is the only thing
            // that says which protocol this is.
            (version_ether_type(frame)?, frame)
        }
        LinkType::IPV4 => (ETHERTYPE_IPV4, frame),
        LinkType::IPV6 => (ETHERTYPE_IPV6, frame),
        LinkType::NULL => {
            let Some(rest) = frame.get(4..) else {
                return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
            };
            let Some(raw) = frame.get(..4).and_then(|b| b.try_into().ok()) else {
                return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
            };
            // The protocol field is in the **capturing host's** byte order,
            // and the file's magic says nothing about it. Both orders are
            // tried and whichever names a protocol wins. This looks like a
            // bug; it is the documented behaviour of the format.
            let ether_type = null_protocol(u32::from_le_bytes(raw))
                .or_else(|| null_protocol(u32::from_be_bytes(raw)));
            match ether_type {
                Some(ether_type) => (ether_type, rest),
                None => return Ok(not_decoded(NotDecoded::EtherType(0))),
            }
        }
        other => return Ok(not_decoded(NotDecoded::LinkType(other))),
    };

    match ether_type {
        ETHERTYPE_IPV4 => decode_ipv4(rest, truncated),
        ETHERTYPE_IPV6 => decode_ipv6(rest, truncated),
        other => Ok(not_decoded(NotDecoded::EtherType(other))),
    }
}

/// Wraps a reason as a decoded answer.
const fn not_decoded<'a>(what: NotDecoded) -> Decoded<'a> {
    Decoded::NotDecoded { what }
}

/// The BSD loopback protocol numbers salman knows.
const fn null_protocol(value: u32) -> Option<u16> {
    match value {
        2 => Some(ETHERTYPE_IPV4),
        // Three numbers have all meant IPv6 on different systems.
        24 | 28 | 30 => Some(ETHERTYPE_IPV6),
        _ => None,
    }
}

/// The EtherType a bare IP packet implies, from its version nibble.
fn version_ether_type(frame: &[u8]) -> Result<u16, FrameError> {
    let first = frame.first().copied().unwrap_or(0);
    match first >> 4 {
        4 => Ok(ETHERTYPE_IPV4),
        6 => Ok(ETHERTYPE_IPV6),
        version => Err(FrameError::NotAnIpVersion { version }),
    }
}

/// Steps past any VLAN tags and returns the real type and what follows.
fn follow_vlan_tags(mut ether_type: u16, mut rest: &[u8]) -> Result<(u16, &[u8]), FrameError> {
    let mut depth = 0;
    while matches!(
        ether_type,
        ETHERTYPE_C_TAG | ETHERTYPE_S_TAG | ETHERTYPE_S_TAG_ALT
    ) {
        depth += 1;
        if depth > MAX_VLAN_TAGS {
            return Err(FrameError::TooManyVlanTags {
                limit: MAX_VLAN_TAGS,
            });
        }
        // A tag is two octets of control information and two of the next type.
        let Some(next) = be16(rest, 2) else {
            return Ok((ether_type, &[]));
        };
        let Some(after) = rest.get(4..) else {
            return Ok((ether_type, &[]));
        };
        ether_type = next;
        rest = after;
    }
    Ok((ether_type, rest))
}

/// Decodes IPv4 and whatever it carries.
fn decode_ipv4(packet: &[u8], truncated: bool) -> Result<Decoded<'_>, FrameError> {
    let Some(first) = packet.first().copied() else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    // The header length counts 32-bit words and its minimum is five. Twenty
    // bytes is the common case, not the rule: a packet with IP options is
    // longer, and hardcoding twenty reads the options as a TCP header.
    let header_len = usize::from(first & 0x0F) * 4;
    if header_len < 20 {
        return Err(FrameError::HeaderTooShort {
            layer: "IPv4",
            declared: header_len,
            minimum: 20,
        });
    }
    let Some(total_length) = be16(packet, 2).map(usize::from) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    let Some(protocol) = packet.get(9).copied() else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };

    // Bytes 6 and 7 are three flag bits then a thirteen-bit fragment offset,
    // counted in eight-byte units. A packet that is fragmented at all is one
    // salman will not decode; see `NotDecoded::Fragmented` for why both halves
    // of that are wrong to decode rather than only the obvious half.
    let Some(fragment_word) = be16(packet, 6) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    let more_fragments = fragment_word & 0x2000 != 0;
    let fragment_offset = u32::from(fragment_word & 0x1FFF) * 8;
    if more_fragments || fragment_offset != 0 {
        return Ok(not_decoded(NotDecoded::Fragmented {
            offset: fragment_offset,
            more_fragments,
        }));
    }
    let source = packet
        .get(12..16)
        .and_then(|b| b.try_into().ok())
        .map(Address::V4);
    let destination = packet
        .get(16..20)
        .and_then(|b| b.try_into().ok())
        .map(Address::V4);
    let (Some(source), Some(destination)) = (source, destination) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };

    if total_length < header_len {
        return Err(FrameError::HeaderTooShort {
            layer: "IPv4",
            declared: total_length,
            minimum: header_len,
        });
    }

    // The total length is what bounds the payload — never the frame's own
    // length, which includes Ethernet padding on any short frame. The frame
    // length is still the outer bound, because the field can claim more than
    // arrived when the capture was snapped short or the header is lying.
    let declared_end = total_length;
    let available_end = packet.len();
    let short_by_capture = declared_end > available_end;
    let end = declared_end.min(available_end);
    let Some(body) = packet.get(header_len..end) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };

    match protocol {
        PROTOCOL_TCP => decode_tcp(body, source, destination, truncated || short_by_capture),
        other => Ok(not_decoded(NotDecoded::Protocol(other))),
    }
}

/// Decodes IPv6 and whatever it carries.
///
/// Extension header chains are **not** followed: a packet carrying one is
/// reported as not decoded rather than guessed at, because walking a chain
/// wrongly produces a payload that looks like data and is not.
fn decode_ipv6(packet: &[u8], truncated: bool) -> Result<Decoded<'_>, FrameError> {
    const HEADER: usize = 40;
    let Some(payload_length) = be16(packet, 4).map(usize::from) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    let Some(next_header) = packet.get(6).copied() else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    let source = packet
        .get(8..24)
        .and_then(|b| b.try_into().ok())
        .map(Address::V6);
    let destination = packet
        .get(24..40)
        .and_then(|b| b.try_into().ok())
        .map(Address::V6);
    let (Some(source), Some(destination)) = (source, destination) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };

    // As with IPv4: the header's own length field bounds the payload, and what
    // actually arrived bounds that.
    let declared_end = HEADER + payload_length;
    let short_by_capture = declared_end > packet.len();
    let end = declared_end.min(packet.len());
    let Some(body) = packet.get(HEADER..end) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };

    match next_header {
        PROTOCOL_TCP => decode_tcp(body, source, destination, truncated || short_by_capture),
        other => Ok(not_decoded(NotDecoded::Protocol(other))),
    }
}

/// Decodes a TCP segment.
fn decode_tcp(
    segment: &[u8],
    source: Address,
    destination: Address,
    truncated: bool,
) -> Result<Decoded<'_>, FrameError> {
    let (Some(source_port), Some(destination_port)) = (be16(segment, 0), be16(segment, 2)) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    let (Some(sequence), Some(acknowledgement)) = (be32(segment, 4), be32(segment, 8)) else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    let Some(offset_byte) = segment.get(12).copied() else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    // Like IPv4's IHL: 32-bit words, minimum five. A segment carrying options —
    // and a SYN almost always does — has a longer header than twenty bytes.
    let header_len = usize::from(offset_byte >> 4) * 4;
    if header_len < 20 {
        return Err(FrameError::HeaderTooShort {
            layer: "TCP",
            declared: header_len,
            minimum: 20,
        });
    }
    let Some(flags) = segment.get(13).copied() else {
        return Ok(not_decoded(NotDecoded::TruncatedBeforeHeaders));
    };
    let short_by_capture = header_len > segment.len();
    let payload = segment.get(header_len..).unwrap_or(&[]);

    Ok(Decoded::Tcp(Segment {
        source: Endpoint {
            address: source,
            port: source_port,
        },
        destination: Endpoint {
            address: destination,
            port: destination_port,
        },
        sequence,
        acknowledgement,
        payload,
        fin: flags & 0x01 != 0,
        syn: flags & 0x02 != 0,
        rst: flags & 0x04 != 0,
        ack: flags & 0x10 != 0,
        truncated: truncated || short_by_capture,
    }))
}

/// A big-endian 16-bit field at `offset`.
fn be16(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset + 2)
        .and_then(|b| b.try_into().ok())
        .map(u16::from_be_bytes)
}

/// A big-endian 32-bit field at `offset`.
fn be32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_be_bytes)
}
