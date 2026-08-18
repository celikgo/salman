// SPDX-License-Identifier: Apache-2.0
// TEMPORARY review scratch tests. Delete after use.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_capture::frame::{Address, Endpoint, Segment};
use salman_capture::reassemble::{MAX_PENDING_BYTES, Note, Reassembler};

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

fn syn(sequence: u32) -> Segment<'static> {
    Segment {
        syn: true,
        ack: false,
        ..segment(sequence, &[])
    }
}

/// Out-of-order hold across the 2^32 wrap: pending is a BTreeMap keyed by the
/// raw u32, so ordering is numeric, not modular.
#[test]
fn scratch_wrap_pending_order() {
    let mut r = Reassembler::new();
    // base such that next = 0xFFFF_FF00
    r.push(&syn(0xFFFF_FEFF));
    let s = r.stream(client(), server()).unwrap();
    assert_eq!(s.next_sequence(), 0xFFFF_FF00);

    // Segment A: seq 0xFFFF_FF10, 16 bytes -> held (hole of 0x10 in front)
    let a = r.push(&segment(0xFFFF_FF10, &[b'A'; 16]));
    println!("A: bytes={:?} notes={:?}", a.bytes.len(), a.notes);

    // Segment B: seq 0x0000_0000 (after wrap), 16 bytes -> held further ahead
    let b = r.push(&segment(0x0000_0000, &[b'B'; 16]));
    println!("B: bytes={:?} notes={:?}", b.bytes.len(), b.notes);

    // Segment C fills the hole: seq 0xFFFF_FF00, 16 bytes -> in order
    let c = r.push(&segment(0xFFFF_FF00, &[b'C'; 16]));
    println!("C: bytes={} notes={:?}", c.bytes.len(), c.notes);
    let s = r.stream(client(), server()).unwrap();
    println!(
        "after C: next={:08X} delivered={} pending={}",
        s.next_sequence(),
        s.delivered(),
        s.pending_bytes()
    );
    assert_eq!(
        c.bytes.len(),
        32,
        "C should have delivered C+A = 32 bytes; got {}",
        c.bytes.len()
    );
}

/// pending_bytes can exceed MAX_PENDING_BYTES without bound, because
/// bound_pending gives up on at most one hole per pushed segment.
#[test]
fn scratch_pending_bound_is_not_a_bound() {
    let mut r = Reassembler::new();
    r.push(&syn(0));
    // 70_000 one-byte holes at even sequences starting at 2 (seq 1 never sent).
    let one = [0xAA_u8; 1];
    for i in 0..70_000_u32 {
        r.push(&segment(2 + i * 2, &one));
    }
    let s = r.stream(client(), server()).unwrap();
    println!("after fillers: pending={}", s.pending_bytes());

    // Now push large far-ahead segments; each adds 1400 and only frees ~1.
    let big = [0xBB_u8; 1400];
    let mut seq = 0x4000_0000_u32;
    for _ in 0..200 {
        r.push(&segment(seq, &big));
        seq = seq.wrapping_add(4000);
    }
    let s = r.stream(client(), server()).unwrap();
    println!(
        "after big: pending={} (MAX={})",
        s.pending_bytes(),
        MAX_PENDING_BYTES
    );
    assert!(
        s.pending_bytes() <= MAX_PENDING_BYTES,
        "pending {} exceeds documented bound {}",
        s.pending_bytes(),
        MAX_PENDING_BYTES
    );
}

/// After a Gap jump, `recent` is left holding bytes from before the gap, but
/// the window's start is recomputed from the new `next`.
#[test]
fn scratch_recent_window_after_gap() {
    let mut r = Reassembler::new();
    r.push(&syn(0));
    // Deliver 1024 bytes in order: seq 1 .. 1025
    let filler: Vec<u8> = (0..1024_u32).map(|i| (i % 251) as u8).collect();
    let d = r.push(&segment(1, &filler));
    assert_eq!(d.bytes.len(), 1024);

    // Now open a hole and overflow the pending bound so a Gap is declared.
    let chunk = vec![0xCC_u8; 4096];
    let mut seq = 100_000_u32;
    let mut gapped = None;
    for _ in 0..24 {
        let del = r.push(&segment(seq, &chunk));
        if del.notes.iter().any(|n| matches!(n, Note::Gap { .. })) {
            gapped = Some(del);
            break;
        }
        seq = seq.wrapping_add(chunk.len() as u32);
    }
    let g = gapped.expect("no gap");
    println!("gap notes={:?} delivered_now={}", g.notes, g.bytes.len());
    let s = r.stream(client(), server()).unwrap();
    let next = s.next_sequence();
    println!("next after gap = {next}");

    // A retransmission of bytes that WERE delivered after the gap, matching
    // exactly. It must not be reported as a disagreement.
    let retrans_seq = next.wrapping_sub(200);
    let del = r.push(&segment(retrans_seq, &[0xCC_u8; 200]));
    println!("retrans notes={:?}", del.notes);
    assert!(
        !del.notes
            .iter()
            .any(|n| matches!(n, Note::OverlapDisagreed { .. })),
        "identical retransmission reported as a disagreement: {:?}",
        del.notes
    );
}

/// A repacketised retransmission of a *held* (not yet delivered) segment
/// destroys bytes salman already had.
#[test]
fn scratch_pending_replacement_loses_bytes() {
    let mut r = Reassembler::new();
    r.push(&syn(0));
    // Held out of order: seq 101, 100 bytes.
    r.push(&segment(101, &[b'X'; 100]));
    // Retransmitted repacketised: seq 101, only the first 50 bytes.
    let d = r.push(&segment(101, &[b'X'; 50]));
    println!("replace notes={:?}", d.notes);
    // Fill the hole.
    let fill = r.push(&segment(1, &[b'F'; 100]));
    println!(
        "fill bytes={} notes={:?}",
        fill.bytes.len(),
        fill.notes
    );
    assert_eq!(
        fill.bytes.len(),
        200,
        "bytes 151..201 were captured and then thrown away"
    );
}

/// A SYN carrying data (TCP Fast Open): the payload starts at seq+1.
#[test]
fn scratch_syn_with_payload() {
    let mut r = Reassembler::new();
    let d = r.push(&Segment {
        syn: true,
        ..segment(1000, b"hello")
    });
    println!("syn+data bytes={:?} notes={:?}", d.bytes, d.notes);
    assert_eq!(d.bytes, b"hello");
}

/// A Duplicate note asserts byte-for-byte identity. Beyond the 1024-byte
/// recent window it cannot be checked, and is asserted anyway.
#[test]
fn scratch_duplicate_claim_beyond_recent_window() {
    let mut r = Reassembler::new();
    r.push(&syn(0));
    let a: Vec<u8> = vec![b'A'; 3000];
    r.push(&segment(1, &a));
    // Resend the same range with *different* content, entirely behind next.
    let d = r.push(&segment(1, &[b'Z'; 3000]));
    println!("stale overlap notes={:?}", d.notes);
    assert!(
        !d.notes.iter().any(|n| matches!(n, Note::Duplicate { .. })),
        "claimed byte-for-byte identity for bytes it never compared: {:?}",
        d.notes
    );
}
