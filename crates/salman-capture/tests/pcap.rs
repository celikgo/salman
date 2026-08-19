// SPDX-License-Identifier: Apache-2.0
//! Reading and writing the classic pcap container.
//!
//! Two of these tests exist because of documented traps rather than because of
//! anything salman got wrong:
//!
//! * `a_variant_with_a_longer_record_header_is_refused_by_name` — Kuznetzov's
//!   modified pcap has a longer per-record header, so a reader that treated an
//!   unknown magic as "probably fine" would misparse every record and produce
//!   output that looks entirely reasonable. Refusing by name is the only safe
//!   answer, and the message has to say why.
//! * `a_record_whose_original_length_is_smaller_than_its_captured_length_is_read`
//!   — the specification permits it, and a reader that bounded its reads on
//!   the original length would drop data on such a file.
//!
//! # Checked against files salman did not write
//!
//! Every test here builds its own bytes, which proves salman agrees with
//! itself and nothing more. The reader was additionally run against two real
//! captures found on a development machine — `ntp.pcap` and
//! `modified-format.pcap` from the `pcap-parser` crate's assets, and Homebrew's
//! `test.pcap` — and cross-checked against `tcpdump -r`. It agreed on the
//! record count (12 in each) and on the first timestamp to the microsecond
//! (1476535656.489094 and 1412245746.168497), and refused
//! `modified-format.pcap` by name as byte-swapped Kuznetzov, which is exactly
//! what that file exists to be. Those files are not committed here: their
//! provenance and licensing are not salman's to redistribute.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_capture::pcap::{ByteOrder, CaptureError, LinkType, Reader, TimestampScale, Writer};

/// Builds a pcap file by hand, so the tests own every byte.
fn file(
    magic: [u8; 4],
    big_endian: bool,
    link: u32,
    records: &[(u32, u32, u32, &[u8])],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&magic);
    let put16 = |bytes: &mut Vec<u8>, v: u16| {
        if big_endian {
            bytes.extend_from_slice(&v.to_be_bytes());
        } else {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    };
    let put32 = |bytes: &mut Vec<u8>, v: u32| {
        if big_endian {
            bytes.extend_from_slice(&v.to_be_bytes());
        } else {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    };
    put16(&mut bytes, 2);
    put16(&mut bytes, 4);
    put32(&mut bytes, 0);
    put32(&mut bytes, 0);
    put32(&mut bytes, 262_144);
    put32(&mut bytes, link);
    for (seconds, fraction, original, data) in records {
        put32(&mut bytes, *seconds);
        put32(&mut bytes, *fraction);
        put32(&mut bytes, data.len() as u32);
        put32(&mut bytes, *original);
        bytes.extend_from_slice(data);
    }
    bytes
}

const FRAME: &[u8] = &[0xAA, 0xBB, 0xCC, 0xDD];

// -- the four magics -----------------------------------------------------

#[test]
fn all_four_magics_are_read_and_each_says_what_it_means() {
    // The single most common mistake in a hand-written pcap reader is getting
    // these backwards, and the IETF draft's own §4.1 has them inverted. A file
    // beginning D4 C3 B2 A1 is LITTLE-endian.
    let cases = [
        (
            [0xA1, 0xB2, 0xC3, 0xD4],
            true,
            ByteOrder::Big,
            TimestampScale::Microseconds,
        ),
        (
            [0xA1, 0xB2, 0x3C, 0x4D],
            true,
            ByteOrder::Big,
            TimestampScale::Nanoseconds,
        ),
        (
            [0xD4, 0xC3, 0xB2, 0xA1],
            false,
            ByteOrder::Little,
            TimestampScale::Microseconds,
        ),
        (
            [0x4D, 0x3C, 0xB2, 0xA1],
            false,
            ByteOrder::Little,
            TimestampScale::Nanoseconds,
        ),
    ];
    for (magic, big, order, scale) in cases {
        let bytes = file(magic, big, 1, &[(1, 2, 4, FRAME)]);
        let mut reader = Reader::new(&bytes).unwrap_or_else(|e| panic!("{magic:02X?}: {e}"));
        assert_eq!(reader.byte_order(), order, "{magic:02X?}");
        assert_eq!(reader.scale(), scale, "{magic:02X?}");
        assert_eq!(reader.link_type(), LinkType::ETHERNET);
        let records = reader.records().0;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].data, FRAME);
        assert_eq!(records[0].seconds, 1);
    }
}

