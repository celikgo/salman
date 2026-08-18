// SPDX-License-Identifier: Apache-2.0
//! The pcap container, against a file salman did not write.
//!
//! A capture is the most likely hostile input salman handles: it arrives as a
//! file from somewhere else, it is often large, and every length in it is
//! attacker-controlled. The target asserts three properties beyond "it did not
//! crash":
//!
//! * a record's data never exceeds the file it came from — the guard against a
//!   length field being trusted;
//! * `truncated` means exactly `captured < original`, so a short frame is
//!   never confused with a malformed one;
//! * whatever salman writes, salman reads back identically.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_capture::pcap::{LinkType, Reader, Writer};

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 * 1024 {
        return;
    }

    if let Ok(mut reader) = Reader::new(data) {
        let scale = reader.scale();
        let mut previous_index = None;
        while let Ok(Some(record)) = reader.next_record() {
            assert!(
                record.data.len() <= data.len(),
                "a record longer than the file it came from"
            );
            assert_eq!(
                record.truncated,
                (record.data.len() as u32) < record.original_length,
                "truncation must mean exactly that the capture kept less than was sent"
            );
            // Indices are the reader's own counter and must not skip or repeat.
            if let Some(previous) = previous_index {
                assert_eq!(record.index, previous + 1);
            }
            previous_index = Some(record.index);
            let _ = record.nanos(scale);
        }
    }

    // Round trip: anything used as a frame body must survive being written and
    // read back. The frame here is `data` itself, which is arbitrary.
    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    let body = data.get(..data.len().min(2048)).unwrap_or(&[]);
    writer.write(1_700_000_000_000_000_000, body.len() as u32, body);
    let written = writer.finish();
    let mut reader = Reader::new(&written).expect("salman must read what salman wrote");
    let record = reader
        .next_record()
        .expect("a written record must read back")
        .expect("there is one record");
    assert_eq!(record.data, body);
    assert!(reader.next_record().expect("no more").is_none());
});
