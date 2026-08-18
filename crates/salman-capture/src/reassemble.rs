// SPDX-License-Identifier: Apache-2.0
//! Turning captured TCP segments back into the byte streams they carried.
//!
//! A protocol decoder above this layer must be fed a **contiguous byte
//! stream**, not packets. Feeding it packets is the mistake that makes a
//! capture-based tool disagree with the device it is watching: a Modbus frame
//! split across two segments looks malformed, two frames in one segment lose
//! the second, and a retransmission looks like a repeated request.
//!
//! # Five things this has to get right
//!
//! **Each direction is its own stream.** A request stream and a response
//! stream share a connection and nothing else. They have independent sequence
//! numbers, and mixing them produces bytes that never existed.
//!
//! **Sequence numbers wrap.** They are 32 bits and a busy connection passes
//! `0xFFFFFFFF` in minutes at gigabit speeds. Comparing them with `<` is wrong
//! at exactly the moment the comparison matters, so every comparison here is
//! modular: `a` is before `b` when `b - a`, computed with wrapping arithmetic,
//! is less than 2³¹.
//!
//! **A capture rarely starts at the SYN.** Most captures begin in the middle
//! of a conversation that was already running. The first sequence number seen
//! is adopted as the base, and the stream is marked
//! [`mid_stream`](Stream::mid_stream) so that nothing downstream mistakes
//! "salman did not see the beginning" for "the device sent nothing before
//! this".
//!
//! **Real captures contain duplicates.** A SPAN or mirror port routinely
//! delivers the same packet twice. Counting frames rather than reassembling
//! them makes a mirrored capture double-count every transaction. Doing the
//! work in sequence space makes duplicate suppression fall out for free.
//!
//! **Gaps must not block for ever.** A missing segment that never arrives —
//! because it was dropped by the capture rather than by the network — would
//! hold everything after it in a buffer indefinitely. There is a bound, and
//! passing it produces a named gap rather than silence.
//!
//! # The overlap policy, written down
//!
//! When a retransmission overlaps bytes already delivered, **the bytes already
//! delivered win** and the overlapping prefix is discarded.
//!
//! This is a choice, not a fact. Operating systems differ, and the difference
//! between first-writer-wins and last-writer-wins is exactly what traffic
//! normalisation and evasion techniques exploit. salman picks the first
//! writer because those bytes have already been handed to a decoder and
//! cannot be recalled, so the alternative would mean the stream salman
//! reported and the stream salman decoded were different streams. The choice
//! is recorded here and in `docs/CONFORMANCE.md` rather than left to be
//! inferred from behaviour.

use core::fmt;
use std::collections::BTreeMap;

use crate::frame::{Endpoint, Segment};

/// How many bytes of out-of-order data one direction may hold before salman
/// gives up on the missing piece.
///
/// A gap that never fills would otherwise hold everything after it for ever.
/// The bound is generous — far more than a Modbus exchange needs — because
/// crossing it means data is being discarded, which must be rare enough to be
/// worth a diagnostic.
pub const MAX_PENDING_BYTES: usize = 64 * 1024;

/// Whether `a` comes before `b` in sequence space.
///
/// Sequence numbers are 32 bits and wrap. `a < b` is wrong exactly when the
/// numbers straddle the wrap, which on a busy connection is a matter of
/// minutes rather than a theoretical concern.
#[must_use]
pub const fn before(a: u32, b: u32) -> bool {
    // The difference, computed with wrapping arithmetic, is small going
    // forwards and enormous going backwards.
    b.wrapping_sub(a) != 0 && b.wrapping_sub(a) < 0x8000_0000
}

/// Whether `a` is at or before `b` in sequence space.
#[must_use]
pub const fn at_or_before(a: u32, b: u32) -> bool {
    a == b || before(a, b)
}

/// How far `b` is after `a`, or `None` if it is before it.
#[must_use]
pub const fn distance(a: u32, b: u32) -> Option<u32> {
    let gap = b.wrapping_sub(a);
    if gap < 0x8000_0000 { Some(gap) } else { None }
}