#[test]
fn a_variant_with_a_longer_record_header_is_refused_by_name() {
    // Kuznetzov's modified pcap. Reading it as standard pcap does not fail —
    // it misparses every record and produces plausible nonsense, which is the
    // worst outcome available here.
    let bytes = file([0xA1, 0xB2, 0xCD, 0x34], true, 1, &[(1, 2, 4, FRAME)]);
    let error = Reader::new(&bytes).unwrap_err();
    let CaptureError::UnsupportedVariant { name, why, .. } = error else {
        panic!("expected a named refusal, got {error}")
    };
    assert!(name.contains("Kuznetzov"), "{name}");
    assert!(
        why.contains("longer"),
        "the reason must say why guessing is unsafe: {why}"
    );
}

#[test]
fn every_libpcap_variant_salman_will_not_read_is_refused_by_name() {
    for magic in [
        [0xA1, 0xB2, 0xCD, 0x34],
        [0x34, 0xCD, 0xB2, 0xA1],
        [0xA1, 0xB2, 0x34, 0xCD],
        [0xA1, 0x2B, 0x3C, 0x4D],
        [0xA1, 0xB2, 0xC3, 0xCB],
    ] {
        let bytes = file(magic, true, 1, &[]);
        assert!(
            matches!(
                Reader::new(&bytes),
                Err(CaptureError::UnsupportedVariant { .. })
            ),
            "{magic:02X?} was not refused by name"
        );
    }
}

#[test]
fn something_that_is_not_a_capture_says_what_the_magics_are() {
    let error = Reader::new(&[0u8; 64]).unwrap_err();
    let said = error.to_string();
    assert!(said.contains("A1B2C3D4"), "{said}");
}

#[test]
fn a_file_too_short_for_a_header_is_refused() {
    assert!(matches!(
        Reader::new(&[0xD4, 0xC3, 0xB2, 0xA1]),
        Err(CaptureError::TooShort { needed: 24, .. })
    ));
}

#[test]
fn a_snapshot_length_of_zero_is_refused() {
    // No frame in such a file could hold anything, so it is a file that says
    // it contains nothing while appearing to contain records.
    let mut bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 1, &[]);
    bytes[16..20].copy_from_slice(&0_u32.to_le_bytes());
    assert_eq!(
        Reader::new(&bytes).unwrap_err(),
        CaptureError::ZeroSnapshotLength
    );
}

// -- records -------------------------------------------------------------

#[test]
fn the_reserved_fields_are_ignored_rather_than_validated() {
    // The draft says writers SHOULD zero them and readers MUST ignore them.
    // Real files predate that, and validating would refuse captures every
    // other tool reads.
    let mut bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 1, &[(1, 2, 4, FRAME)]);
    bytes[8..16].copy_from_slice(&[0xFF; 8]);
    let mut reader = Reader::new(&bytes).expect("reserved bytes are not salman's business");
    assert_eq!(reader.records().0.len(), 1);
}

#[test]
fn only_the_low_sixteen_bits_of_the_link_word_are_the_link_type() {
    // The rest carries an FCS length and two flags. A reader that took the
    // whole word would see link type 0x00010000 and decode nothing.
    let bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 0x1234_0001, &[]);
    let reader = Reader::new(&bytes).unwrap();
    assert_eq!(reader.link_type(), LinkType::ETHERNET);
}

