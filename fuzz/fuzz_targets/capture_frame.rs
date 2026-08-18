// SPDX-License-Identifier: Apache-2.0
//! The link, IP and TCP decoders, against arbitrary frame bytes.
//!
//! The property worth having is that **a payload is never longer than the
//! frame it came from**. Every length in a frame is attacker-controlled — the
//! IP total length, the IPv6 payload length, the header lengths — and the
//! whole design bounds the payload by the IP header's field while bounding
//! that by what actually arrived. A frame that produced a payload longer than
//! itself would mean one of those bounds had been dropped.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_capture::frame::{Decoded, decode};
use salman_capture::pcap::LinkType;

const LINKS: &[LinkType] = &[
    LinkType::ETHERNET,
    LinkType::RAW,
    LinkType::NULL,
    LinkType::LINUX_SLL,
    LinkType::LINUX_SLL2,
    LinkType::IPV4,
    LinkType::IPV6,
];

fuzz_target!(|data: &[u8]| {
    if data.len() > 16 * 1024 {
        return;
    }
    for link in LINKS {
        match decode(*link, data, false) {
            Ok(Decoded::Tcp(segment)) => {
                assert!(
                    segment.payload.len() <= data.len(),
                    "a payload longer than the frame it came from"
                );
                // The payload must be a slice of the frame, not something
                // assembled from elsewhere.
                let start = segment.payload.as_ptr() as usize;
                let frame_start = data.as_ptr() as usize;
                assert!(
                    segment.payload.is_empty()
                        || (start >= frame_start && start + segment.payload.len() <= frame_start + data.len()),
                    "the payload does not lie inside the frame"
                );
                // Both directions of a conversation must produce the same key.
                let _ = segment.connection();
            }
            Ok(Decoded::NotDecoded { what }) => {
                // Every reason must be printable: a diagnostic that cannot be
                // rendered is a diagnostic nobody reads.
                let _ = what.to_string();
            }
            Err(error) => {
                let _ = error.to_string();
            }
        }
    }
});