/// Something worth saying about a stream, alongside the bytes.
///
/// These are **observations**, not faults. A retransmission is ordinary on any
/// real network, and reporting it as a problem is how a diagnostic tool loses
/// the reader's trust. What is done with them is the findings layer's
/// business.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    /// This stream was joined in progress; salman did not see it open.
    MidStream {
        /// The sequence number adopted as the base.
        base: u32,
    },
    /// A segment carried bytes already delivered, and was discarded.
    Retransmission {
        /// How many bytes were already known.
        bytes: usize,
    },
    /// A segment was byte-for-byte identical to one already seen.
    ///
    /// Typical of a mirror or SPAN port delivering each packet twice, which is
    /// why this is separate from a retransmission: nothing on the network
    /// resent anything.
    Duplicate {
        /// How many bytes were repeated.
        bytes: usize,
    },
    /// A retransmission overlapped delivered bytes and disagreed with them.
    ///
    /// The delivered bytes were kept. This is the one note that means
    /// something is genuinely odd: a sender that resent different data for the
    /// same sequence numbers is either broken or trying to be.
    OverlapDisagreed {
        /// Where the disagreement starts.
        sequence: u32,
        /// How many bytes disagreed.
        bytes: usize,
    },
    /// A segment arrived out of order and is being held.
    OutOfOrder {
        /// How far ahead of what was expected.
        ahead: u32,
    },
    /// A hole was given up on, and bytes after it were delivered.
    Gap {
        /// The first sequence number that was never seen.
        from: u32,
        /// How many bytes were skipped.
        bytes: u32,
    },
    /// The stream was closed by a FIN.
    Finished,
    /// The stream was aborted by a RST.
    Reset,
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MidStream { base } => write!(
                f,
                "this stream was joined in progress at sequence {base}; whatever came \
                 before it was not captured"
            ),
            Self::Retransmission { bytes } => {
                write!(f, "{bytes} bytes were sent again and were already known")
            }
            Self::Duplicate { bytes } => write!(
                f,
                "{bytes} bytes arrived twice, identically, which usually means the capture \
                 point is a mirror port rather than that anything was resent"
            ),
            Self::OverlapDisagreed { sequence, bytes } => write!(
                f,
                "{bytes} bytes at sequence {sequence} were sent again with different \
                 contents; salman kept what it had already delivered"
            ),
            Self::OutOfOrder { ahead } => {
                write!(
                    f,
                    "a segment arrived {ahead} bytes ahead of what was expected"
                )
            }
            Self::Gap { from, bytes } => write!(
                f,
                "{bytes} bytes from sequence {from} were never captured, and salman \
                 carried on after them"
            ),
            Self::Finished => f.write_str("this direction was closed"),
            Self::Reset => f.write_str("this connection was reset"),
        }
    }
}

/// What one segment produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delivery {
    /// Contiguous stream bytes, ready for a protocol decoder.
    pub bytes: Vec<u8>,
    /// What was worth saying about the stream while producing them.
    pub notes: Vec<Note>,
}

impl Delivery {
    /// Whether anything at all came of the segment.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty() && self.notes.is_empty()
    }
}

/// One direction of one connection.
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
pub struct Stream {
    next: u32,
    started: bool,
    /// Set when the capture joined this stream after it had already started.
    mid_stream: bool,
    finished: bool,
    pending: BTreeMap<u32, Vec<u8>>,
    pending_bytes: usize,
    delivered: u64,
    /// The last few delivered bytes, so an overlapping retransmission can be
    /// compared rather than assumed identical.
    recent: Vec<u8>,
}

/// How many delivered bytes are kept for comparing against an overlap.
///
/// Enough for any Modbus frame and then some. Keeping the whole stream would
/// mean a capture-sized buffer per direction for a check that only ever looks
/// backwards a little way.
const RECENT_BYTES: usize = 1024;

impl Stream {
    fn new() -> Self {
        Self {
            next: 0,
            started: false,
            mid_stream: false,
            finished: false,
            pending: BTreeMap::new(),
            pending_bytes: 0,
            delivered: 0,
            recent: Vec::new(),
        }
    }

