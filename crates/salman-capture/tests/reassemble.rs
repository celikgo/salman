// SPDX-License-Identifier: Apache-2.0
//! Turning captured segments back into byte streams.
//!
//! Most of these tests are about things that happen on every real network and
//! that a naive reassembler gets wrong quietly: segments arriving out of
//! order, a mirror port delivering everything twice, a retransmission that
//! overlaps what has already been handed to a decoder, and sequence numbers
//! crossing 2³².
//!
//! The wrap test is the one that would otherwise be found in production. A
//! busy connection passes `0xFFFFFFFF` in minutes, and a reassembler that
//! compares sequence numbers with `<` works perfectly until it does not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_capture::frame::{Address, Endpoint, Segment};
use salman_capture::reassemble::{
    MAX_PENDING_BYTES, Note, Reassembler, at_or_before, before, distance,
};

fn client() -> Endpoint {
    Endpoint {
        address: Address::V4([192, 168, 1, 10]),
        port: 51_000,
    }
}

fn server() -> Endpoint {
    Endpoint {
        address: Address::V4([192, 168, 1, 20]),
        port: 502,
    }
}

/// A segment from the client to the server.
fn segment(sequence: u32, payload: &[u8]) -> Segment<'_> {
    Segment {
        source: client(),
        destination: server(),
        sequence,
        acknowledgement: 0,
        payload,
        syn: false,
        ack: true,
        fin: false,
        rst: false,
        truncated: false,
    }
}

/// The same, the other way round.
fn reply(sequence: u32, payload: &[u8]) -> Segment<'_> {
    Segment {
        source: server(),
        destination: client(),
        sequence,
        acknowledgement: 0,
        payload,
        syn: false,
        ack: true,
        fin: false,
        rst: false,
        truncated: false,
    }
}

fn syn(sequence: u32) -> Segment<'static> {
    Segment {
        syn: true,
        ack: false,
        ..segment(sequence, &[])
    }
}

// -- sequence arithmetic -------------------------------------------------

#[test]
fn sequence_comparison_is_modular() {
    // The whole reason `<` cannot be used. These are true in sequence space
    // and false as plain integers.
    assert!(before(0xFFFF_FFF0, 0x0000_0010));
    assert!(!before(0x0000_0010, 0xFFFF_FFF0));
    assert!(at_or_before(0xFFFF_FFFF, 0xFFFF_FFFF));
    assert_eq!(distance(0xFFFF_FFF0, 0x0000_0010), Some(32));
    assert_eq!(distance(0x0000_0010, 0xFFFF_FFF0), None);

    // And the ordinary cases still behave.
    assert!(before(1, 2));
    assert!(!before(2, 1));
    assert!(!before(5, 5));
    assert_eq!(distance(100, 150), Some(50));
}

#[test]
fn a_stream_that_crosses_the_wrap_stays_contiguous() {
    // A busy connection passes 0xFFFFFFFF in minutes. A reassembler that
    // compared with `<` would treat everything after the wrap as ancient and
    // silently discard it.
    let mut reassembler = Reassembler::new();
    // Six bytes from here crosses zero, which is the whole point.
    let base = 0xFFFF_FFFC_u32;
    reassembler.push(&syn(base.wrapping_sub(1)));

    let first = reassembler.push(&segment(base, b"before"));
    assert_eq!(first.bytes, b"before");

    let after = base.wrapping_add(6);
    assert!(after < base, "this test is pointless unless it wraps");
    let second = reassembler.push(&segment(after, b"after"));
    assert_eq!(second.bytes, b"after");

    let stream = reassembler.stream(client(), server()).unwrap();
    assert_eq!(stream.delivered(), 11);
}

// -- the ordinary path ---------------------------------------------------

#[test]
fn segments_in_order_become_one_stream() {
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(1000));
    let mut stream = Vec::new();
    for (sequence, payload) in [(1001_u32, &b"one"[..]), (1004, b"two"), (1007, b"three")] {
        stream.extend_from_slice(&reassembler.push(&segment(sequence, payload)).bytes);
    }
    assert_eq!(stream, b"onetwothree");
}

#[test]
fn a_syn_occupies_one_sequence_number() {
    // Data starts at seq+1, not seq. Getting this wrong shifts the whole
    // stream by one byte, which corrupts every frame in it.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(5000));
    let delivery = reassembler.push(&segment(5001, b"data"));
    assert_eq!(delivery.bytes, b"data");
    assert!(delivery.notes.is_empty(), "{:?}", delivery.notes);
}

