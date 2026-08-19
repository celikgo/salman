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
//! at exactly the moment the comparison matters.
//!
//! Rather than remember to compare modularly everywhere, each stream converts
//! every sequence number it sees into an **absolute position** — a 64-bit
//! count of bytes from where the stream was joined — and does all its ordering
//! and arithmetic on that. Held segments are keyed by absolute position, so
//! the ordering of the buffer that holds them is right by construction.
//!
//! That last part is why. An earlier version compared modularly everywhere it
//! wrote a comparison, and keyed its held segments on the raw sequence number
//! in a `BTreeMap` — whose ordering is numeric, and which therefore put a
//! post-wrap segment ahead of a pre-wrap one. The one comparison that was not
//! modular was the one nobody had written down. It stranded captured bytes and
//! then reported them as never captured.
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
    /// A segment carried bytes salman already has, and agreed with them
    /// everywhere it could check.
    ///
    /// Typical of a mirror or SPAN port delivering each packet twice, which is
    /// why this is separate from a retransmission: nothing on the network
    /// resent anything.
    ///
    /// **"Everywhere it could check" is load-bearing.** salman keeps a bounded
    /// window of delivered bytes, so bytes older than that window cannot be
    /// compared at all. [`Note::Unverified`] is what salman says then — this
    /// note is only for a comparison that actually happened.
    Duplicate {
        /// How many bytes were repeated.
        bytes: usize,
    },
    /// A segment carried bytes salman already has and could not compare.
    ///
    /// They are too far back for the window of delivered bytes salman keeps.
    /// Reporting this as a duplicate would be a positive claim of identity
    /// that nothing checked, and reporting it as a disagreement would be a
    /// claim about a device that salman cannot support. Neither is true, so
    /// this says what actually happened.
    Unverified {
        /// How many bytes could not be compared.
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
            Self::Unverified { bytes } => write!(
                f,
                "{bytes} bytes arrived again from too far back to compare, so salman cannot \
                 say whether they were the same bytes"
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

/// What comparing repeated bytes against delivered ones established.
///
/// Three answers rather than two, because "salman could not compare these"
/// is not the same as "these were the same" and reporting it as such would be
/// a positive claim of identity that nothing checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comparison {
    /// They agreed everywhere they could be compared.
    Same,
    /// They disagreed.
    Different,
    /// Too far back to compare against anything salman still holds.
    Unknown,
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
    /// The sequence number of the next byte to deliver.
    next_seq: u32,
    /// That same point, as a count of bytes from where the stream was joined.
    ///
    /// Everything that orders or measures works on this rather than on the
    /// sequence number, so nothing here has to remember to compare modularly.
    next_abs: u64,
    started: bool,
    /// Set when the capture joined this stream after it had already started.
    mid_stream: bool,
    finished: bool,
    /// Held segments, keyed by **absolute position**. Ordinary ordering.
    pending: BTreeMap<u64, Vec<u8>>,
    pending_bytes: usize,
    delivered: u64,
    /// The last few delivered bytes, so an overlapping retransmission can be
    /// compared rather than assumed identical.
    recent: Vec<u8>,
    /// The absolute position the first byte of `recent` sits at.
    ///
    /// Kept explicitly rather than derived from `next_abs` and `recent.len()`,
    /// because `next_abs` jumps when a hole is given up on and the two would
    /// then disagree — comparing an overlap against the wrong bytes.
    recent_at: u64,
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
            next_seq: 0,
            next_abs: 0,
            started: false,
            mid_stream: false,
            finished: false,
            pending: BTreeMap::new(),
            pending_bytes: 0,
            delivered: 0,
            recent: Vec::new(),
            recent_at: 0,
        }
    }

    /// Where `sequence` sits, as an absolute position.
    ///
    /// `None` when it is further from what is expected than half the sequence
    /// space, which is not a distance any real segment is at: it means the
    /// numbers belong to a different connection, or the capture is
    /// nonsensical. Refusing beats placing it somewhere arbitrary.
    fn absolute(&self, sequence: u32) -> Option<u64> {
        // Cast to i32 to read the wrapped difference as a signed distance,
        // which is correct for anything within 2^31 either way.
        let delta = sequence.wrapping_sub(self.next_seq).cast_signed();
        if delta >= 0 {
            self.next_abs.checked_add(delta.unsigned_abs().into())
        } else {
            self.next_abs.checked_sub(delta.unsigned_abs().into())
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
        self.next_seq
    }

    /// Moves the stream on by `bytes`, in both spaces at once.
    fn advance(&mut self, bytes: u64) {
        self.next_seq = self.next_seq.wrapping_add(bytes as u32);
        self.next_abs = self.next_abs.saturating_add(bytes);
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
            // A SYN occupies one sequence number of its own, and any payload
            // it carries starts *after* that number. Treating the SYN's
            // sequence as the first data byte loses the first byte of the
            // payload and then reports the rest as a retransmission — which is
            // what happened before, on the rare but legal SYN-with-data that
            // TCP Fast Open produces.
            stream.next_seq = segment.sequence.wrapping_add(1);
            stream.next_abs = 0;
            stream.started = true;
            stream.mid_stream = false;
            stream.recent.clear();
            stream.recent_at = 0;
        } else if !stream.started {
            // No SYN was captured. Adopt what is here as the base and say so,
            // rather than letting anything downstream read "salman did not see
            // the beginning" as "nothing came before".
            stream.next_seq = segment.sequence;
            stream.next_abs = 0;
            stream.started = true;
            stream.mid_stream = true;
            stream.recent_at = 0;
            delivery.notes.push(Note::MidStream {
                base: segment.sequence,
            });
        }

        if !segment.payload.is_empty() {
            // A SYN's payload starts one past the SYN's own sequence number,
            // because the flag occupies that number. Passing the SYN's number
            // makes the payload look as though it overlaps by a byte: the
            // first byte is dropped as already delivered and the rest reported
            // as a retransmission. TCP Fast Open produces exactly this segment.
            let payload_at = if segment.syn {
                segment.sequence.wrapping_add(1)
            } else {
                segment.sequence
            };
            Self::accept(stream, payload_at, segment.payload, &mut delivery);
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
        let Some(start) = stream.absolute(sequence) else {
            // Further from what is expected than half the sequence space. No
            // real segment is at that distance; placing it somewhere would be
            // inventing a position for it.
            delivery.notes.push(Note::OutOfOrder { ahead: u32::MAX });
            return;
        };
        let end = start.saturating_add(payload.len() as u64);
        let next = stream.next_abs;

        // Entirely behind what has been delivered: nothing new.
        if end <= next {
            delivery
                .notes
                .push(match Self::agrees_with_delivered(stream, start, payload) {
                    Comparison::Same => Note::Duplicate {
                        bytes: payload.len(),
                    },
                    Comparison::Different => Note::OverlapDisagreed {
                        sequence,
                        bytes: payload.len(),
                    },
                    Comparison::Unknown => Note::Unverified {
                        bytes: payload.len(),
                    },
                });
            return;
        }

        // Straddles the boundary: the overlapping prefix was already
        // delivered, and delivered bytes win. See the module documentation for
        // why that is the policy.
        if start < next {
            let overlap = (next - start) as usize;
            let Some(fresh) = payload.get(overlap..) else {
                return;
            };
            let already = payload.get(..overlap).unwrap_or(&[]);
            delivery
                .notes
                .push(match Self::agrees_with_delivered(stream, start, already) {
                    Comparison::Same => Note::Retransmission { bytes: overlap },
                    Comparison::Different => Note::OverlapDisagreed {
                        sequence,
                        bytes: overlap,
                    },
                    Comparison::Unknown => Note::Unverified { bytes: overlap },
                });
            Self::deliver(stream, fresh, delivery);
            return;
        }

        if start == next {
            Self::deliver(stream, payload, delivery);
            return;
        }

        // Ahead of what is expected: hold it until the hole fills.
        Self::hold(stream, start, payload, delivery);
    }

    /// Holds a segment that arrived early, keeping whichever copy is longer.
    ///
    /// A retransmission of a held segment used to replace it wholesale. When
    /// the retransmission was shorter — which happens when a sender resegments
    /// — that discarded captured bytes with nothing saying so, and reported a
    /// disagreement between two things that agreed everywhere they overlapped.
    fn hold(stream: &mut Stream, start: u64, payload: &[u8], delivery: &mut Delivery) {
        let ahead = (start - stream.next_abs).min(u64::from(u32::MAX)) as u32;
        match stream.pending.get(&start) {
            Some(held) if held.len() >= payload.len() => {
                // Already have this much or more. If the shared prefix agrees,
                // nothing was resent that salman does not have.
                let shared = payload.len().min(held.len());
                let same = held.get(..shared) == payload.get(..shared);
                delivery.notes.push(if same {
                    Note::Duplicate {
                        bytes: payload.len(),
                    }
                } else {
                    Note::OverlapDisagreed {
                        sequence: stream.next_seq.wrapping_add(ahead),
                        bytes: shared,
                    }
                });
            }
            Some(held) => {
                // The new copy is longer. Keep it, and say only what is true
                // about the part salman already had.
                let shared = held.len();
                let same = held.as_slice() == payload.get(..shared).unwrap_or(&[]);
                let previous = held.len();
                stream.pending.insert(start, payload.to_vec());
                stream.pending_bytes = stream.pending_bytes.saturating_sub(previous);
                stream.pending_bytes += payload.len();
                delivery.notes.push(if same {
                    Note::Retransmission { bytes: shared }
                } else {
                    Note::OverlapDisagreed {
                        sequence: stream.next_seq.wrapping_add(ahead),
                        bytes: shared,
                    }
                });
            }
            None => {
                stream.pending.insert(start, payload.to_vec());
                stream.pending_bytes += payload.len();
                delivery.notes.push(Note::OutOfOrder { ahead });
            }
        }
    }

    /// Hands bytes to the caller and advances the stream.
    fn deliver(stream: &mut Stream, bytes: &[u8], delivery: &mut Delivery) {
        delivery.bytes.extend_from_slice(bytes);
        stream.advance(bytes.len() as u64);
        stream.recent.extend_from_slice(bytes);
        stream.delivered += bytes.len() as u64;
        if stream.recent.len() > RECENT_BYTES {
            let excess = stream.recent.len() - RECENT_BYTES;
            stream.recent.drain(..excess);
            stream.recent_at += excess as u64;
        }
    }

    /// Delivers whatever held segments have become contiguous.
    fn drain(stream: &mut Stream, delivery: &mut Delivery) {
        loop {
            let Some((&start, _)) = stream.pending.first_key_value() else {
                return;
            };
            if start > stream.next_abs {
                // Still a hole in front of it.
                return;
            }
            let Some(held) = stream.pending.remove(&start) else {
                return;
            };
            stream.pending_bytes = stream.pending_bytes.saturating_sub(held.len());
            let end = start.saturating_add(held.len() as u64);
            if end <= stream.next_abs {
                // Entirely covered by what has since been delivered.
                continue;
            }
            let skip = (stream.next_abs - start) as usize;
            let fresh = held.get(skip..).unwrap_or(&[]);
            Self::deliver(stream, fresh, delivery);
        }
    }

    /// Gives up on holes until the buffer is back inside its bound.
    ///
    /// A loop, not one jump. Giving up on a single hole per segment is not a
    /// bound at all: a stream with many holes grows past it for ever, and once
    /// past it the jump fires on every packet and skips forward over live data,
    /// producing a byte stream no sender ever sent.
    fn bound_pending(stream: &mut Stream, delivery: &mut Delivery) {
        // Bounded so that a pathological stream cannot spin here even if the
        // arithmetic below is ever wrong again.
        for _ in 0..64 {
            if stream.pending_bytes <= MAX_PENDING_BYTES {
                return;
            }
            let Some((&start, _)) = stream.pending.first_key_value() else {
                return;
            };
            if start <= stream.next_abs {
                // Nothing in front of it to give up on; drain will take it.
                Self::drain(stream, delivery);
                continue;
            }
            let skipped = start - stream.next_abs;
            delivery.notes.push(Note::Gap {
                from: stream.next_seq,
                bytes: skipped.min(u64::from(u32::MAX)) as u32,
            });
            // Jump the hole in both spaces, and forget the recent window: the
            // bytes it holds are no longer adjacent to where the stream now is,
            // so comparing an overlap against them would compare the wrong
            // bytes.
            stream.advance(skipped);
            stream.recent.clear();
            stream.recent_at = stream.next_abs;
            Self::drain(stream, delivery);
        }
    }

    /// What comparing a repeat against what was delivered established.
    fn agrees_with_delivered(stream: &Stream, start: u64, payload: &[u8]) -> Comparison {
        if payload.is_empty() {
            return Comparison::Same;
        }
        let Some(offset) = start.checked_sub(stream.recent_at) else {
            // Before anything still held: too far back to compare.
            return Comparison::Unknown;
        };
        let Ok(offset) = usize::try_from(offset) else {
            return Comparison::Unknown;
        };
        let Some(window) = stream.recent.get(offset..) else {
            return Comparison::Unknown;
        };
        let comparable = window.len().min(payload.len());
        if comparable == 0 {
            return Comparison::Unknown;
        }
        if window.get(..comparable) == payload.get(..comparable) {
            Comparison::Same
        } else {
            Comparison::Different
        }
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