#[test]
fn a_truncated_record_is_marked_truncated_and_not_malformed() {
    // The sender sent a whole frame; the capturing tool kept part of it.
    // Reporting that as a malformed frame is a common and confusing mistake.
    let bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 1, &[(1, 0, 1514, FRAME)]);
    let mut reader = Reader::new(&bytes).unwrap();
    let records = reader.records().0;
    assert!(records[0].truncated);
    assert_eq!(records[0].data.len(), 4);
    assert_eq!(records[0].original_length, 1514);
}

#[test]
fn a_record_whose_original_length_is_smaller_than_its_captured_length_is_read() {
    // The specification permits it. A reader that bounded its reads on the
    // original length would silently drop the tail of such a record.
    let bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 1, &[(1, 0, 2, FRAME)]);
    let mut reader = Reader::new(&bytes).unwrap();
    let records = reader.records().0;
    assert_eq!(records[0].data, FRAME, "all four bytes must survive");
    assert!(!records[0].truncated);
}

#[test]
fn a_record_claiming_more_than_the_file_holds_is_refused_and_reserves_nothing() {
    // The hostile case: a four-gigabyte claim in a forty-byte file.
    let mut bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 1, &[(1, 0, 4, FRAME)]);
    let header = 24;
    bytes[header + 8..header + 12].copy_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    let mut reader = Reader::new(&bytes).unwrap();
    assert!(matches!(
        reader.next_record(),
        Err(CaptureError::RecordPastEndOfFile { .. })
    ));
}

#[test]
fn a_trailing_partial_record_header_is_refused() {
    let mut bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 1, &[(1, 0, 4, FRAME)]);
    bytes.extend_from_slice(&[0, 0, 0]);
    let mut reader = Reader::new(&bytes).unwrap();
    assert!(reader.next_record().unwrap().is_some());
    assert!(matches!(
        reader.next_record(),
        Err(CaptureError::ShortRecordHeader { .. })
    ));
}

#[test]
fn records_are_numbered_from_zero_in_file_order() {
    let bytes = file(
        [0xD4, 0xC3, 0xB2, 0xA1],
        false,
        1,
        &[(1, 0, 4, FRAME), (2, 0, 4, FRAME), (3, 0, 4, FRAME)],
    );
    let mut reader = Reader::new(&bytes).unwrap();
    let records = reader.records().0;
    assert_eq!(
        records.iter().map(|r| r.index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
}

#[test]
fn an_empty_capture_has_no_records_and_is_not_an_error() {
    let bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 1, &[]);
    let mut reader = Reader::new(&bytes).unwrap();
    assert!(reader.records().0.is_empty());
}

// -- timestamps ----------------------------------------------------------

#[test]
fn the_timestamp_scale_changes_what_the_fraction_means() {
    // Assuming microseconds when a file says nanoseconds does not throw an
    // error. It silently mis-times every frame by a thousand, which for a
    // timeline is the worst available failure.
    let micros = file(
        [0xD4, 0xC3, 0xB2, 0xA1],
        false,
        1,
        &[(10, 500_000, 4, FRAME)],
    );
    let nanos = file(
        [0x4D, 0x3C, 0xB2, 0xA1],
        false,
        1,
        &[(10, 500_000, 4, FRAME)],
    );

    let mut reader = Reader::new(&micros).unwrap();
    let scale = reader.scale();
    let record = reader.next_record().unwrap().unwrap();
    assert_eq!(
        record.nanos(scale),
        10_500_000_000,
        "half a second past ten"
    );

    let mut reader = Reader::new(&nanos).unwrap();
    let scale = reader.scale();
    let record = reader.next_record().unwrap().unwrap();
    assert_eq!(record.nanos(scale), 10_000_500_000, "half a millisecond");
}

// -- writing -------------------------------------------------------------

#[test]
fn what_salman_writes_salman_reads() {
    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    writer.write(1_700_000_000_123_456_000, 4, FRAME);
    writer.write(1_700_000_001_000_000_000, 1514, &[0x01, 0x02]);
    let bytes = writer.finish();

    let mut reader = Reader::new(&bytes).expect("salman writes a file salman reads");
    assert_eq!(reader.byte_order(), ByteOrder::Little);
    assert_eq!(reader.scale(), TimestampScale::Microseconds);
    assert_eq!(reader.link_type(), LinkType::ETHERNET);
    let scale = reader.scale();
    let records = reader.records().0;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].data, FRAME);
    assert_eq!(records[0].nanos(scale), 1_700_000_000_123_456_000);
    assert!(records[1].truncated);
    assert_eq!(records[1].original_length, 1514);
}