#[test]
fn the_two_directions_are_separate_streams() {
    // They share a connection and nothing else: independent sequence spaces,
    // and mixing them produces bytes that never existed.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(100));
    let out = reassembler.push(&segment(101, b"request"));
    let back = reassembler.push(&reply(9_000_000, b"response"));
    assert_eq!(out.bytes, b"request");
    assert_eq!(back.bytes, b"response");
    assert_eq!(reassembler.streams().count(), 2);
}

// -- out of order --------------------------------------------------------

#[test]
fn a_segment_that_arrives_early_is_held_until_the_hole_fills() {
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));

    let early = reassembler.push(&segment(4, b"second"));
    assert!(
        early.bytes.is_empty(),
        "nothing may be delivered over a hole"
    );
    assert!(matches!(
        early.notes.as_slice(),
        [Note::OutOfOrder { ahead: 3 }]
    ));

    let fills = reassembler.push(&segment(1, b"abc"));
    assert_eq!(
        fills.bytes, b"abcsecond",
        "the held segment follows the one that filled the hole"
    );
    let stream = reassembler.stream(client(), server()).unwrap();
    assert_eq!(stream.pending_bytes(), 0);
}

#[test]
fn several_held_segments_come_out_in_order() {
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));
    reassembler.push(&segment(7, b"ccc"));
    reassembler.push(&segment(4, b"bbb"));
    let delivery = reassembler.push(&segment(1, b"aaa"));
    assert_eq!(delivery.bytes, b"aaabbbccc");
}

#[test]
fn a_hole_that_never_fills_is_given_up_on_rather_than_held_for_ever() {
    // A segment dropped by the capture rather than by the network would
    // otherwise hold everything after it in a buffer indefinitely, and the
    // stream would go silent with nothing saying why.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));

    let chunk = vec![0xAB_u8; 4096];
    let mut sequence = 100_u32;
    let mut gapped = None;
    for _ in 0..(MAX_PENDING_BYTES / chunk.len()) + 2 {
        let delivery = reassembler.push(&segment(sequence, &chunk));
        if delivery.notes.iter().any(|n| matches!(n, Note::Gap { .. })) {
            gapped = Some(delivery);
            break;
        }
        sequence = sequence.wrapping_add(chunk.len() as u32);
    }

    let delivery = gapped.expect("the bound was never reached");
    let Some(Note::Gap { from, bytes }) = delivery
        .notes
        .iter()
        .find(|n| matches!(n, Note::Gap { .. }))
        .cloned()
    else {
        panic!("{:?}", delivery.notes)
    };
    assert_eq!(from, 1, "the hole started right after the SYN");
    assert_eq!(bytes, 99);
    assert!(
        !delivery.bytes.is_empty(),
        "after giving up, the held bytes must actually come out"
    );
}

// -- duplicates and retransmissions --------------------------------------

#[test]
fn a_mirror_port_delivering_everything_twice_does_not_double_the_stream() {
    // Real captures from a SPAN port contain byte-identical packet pairs. A
    // tool that counted frames would double-count every transaction.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));
    let first = reassembler.push(&segment(1, b"payload"));
    let again = reassembler.push(&segment(1, b"payload"));

    assert_eq!(first.bytes, b"payload");
    assert!(again.bytes.is_empty(), "the duplicate was delivered again");
    assert!(
        matches!(again.notes.as_slice(), [Note::Duplicate { bytes: 7 }]),
        "{:?}",
        again.notes
    );
}

#[test]
fn a_retransmission_that_overlaps_delivered_bytes_contributes_only_what_is_new() {
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));
    assert_eq!(reassembler.push(&segment(1, b"abcdef")).bytes, b"abcdef");

    // Resent from three bytes back, carrying three new bytes.
    let delivery = reassembler.push(&segment(4, b"defghi"));
    assert_eq!(delivery.bytes, b"ghi", "only the new bytes");
    assert!(
        delivery
            .notes
            .iter()
            .any(|n| matches!(n, Note::Retransmission { bytes: 3 })),
        "{:?}",
        delivery.notes
    );
}

#[test]
fn a_retransmission_that_disagrees_with_what_was_delivered_says_so() {
    // The one case that means something is genuinely odd: a sender that resent
    // different data for the same sequence numbers. The delivered bytes are
    // kept — they have already gone to a decoder and cannot be recalled — and
    // the disagreement is reported rather than hidden.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));
    reassembler.push(&segment(1, b"abcdef"));

    let delivery = reassembler.push(&segment(4, b"XYZghi"));
    assert_eq!(delivery.bytes, b"ghi");
    assert!(
        delivery
            .notes
            .iter()
            .any(|n| matches!(n, Note::OverlapDisagreed { .. })),
        "{:?}",
        delivery.notes
    );
}