    /// Whether salman joined this stream after it had already started.
    #[must_use]
    pub const fn mid_stream(&self) -> bool {
        self.mid_stream
    }

    /// How many stream bytes have been handed out.
    #[must_use]
    pub const fn delivered(&self) -> u64 {
        self.delivered
    }

    /// How many bytes are being held for a gap that has not filled.
    #[must_use]
    pub const fn pending_bytes(&self) -> usize {
        self.pending_bytes
    }

    /// The sequence number expected next.
    #[must_use]
    pub const fn next_sequence(&self) -> u32 {
        self.next
    }
}

/// Reassembles every stream in a capture.
#[derive(Debug, Clone, Default)]
pub struct Reassembler {
    streams: BTreeMap<(Endpoint, Endpoint), Stream>,
}

impl Reassembler {
    /// An empty reassembler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            streams: BTreeMap::new(),
        }
    }

    /// Feeds one segment in and takes whatever stream bytes come out.
    ///
    /// The key is the **directed** pair, so each direction keeps its own
    /// sequence space.
    pub fn push(&mut self, segment: &Segment<'_>) -> Delivery {
        let key = (segment.source, segment.destination);
        let stream = self.streams.entry(key).or_insert_with(Stream::new);
        let mut delivery = Delivery::default();

        if segment.rst {
            delivery.notes.push(Note::Reset);
        }

        if segment.syn {
            // A SYN occupies one sequence number, so data starts after it.
            stream.next = segment.sequence.wrapping_add(1);
            stream.started = true;
            stream.mid_stream = false;
        } else if !stream.started {
            // No SYN was captured. Adopt what is here as the base and say so,
            // rather than letting anything downstream read "salman did not see
            // the beginning" as "nothing came before".
            stream.next = segment.sequence;
            stream.started = true;
            stream.mid_stream = true;
            delivery.notes.push(Note::MidStream {
                base: segment.sequence,
            });
        }

        if !segment.payload.is_empty() {
            Self::accept(stream, segment.sequence, segment.payload, &mut delivery);
            Self::drain(stream, &mut delivery);
            Self::bound_pending(stream, &mut delivery);
        }

        if segment.fin {
            stream.finished = true;
            delivery.notes.push(Note::Finished);
        }
        delivery
    }

    /// Places one segment's payload, in order or held for later.
    fn accept(stream: &mut Stream, sequence: u32, payload: &[u8], delivery: &mut Delivery) {
        let end = sequence.wrapping_add(payload.len() as u32);

        // Entirely behind what has been delivered: nothing new.
        if at_or_before(end, stream.next) {
            let already = Self::compare_with_recent(stream, sequence, payload);
            delivery.notes.push(if already {
                Note::Duplicate {
                    bytes: payload.len(),
                }
            } else {
                Note::OverlapDisagreed {
                    sequence,
                    bytes: payload.len(),
                }
            });
            return;
        }

        // Straddles the boundary: the overlapping prefix was already
        // delivered, and delivered bytes win. See the module documentation for
        // why that is the policy.
        if before(sequence, stream.next) {
            let Some(overlap) = distance(sequence, stream.next) else {
                return;
            };
            let overlap = overlap as usize;
            let Some(fresh) = payload.get(overlap..) else {
                return;
            };
            let already = payload.get(..overlap).unwrap_or(&[]);
            if Self::compare_with_recent(stream, sequence, already) {
                delivery.notes.push(Note::Retransmission { bytes: overlap });
            } else {
                delivery.notes.push(Note::OverlapDisagreed {
                    sequence,
                    bytes: overlap,
                });
            }
            Self::deliver(stream, fresh, delivery);
            return;
        }

        if sequence == stream.next {
            Self::deliver(stream, payload, delivery);
            return;
        }

        // Ahead of what is expected: hold it until the hole fills.
        if let Some(ahead) = distance(stream.next, sequence) {
            let existing = stream.pending.insert(sequence, payload.to_vec());
            match existing {
                Some(old) if old == payload => {
                    // Already held, identically. Put it back and say so.
                    stream.pending.insert(sequence, old);
                    delivery.notes.push(Note::Duplicate {
                        bytes: payload.len(),
                    });
                }
                Some(old) => {
                    stream.pending_bytes = stream.pending_bytes.saturating_sub(old.len());
                    stream.pending_bytes += payload.len();
                    delivery.notes.push(Note::OverlapDisagreed {
                        sequence,
                        bytes: payload.len(),
                    });
                }
                None => {
                    stream.pending_bytes += payload.len();
                    delivery.notes.push(Note::OutOfOrder { ahead });
                }
            }
        }
    }

    /// Hands bytes to the caller and advances the stream.
    fn deliver(stream: &mut Stream, bytes: &[u8], delivery: &mut Delivery) {
        delivery.bytes.extend_from_slice(bytes);
        stream.next = stream.next.wrapping_add(bytes.len() as u32);
        stream.delivered += bytes.len() as u64;
        stream.recent.extend_from_slice(bytes);
        if stream.recent.len() > RECENT_BYTES {
            let excess = stream.recent.len() - RECENT_BYTES;
            stream.recent.drain(..excess);
        }
    }

    /// Delivers whatever held segments have become contiguous.
    fn drain(stream: &mut Stream, delivery: &mut Delivery) {
        loop {
            let Some((&sequence, _)) = stream.pending.first_key_value() else {
                return;
            };
            if before(stream.next, sequence) {
                // Still a hole in front of it.
                return;
            }
            let Some(held) = stream.pending.remove(&sequence) else {
                return;
            };
            stream.pending_bytes = stream.pending_bytes.saturating_sub(held.len());
            let end = sequence.wrapping_add(held.len() as u32);
            if at_or_before(end, stream.next) {
                // Entirely covered by what has since been delivered.
                continue;
            }
            let skip = distance(sequence, stream.next).unwrap_or(0) as usize;
            let fresh = held.get(skip..).unwrap_or(&[]);
            Self::deliver(stream, fresh, delivery);
        }
    }

    /// Gives up on a hole that has held too much behind it.
    fn bound_pending(stream: &mut Stream, delivery: &mut Delivery) {
        if stream.pending_bytes <= MAX_PENDING_BYTES {
            return;
        }
        // Jump to the earliest held segment. Everything between what was
        // expected and that point was never captured, and saying so is the
        // point: silence would look like the device sent nothing.
        let Some((&sequence, _)) = stream.pending.first_key_value() else {
            return;
        };
        let skipped = distance(stream.next, sequence).unwrap_or(0);
        delivery.notes.push(Note::Gap {
            from: stream.next,
            bytes: skipped,
        });
        stream.next = sequence;
        Self::drain(stream, delivery);
    }

    /// Whether `payload` matches what was delivered at `sequence`, as far as
    /// the recent window can tell.
    ///
    /// Returns `true` when it cannot tell, because reporting a disagreement
    /// salman cannot demonstrate would be a confident lie about a device.
    fn compare_with_recent(stream: &Stream, sequence: u32, payload: &[u8]) -> bool {
        if payload.is_empty() {
            return true;
        }
        // Where in the recent window this sequence falls.
        let recent_start = stream.next.wrapping_sub(stream.recent.len() as u32);
        let Some(offset) = distance(recent_start, sequence) else {
            return true;
        };
        let offset = offset as usize;
        let Some(window) = stream.recent.get(offset..) else {
            return true;
        };
        let comparable = window.len().min(payload.len());
        if comparable == 0 {
            return true;
        }
        window.get(..comparable) == payload.get(..comparable)
    }

    /// One direction of one connection, if anything has been seen of it.
    #[must_use]
    pub fn stream(&self, from: Endpoint, to: Endpoint) -> Option<&Stream> {
        self.streams.get(&(from, to))
    }

    /// Every direction seen, in a stable order.
    pub fn streams(&self) -> impl Iterator<Item = ((Endpoint, Endpoint), &Stream)> {
        self.streams.iter().map(|(k, v)| (*k, v))
    }
}