#[test]
fn writing_is_deterministic() {
    // Two captures of the same frames must be byte-identical, or a golden
    // capture in CI is worthless.
    let build = || {
        let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
        writer.write(1_700_000_000_000_000_000, 4, FRAME);
        writer.finish()
    };
    assert_eq!(build(), build());
}

#[test]
fn a_written_original_length_is_never_less_than_what_was_kept() {
    // A record claiming to be shorter than its own data is legal to read and
    // absurd to write.
    let mut writer = Writer::new(LinkType::ETHERNET, 262_144);
    writer.write(0, 1, FRAME);
    let bytes = writer.finish();
    let mut reader = Reader::new(&bytes).unwrap();
    let record = reader.next_record().unwrap().unwrap();
    assert_eq!(record.original_length, 4);
    assert!(!record.truncated);
}

// -- robustness ----------------------------------------------------------

#[test]
fn no_byte_string_makes_the_reader_panic() {
    let mut seed = 0x5DEE_CE66_D1E5_1EED_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    // Random bytes rarely produce a valid magic, so half the cases start from
    // a real file and corrupt it. Otherwise this would only ever test the
    // "not a pcap" path.
    let valid = file(
        [0xD4, 0xC3, 0xB2, 0xA1],
        false,
        1,
        &[(1, 0, 4, FRAME), (2, 0, 4, FRAME)],
    );
    for round in 0..20_000 {
        let bytes: Vec<u8> = if round % 2 == 0 {
            let length = (next() % 200) as usize;
            (0..length).map(|_| (next() >> 33) as u8).collect()
        } else {
            let mut corrupted = valid.clone();
            let hits = 1 + (next() % 4) as usize;
            for _ in 0..hits {
                let at = (next() as usize) % corrupted.len();
                corrupted[at] = (next() >> 33) as u8;
            }
            corrupted
        };
        if let Ok(mut reader) = Reader::new(&bytes) {
            let _ = reader.records();
        }
    }
}

#[test]
fn a_truncated_capture_gives_back_what_it_had_as_well_as_the_error() {
    // Found by review: `records` documented returning the records read before
    // an error and threw them away. Losing a whole capture over its final
    // frame is bad behaviour, not only a false doc comment — a file still
    // being written ends this way every time it is read.
    let mut bytes = file(
        [0xD4, 0xC3, 0xB2, 0xA1],
        false,
        1,
        &[(1, 0, 4, FRAME), (2, 0, 4, FRAME)],
    );
    // Cut the last record in half.
    bytes.truncate(bytes.len() - 2);

    let mut reader = Reader::new(&bytes).unwrap();
    let (records, error) = reader.records();
    assert_eq!(records.len(), 1, "the intact record must survive");
    assert!(
        matches!(error, Some(CaptureError::RecordPastEndOfFile { .. })),
        "{error:?}"
    );
}

#[test]
fn a_whole_capture_reports_no_error_at_all() {
    let bytes = file([0xD4, 0xC3, 0xB2, 0xA1], false, 1, &[(1, 0, 4, FRAME)]);
    let mut reader = Reader::new(&bytes).unwrap();
    let (records, error) = reader.records();
    assert_eq!(records.len(), 1);
    assert!(error.is_none());
}