#[test]
fn a_segment_entirely_behind_the_stream_delivers_nothing() {
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));
    reassembler.push(&segment(1, b"abcdefghij"));
    let delivery = reassembler.push(&segment(1, b"abc"));
    assert!(delivery.bytes.is_empty());
    assert!(matches!(
        delivery.notes.as_slice(),
        [Note::Duplicate { .. }]
    ));
}

// -- joining in progress -------------------------------------------------

#[test]
fn a_capture_that_starts_mid_conversation_says_so_and_still_works() {
    // Most captures do. Silence here would let something downstream read "no
    // SYN was captured" as "the device sent nothing before this".
    let mut reassembler = Reassembler::new();
    let delivery = reassembler.push(&segment(777_000, b"already running"));
    assert_eq!(delivery.bytes, b"already running");
    assert!(
        delivery
            .notes
            .iter()
            .any(|n| matches!(n, Note::MidStream { base: 777_000 })),
        "{:?}",
        delivery.notes
    );
    assert!(reassembler.stream(client(), server()).unwrap().mid_stream());

    // And it carries on from there.
    assert_eq!(
        reassembler.push(&segment(777_015, b" still")).bytes,
        b" still"
    );
}

#[test]
fn a_stream_opened_with_a_syn_is_not_marked_mid_stream() {
    let mut reassembler = Reassembler::new();
    let delivery = reassembler.push(&syn(1));
    assert!(delivery.notes.is_empty(), "{:?}", delivery.notes);
    assert!(!reassembler.stream(client(), server()).unwrap().mid_stream());
}

// -- closing -------------------------------------------------------------

#[test]
fn a_fin_and_a_reset_are_both_reported() {
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));
    let finished = reassembler.push(&Segment {
        fin: true,
        ..segment(1, b"last")
    });
    assert_eq!(finished.bytes, b"last");
    assert!(finished.notes.contains(&Note::Finished));

    let reset = reassembler.push(&Segment {
        rst: true,
        ..segment(5, &[])
    });
    assert!(reset.notes.contains(&Note::Reset));
}

// -- properties ----------------------------------------------------------

#[test]
fn the_stream_that_comes_out_is_the_stream_that_went_in_however_it_was_cut() {
    // The property the whole module exists for. The same bytes, split into
    // segments at every possible boundary, must reassemble identically.
    let payload: Vec<u8> = (0..97_u8).collect();
    for chunk in 1..=16_usize {
        let mut reassembler = Reassembler::new();
        reassembler.push(&syn(0));
        let mut out = Vec::new();
        let mut sequence = 1_u32;
        for piece in payload.chunks(chunk) {
            out.extend_from_slice(&reassembler.push(&segment(sequence, piece)).bytes);
            sequence = sequence.wrapping_add(piece.len() as u32);
        }
        assert_eq!(out, payload, "cut into {chunk}-byte pieces");
    }
}

#[test]
fn reordering_the_segments_does_not_change_the_stream() {
    // Delivery order is the network's business. What comes out must not
    // depend on it.
    let payload: Vec<u8> = (0..64_u8).collect();
    let pieces: Vec<(u32, &[u8])> = payload
        .chunks(8)
        .enumerate()
        .map(|(index, piece)| (1 + (index * 8) as u32, piece))
        .collect();

    // Every rotation of the arrival order.
    for rotation in 0..pieces.len() {
        let mut reassembler = Reassembler::new();
        reassembler.push(&syn(0));
        let mut out = Vec::new();
        for offset in 0..pieces.len() {
            let (sequence, piece) = pieces[(offset + rotation) % pieces.len()];
            out.extend_from_slice(&reassembler.push(&segment(sequence, piece)).bytes);
        }
        assert_eq!(out, payload, "rotated by {rotation}");
    }
}

#[test]
fn no_sequence_of_segments_makes_the_reassembler_panic() {
    let mut seed = 0x1234_5678_9ABC_DEF0_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut reassembler = Reassembler::new();
    let payload = [0x5A_u8; 64];
    for _ in 0..50_000 {
        let sequence = (next() >> 32) as u32;
        let length = (next() % 65) as usize;
        let flags = next();
        let piece = payload.get(..length).unwrap_or(&[]);
        let outgoing = flags % 2 == 0;
        let base = if outgoing {
            segment(sequence, piece)
        } else {
            reply(sequence, piece)
        };
        let _ = reassembler.push(&Segment {
            syn: flags & 0x04 != 0,
            fin: flags & 0x08 != 0,
            rst: flags & 0x10 != 0,
            ..base
        });
    }
}

