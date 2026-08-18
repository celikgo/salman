// SPDX-License-Identifier: Apache-2.0
//! The TCP stream framer, against a hostile byte stream.
//!
//! The property that matters is **split-independence**: what comes out must
//! not depend on where the segment boundaries fell. A framer can pass every
//! fixed test and still lose a frame when a header lands across a boundary,
//! and a fuzzer finds that in seconds where a fixed test never will.
//!
//! It also asserts that the framer always makes progress. A framer that
//! returned a frame while consuming nothing would spin for ever in every
//! caller's loop, which is a hang rather than a crash and is exactly the
//! failure a naive never-panics target misses.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_modbus::tcp::{Framer, TcpAdu};

/// Feeds a stream in the given segment sizes and returns what came out, or
/// `None` if framing failed.
fn frames(stream: &[u8], segments: &[usize]) -> Option<Vec<TcpAdu>> {
    let mut framer = Framer::new();
    let mut out = Vec::new();
    let mut offset = 0;
    let mut sizes = segments.iter().copied().cycle();

    while offset < stream.len() {
        let size = sizes.next().unwrap_or(1).max(1).min(stream.len() - offset);
        let segment = &stream[offset..offset + size];
        offset += size;

        let mut rest = segment;
        loop {
            let (used, outcome) = framer.advance(rest);
            match outcome {
                Ok(Some(frame)) => {
                    // Progress: a frame can never arrive for free, or every
                    // caller's loop hangs.
                    assert!(used > 0, "a frame was delivered without consuming input");
                    out.push(frame);
                }
                Ok(None) => break,
                Err(error) => {
                    assert!(error.is_fatal());
                    return None;
                }
            }
            rest = rest.get(used..).unwrap_or(&[]);
            if rest.is_empty() {
                break;
            }
        }
    }
    Some(out)
}

fuzz_target!(|data: &[u8]| {
    if data.len() > 8 * 1024 {
        return;
    }

    // The whole stream in one delivery is the reference.
    let whole = frames(data, &[usize::MAX]);

    // Every frame that came out must be one that could have been sent: the
    // length field and the PDU it delimited have to agree, or the framer
    // delivered bytes it had not accounted for.
    if let Some(ref list) = whole {
        for frame in list {
            assert_eq!(frame.header.protocol, 0);
            assert_eq!(
                frame.header.length as usize,
                frame.pdu.len() + 1,
                "the delivered length disagrees with the delivered PDU"
            );
            assert_eq!(frame.header.claimed_pdu_len(), Some(frame.pdu.len()));
        }
    }

    // Now the same bytes cut differently. Split-independence is the property
    // the design exists to give, and the only one worth this much machinery.
    for pattern in [&[1_usize][..], &[2][..], &[3, 1][..], &[7][..], &[5, 11][..]] {
        assert_eq!(
            frames(data, pattern),
            whole,
            "segment sizes {pattern:?} changed what came out of {data:02X?}"
        );
    }
});
