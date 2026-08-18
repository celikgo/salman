// SPDX-License-Identifier: Apache-2.0
//! TCP reassembly, against segments in any order with any sequence numbers.
//!
//! The two properties that matter:
//!
//! * **the reassembler never invents bytes.** Over a whole run it may deliver
//!   no more than was pushed into it. A reassembler that duplicated data would
//!   be worse than one that dropped it, because the extra bytes look like
//!   traffic a device never sent;
//! * **it always makes progress.** Sequence numbers wrap, segments arrive out
//!   of order, and holes never fill; none of that may make it hold bytes for
//!   ever or spin.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_capture::frame::{Address, Endpoint, Segment};
use salman_capture::reassemble::{MAX_PENDING_BYTES, Reassembler};

fn endpoint(port: u16) -> Endpoint {
    Endpoint {
        address: Address::V4([10, 0, 0, u8::try_from(port % 251).unwrap_or(1)]),
        port,
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() > 8 * 1024 {
        return;
    }
    let mut reassembler = Reassembler::new();
    let mut pushed = 0_u64;
    let mut delivered = 0_u64;

    // Read the input as a script of segments: six header bytes then a body.
    let mut rest = data;
    while rest.len() >= 6 {
        let (header, body) = rest.split_at(6);
        let sequence = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let flags = header[4];
        let take = usize::from(header[5]).min(body.len());
        let (payload, remainder) = body.split_at(take);
        rest = remainder;

        let outgoing = flags & 0x80 == 0;
        let segment = Segment {
            source: if outgoing { endpoint(1000) } else { endpoint(502) },
            destination: if outgoing { endpoint(502) } else { endpoint(1000) },
            sequence,
            acknowledgement: 0,
            payload,
            syn: flags & 0x01 != 0,
            ack: flags & 0x02 != 0,
            fin: flags & 0x04 != 0,
            rst: flags & 0x08 != 0,
            truncated: false,
        };
        pushed += payload.len() as u64;
        let delivery = reassembler.push(&segment);
        delivered += delivery.bytes.len() as u64;
        for note in &delivery.notes {
            let _ = note.to_string();
        }
    }

    assert!(
        delivered <= pushed,
        "the reassembler delivered {delivered} bytes and was given {pushed}"
    );
    for (_, stream) in reassembler.streams() {
        assert!(
            stream.pending_bytes() <= MAX_PENDING_BYTES,
            "a hole held {} bytes, past the bound",
            stream.pending_bytes()
        );
    }
});