#[test]
fn a_stream_never_delivers_more_than_it_was_given() {
    // A reassembler that duplicated data would be worse than one that dropped
    // it: the extra bytes look like traffic the device never sent.
    let mut seed = 0x0F0F_0F0F_0F0F_0F0F_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));

    let source: Vec<u8> = (0..=255_u8).cycle().take(4096).collect();
    let mut delivered = Vec::new();
    // Send every chunk twice, in a jumbled order, as a lossy mirrored capture
    // would.
    let mut order: Vec<usize> = (0..source.len() / 32).collect();
    for index in 0..order.len() {
        let swap = (next() as usize) % order.len();
        order.swap(index, swap);
    }
    for pass in 0..2 {
        for &index in &order {
            let start = index * 32;
            let piece = source.get(start..start + 32).unwrap_or(&[]);
            let sequence = 1 + start as u32;
            let delivery = reassembler.push(&segment(sequence, piece));
            delivered.extend_from_slice(&delivery.bytes);
        }
        let _ = pass;
    }
    assert_eq!(
        delivered, source,
        "the stream delivered is not the stream sent"
    );
}

// -- what review found ---------------------------------------------------

#[test]
fn a_segment_held_across_the_wrap_is_delivered_rather_than_stranded() {
    // Found by review, with this exact sequence. The held segments were keyed
    // on the raw sequence number in a map ordered numerically, so a post-wrap
    // segment sorted ahead of a pre-wrap one — and the pre-wrap segment,
    // sitting exactly where the stream expected it, was never looked at. It
    // was then reported as "never captured" while salman held it.
    //
    // The one comparison that was not modular was the one nobody had written
    // down: the ordering inside the data structure.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0xFFFF_FEFF));
    // next is now 0xFFFF_FF00.

    let held_before_wrap = reassembler.push(&segment(0xFFFF_FF10, b"AAAAAAAAAAAAAAAA"));
    assert!(held_before_wrap.bytes.is_empty());
    let held_after_wrap = reassembler.push(&segment(0x0000_0000, b"BBBBBBBBBBBBBBBB"));
    assert!(held_after_wrap.bytes.is_empty());

    // The segment that fills the hole. Everything contiguous must follow it.
    let fills = reassembler.push(&segment(0xFFFF_FF00, b"CCCCCCCCCCCCCCCC"));
    assert_eq!(
        fills.bytes, b"CCCCCCCCCCCCCCCCAAAAAAAAAAAAAAAA",
        "the segment held across the wrap was stranded"
    );

    let stream = reassembler.stream(client(), server()).unwrap();
    assert_eq!(stream.next_sequence(), 0xFFFF_FF20);
    // The post-wrap segment is still held, correctly: there is a real hole
    // between 0xFFFFFF20 and 0x00000000.
    assert_eq!(stream.pending_bytes(), 16);
}

#[test]
fn the_pending_bound_is_a_bound() {
    // Found by review. Giving up on one hole per segment is not a bound: a
    // stream with many holes grows past it for ever. Worse than the memory,
    // once past it the jump fired on every packet and skipped forward over
    // live data, producing a byte stream no sender ever sent.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));

    // Alternating loss: every other byte never arrives. Each one is its own
    // hole, so nothing can ever be delivered and the buffer only grows.
    let byte = [0xAA_u8];
    for index in 0..40_000_u32 {
        reassembler.push(&segment(2 + index * 2, &byte));
    }
    let held = reassembler
        .stream(client(), server())
        .unwrap()
        .pending_bytes();
    assert!(
        held <= MAX_PENDING_BYTES,
        "the buffer holds {held} bytes against a bound of {MAX_PENDING_BYTES}"
    );
}

#[test]
fn a_shorter_retransmission_of_a_held_segment_does_not_discard_bytes() {
    // Found by review. A retransmission replaced a held segment wholesale, so
    // a shorter one — which happens whenever a sender resegments — threw away
    // captured bytes with nothing saying so, and reported a disagreement
    // between two things that agreed everywhere they overlapped.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));

    // Held ahead of a hole.
    reassembler.push(&segment(11, b"HELLOWORLD"));
    // The same data, resegmented shorter.
    let again = reassembler.push(&segment(11, b"HELLO"));
    assert!(
        !again
            .notes
            .iter()
            .any(|n| matches!(n, Note::OverlapDisagreed { .. })),
        "two segments that agree were reported as disagreeing: {:?}",
        again.notes
    );

    // Fill the hole; all ten bytes must still be there.
    let fills = reassembler.push(&segment(1, b"0123456789"));
    assert_eq!(
        fills.bytes, b"0123456789HELLOWORLD",
        "the longer held copy was replaced by the shorter one"
    );
}

#[test]
fn a_longer_retransmission_of_a_held_segment_keeps_the_extra_bytes() {
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));
    reassembler.push(&segment(11, b"HELLO"));
    reassembler.push(&segment(11, b"HELLOWORLD"));
    let fills = reassembler.push(&segment(1, b"0123456789"));
    assert_eq!(fills.bytes, b"0123456789HELLOWORLD");
}

#[test]
fn an_overlap_is_never_compared_against_the_wrong_bytes_after_a_gap() {
    // The recent window used to be located by subtracting its length from the
    // stream position, which stops being true the moment the position jumps
    // over a hole. An overlap would then be compared against bytes from
    // somewhere else entirely and a disagreement reported that did not exist.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));
    reassembler.push(&segment(1, b"AAAAAAAA"));

    // Force a jump by overflowing the bound with one far-ahead segment.
    let big = vec![0xBB_u8; MAX_PENDING_BYTES + 1];
    let jumped = reassembler.push(&segment(100_000, &big));
    assert!(
        jumped.notes.iter().any(|n| matches!(n, Note::Gap { .. })),
        "{:?}",
        jumped.notes
    );

    // Now a retransmission of bytes from before the jump. salman cannot tell
    // whether they agree — the window was cleared — and must not claim they
    // disagree.
    let old = reassembler.push(&segment(1, b"AAAAAAAA"));
    assert!(
        !old.notes
            .iter()
            .any(|n| matches!(n, Note::OverlapDisagreed { .. })),
        "a disagreement was claimed that salman cannot demonstrate: {:?}",
        old.notes
    );
}

#[test]
fn a_syn_carrying_data_keeps_all_of_it() {
    // Found by review. A SYN occupies one sequence number of its own, and any
    // payload it carries starts after that number — so passing the SYN's own
    // number made the payload look as though it overlapped by a byte. The
    // first byte was dropped as already delivered and the rest reported as a
    // retransmission. TCP Fast Open produces exactly this segment.
    let mut reassembler = Reassembler::new();
    let delivery = reassembler.push(&Segment {
        syn: true,
        ..segment(1000, b"early data")
    });
    assert_eq!(delivery.bytes, b"early data", "the first byte was lost");
    assert!(
        !delivery
            .notes
            .iter()
            .any(|n| matches!(n, Note::Retransmission { .. })),
        "a phantom retransmission was reported: {:?}",
        delivery.notes
    );
    // And the stream carries on from the right place.
    assert_eq!(reassembler.push(&segment(1011, b" more")).bytes, b" more");
}

#[test]
fn a_repeat_too_far_back_to_compare_is_not_called_a_duplicate() {
    // Found by review. `Duplicate` says the bytes were byte-for-byte
    // identical, and salman keeps a bounded window of delivered bytes — so
    // anything older than that window was never compared. Claiming identity
    // there is a positive claim nothing checked; claiming disagreement is a
    // claim about a device salman cannot support. Neither is true.
    let mut reassembler = Reassembler::new();
    reassembler.push(&syn(0));

    // Deliver well past the recent window.
    let filler = vec![0x55_u8; 4096];
    reassembler.push(&segment(1, &filler));

    // Now resend the very first bytes, long since out of the window.
    let repeat = reassembler.push(&segment(1, b"XXXX"));
    assert!(
        repeat
            .notes
            .iter()
            .any(|n| matches!(n, Note::Unverified { bytes: 4 })),
        "expected an unverified note, got {:?}",
        repeat.notes
    );
    assert!(
        !repeat
            .notes
            .iter()
            .any(|n| matches!(n, Note::Duplicate { .. } | Note::OverlapDisagreed { .. })),
        "salman claimed something it could not check: {:?}",
        repeat.notes
    );

    // And a repeat that IS inside the window is still compared properly.
    let recent = reassembler.push(&segment(4000, &filler[..8]));
    assert!(
        recent
            .notes
            .iter()
            .any(|n| matches!(n, Note::Duplicate { .. })),
        "{:?}",
        recent.notes
    );
}
