// SPDX-License-Identifier: Apache-2.0
//! Framing a Modbus TCP byte stream.
//!
//! TCP is a byte stream. A reader that calls `read()` once and decodes what it
//! got is wrong in four ways that all happen on real networks, and each has a
//! test here: a header split across segments, a body split across segments,
//! two frames delivered together, and one and a half frames delivered
//! together.
//!
//! The strongest test in the file is `any_split_of_a_stream_frames_identically`,
//! which asserts that the frames recovered do not depend on where the segment
//! boundaries fell. That is the property the whole design exists to provide,
//! and it is the one a fixed set of cases can only sample.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_modbus::limits::{MAX_PDU, MBAP_HEADER};
use salman_modbus::pdu::{Pdu, Request};
use salman_modbus::tcp::{FrameError, Framer, MAX_MBAP_LENGTH, MbapHeader, TcpAdu};

/// The conformance specification's Read Coils request, whole.
const READ_COILS: [u8; 12] = [
    0x00, 0x00, // transaction
    0x00, 0x00, // protocol
    0x00, 0x06, // length: unit + five PDU bytes
    0x01, // unit
    0x01, 0x00, 0x00, 0x00, 0x01, // read one coil at address zero
];

/// Feeds a whole stream through a framer and returns every frame it yields.
fn frames_of(stream: &[u8]) -> Result<Vec<TcpAdu>, FrameError> {
    frames_of_segments(&[stream])
}

/// Feeds a stream delivered as the given segments, as a real socket would.
fn frames_of_segments(segments: &[&[u8]]) -> Result<Vec<TcpAdu>, FrameError> {
    let mut framer = Framer::new();
    let mut frames = Vec::new();
    for segment in segments {
        let mut rest = *segment;
        loop {
            let (used, outcome) = framer.advance(rest);
            rest = rest.get(used..).unwrap_or(&[]);
            match outcome? {
                Some(frame) => frames.push(frame),
                // Nothing more can be made of this segment: either it is
                // exhausted or what is left is an incomplete frame.
                None => break,
            }
        }
    }
    Ok(frames)
}

#[test]
fn a_whole_frame_in_one_read() {
    let frames = frames_of(&READ_COILS).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].header.transaction, 0);
    assert_eq!(frames[0].header.unit, 1);
    assert_eq!(frames[0].header.length, 6);
    assert_eq!(
        Request::decode(frames[0].pdu.as_bytes()).unwrap(),
        Request::ReadCoils {
            start: 0,
            quantity: 1
        }
    );
}

#[test]
fn a_header_split_across_two_reads() {
    // Three bytes, then the rest. The length field is not even complete in the
    // first segment, so a reader that decoded what it had would read a length
    // out of two bytes it does not have.
    let frames = frames_of_segments(&[&READ_COILS[..3], &READ_COILS[3..]]).unwrap();
    assert_eq!(frames.len(), 1);
}

#[test]
fn a_body_split_across_two_reads() {
    // The header arrives whole and the PDU does not.
    let frames = frames_of_segments(&[&READ_COILS[..9], &READ_COILS[9..]]).unwrap();
    assert_eq!(frames.len(), 1);
}

#[test]
fn two_frames_in_one_read() {
    // The case a single-shot reader silently loses: it decodes the first and
    // throws the second away with the buffer.
    let mut stream = READ_COILS.to_vec();
    stream.extend_from_slice(&READ_COILS);
    let frames = frames_of(&stream).unwrap();
    assert_eq!(frames.len(), 2);
}

#[test]
fn one_and_a_half_frames_then_the_remainder() {
    // The nastiest of the four, because the leftover half has to survive until
    // the next read and be joined to it.
    let mut first = READ_COILS.to_vec();
    first.extend_from_slice(&READ_COILS[..5]);
    let frames = frames_of_segments(&[&first, &READ_COILS[5..]]).unwrap();
    assert_eq!(frames.len(), 2);
}

#[test]
fn a_stream_delivered_one_byte_at_a_time() {
    let segments: Vec<&[u8]> = READ_COILS.chunks(1).collect();
    let frames = frames_of_segments(&segments).unwrap();
    assert_eq!(frames.len(), 1);
}

#[test]
fn any_split_of_a_stream_frames_identically() {
    // The property the design exists for: what comes out must not depend on
    // where the network happened to cut. Every split point of a two-frame
    // stream is checked, which is 24 of them.
    let mut stream = READ_COILS.to_vec();
    stream.extend_from_slice(&READ_COILS);
    let whole = frames_of(&stream).unwrap();
    assert_eq!(whole.len(), 2);

    for cut in 0..=stream.len() {
        let (left, right) = stream.split_at(cut);
        let split = frames_of_segments(&[left, right]).unwrap();
        assert_eq!(split, whole, "a split at {cut} changed the frames");
    }
}

#[test]
fn every_three_way_split_frames_identically() {
    let whole = frames_of(&READ_COILS).unwrap();
    for first in 0..=READ_COILS.len() {
        for second in first..=READ_COILS.len() {
            let segments: [&[u8]; 3] = [
                &READ_COILS[..first],
                &READ_COILS[first..second],
                &READ_COILS[second..],
            ];
            assert_eq!(
                frames_of_segments(&segments).unwrap(),
                whole,
                "splitting at {first} and {second} changed the frames"
            );
        }
    }
}

// -- what the framer refuses ---------------------------------------------

#[test]
fn a_non_zero_protocol_identifier_is_fatal() {
    // MG §3.1.3: zero means Modbus. Anything else means these bytes are some
    // other protocol, and salman has no idea how long its frames are — so
    // there is no safe place to resume.
    let mut stream = READ_COILS;
    stream[2] = 0x00;
    stream[3] = 0x10;
    let error = frames_of(&stream).unwrap_err();
    assert_eq!(error, FrameError::ProtocolNotModbus { found: 0x0010 });
    assert!(error.is_fatal());
}

#[test]
fn a_length_field_larger_than_any_frame_is_fatal_and_reserves_nothing() {
    // The defect worth naming: a reader that sized a buffer from this field
    // would allocate 64 KiB for every frame a hostile peer sent. salman checks
    // the claim against what a Modbus frame may carry before it copies a byte.
    let mut stream = READ_COILS;
    stream[4] = 0xFF;
    stream[5] = 0xFF;
    let error = frames_of(&stream).unwrap_err();
    assert_eq!(
        error,
        FrameError::LengthOutOfRange {
            found: 0xFFFF,
            min: 2,
            max: MAX_MBAP_LENGTH,
        }
    );
    assert!(error.is_fatal());
}

#[test]
fn a_length_field_of_zero_or_one_is_fatal() {
    // A frame has to carry at least a unit identifier and a function code.
    for length in [0_u16, 1] {
        let mut stream = READ_COILS;
        stream[4..6].copy_from_slice(&length.to_be_bytes());
        assert!(matches!(
            frames_of(&stream).unwrap_err(),
            FrameError::LengthOutOfRange { found, .. } if found == length
        ));
    }
}

#[test]
fn the_largest_legal_length_is_accepted_and_one_more_is_not() {
    let pdu = Pdu::from_bytes(&[0x03; MAX_PDU]).unwrap();
    let adu = TcpAdu::new(1, 0xFF, pdu);
    assert_eq!(adu.header.length, MAX_MBAP_LENGTH);
    let frames = frames_of(&adu.to_vec()).unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].pdu.len(), MAX_PDU);

    let mut over = adu.to_vec();
    over[4..6].copy_from_slice(&(MAX_MBAP_LENGTH + 1).to_be_bytes());
    assert!(frames_of(&over).is_err());
}

#[test]
fn a_framer_that_lost_the_stream_stays_lost() {
    // A caller that ignores a fatal error must not be able to go on receiving
    // frames from a stream salman can no longer locate. The next frame in the
    // buffer might be real, and might be assembled from the middle of two
    // others, and nothing in the stream distinguishes the two.
    let mut framer = Framer::new();
    let mut broken = READ_COILS;
    broken[3] = 0x10;
    let (_, first) = framer.advance(&broken);
    let error = first.unwrap_err();
    assert!(framer.is_poisoned());

    // And it keeps saying so. Reporting a fault once and then going quiet is
    // worse than not reporting it: a caller that reads until it has a frame
    // would read for ever against a peer that keeps sending, with the read
    // timeout never firing because bytes keep arriving. That was a real
    // livelock in the client, found by review.
    for _ in 0..3 {
        let (used, outcome) = framer.advance(&READ_COILS);
        assert_eq!(used, 0, "a poisoned framer consumes nothing");
        assert_eq!(
            outcome.unwrap_err(),
            error,
            "a poisoned framer must keep reporting the error that lost the stream"
        );
    }
}

#[test]
fn a_poisoned_framer_cannot_be_read_past_by_a_peer_that_keeps_sending() {
    // The livelock, as a property. However many bytes arrive after framing is
    // lost, the framer consumes none of them and reports the fault every time,
    // so a caller's loop terminates on the error rather than spinning.
    let mut framer = Framer::new();
    let mut broken = READ_COILS;
    broken[3] = 0x10;
    let (_, outcome) = framer.advance(&broken);
    assert!(outcome.is_err());

    let filler = vec![0_u8; 4096];
    let mut consumed_after = 0;
    for _ in 0..100 {
        let (used, outcome) = framer.advance(&filler);
        consumed_after += used;
        assert!(outcome.is_err(), "the framer went quiet");
    }
    assert_eq!(consumed_after, 0);
}

#[test]
fn an_incomplete_frame_yields_nothing_and_holds_what_it_has() {
    let mut framer = Framer::new();
    let (used, outcome) = framer.advance(&READ_COILS[..8]);
    assert_eq!(used, 8);
    assert_eq!(outcome.unwrap(), None);
    assert_eq!(framer.buffered(), 8);
    assert!(!framer.is_poisoned());
}

#[test]
fn an_empty_read_is_not_an_error() {
    // A socket returning zero bytes is ordinary. It must not look like a
    // framing fault.
    let mut framer = Framer::new();
    let (used, outcome) = framer.advance(&[]);
    assert_eq!(used, 0);
    assert_eq!(outcome.unwrap(), None);
}

// -- the header itself ---------------------------------------------------

#[test]
fn the_header_round_trips_and_is_big_endian() {
    let header = MbapHeader {
        transaction: 0x1234,
        protocol: 0,
        length: 0x0006,
        unit: 0xFF,
    };
    assert_eq!(
        header.to_bytes(),
        [0x12, 0x34, 0x00, 0x00, 0x00, 0x06, 0xFF]
    );
    assert_eq!(MbapHeader::from_bytes(header.to_bytes()), header);
}

#[test]
fn the_length_field_counts_the_unit_identifier_as_well_as_the_protocol_data_unit() {
    // Off by one here is the single most common Modbus TCP defect, and it is
    // invisible against an implementation that makes the same mistake.
    let pdu = Request::ReadCoils {
        start: 0,
        quantity: 1,
    }
    .encode()
    .unwrap();
    assert_eq!(pdu.len(), 5);
    let adu = TcpAdu::new(0, 1, pdu);
    assert_eq!(adu.header.length, 6);
    assert_eq!(adu.to_vec().len(), MBAP_HEADER + 5);
    assert_eq!(adu.to_vec().len(), 6 + adu.header.length as usize);
}

#[test]
fn a_header_claiming_a_length_no_frame_could_have_reports_it_as_unusable() {
    assert_eq!(
        MbapHeader {
            transaction: 0,
            protocol: 0,
            length: 1,
            unit: 0
        }
        .claimed_pdu_len(),
        None
    );
    assert_eq!(
        MbapHeader {
            transaction: 0,
            protocol: 0,
            length: 6,
            unit: 0
        }
        .claimed_pdu_len(),
        Some(5)
    );
}

#[test]
fn no_byte_string_makes_the_framer_panic() {
    // Rule 7 again, for the layer that faces the socket directly.
    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for _ in 0..5_000 {
        let mut framer = Framer::new();
        let length = (next() % 600) as usize;
        let bytes: Vec<u8> = (0..length).map(|_| (next() >> 33) as u8).collect();
        let mut rest = &bytes[..];
        // Bounded so that a framer which consumed nothing and returned a frame
        // could not spin here for ever; the assertion is that it terminates.
        for _ in 0..1_000 {
            let (used, outcome) = framer.advance(rest);
            rest = rest.get(used..).unwrap_or(&[]);
            match outcome {
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
    }
}

#[test]
fn a_frame_is_never_delivered_without_consuming_its_bytes() {
    // If `advance` ever returned a frame while consuming nothing, the loop in
    // `frames_of_segments` — and in every caller — would never end.
    let mut stream = READ_COILS.to_vec();
    stream.extend_from_slice(&READ_COILS);
    let mut framer = Framer::new();
    let mut rest = &stream[..];
    let mut delivered = 0;
    while !rest.is_empty() {
        let (used, outcome) = framer.advance(rest);
        if let Ok(Some(_)) = outcome {
            delivered += 1;
            assert!(used > 0, "a frame arrived without consuming anything");
        }
        if used == 0 {
            break;
        }
        rest = rest.get(used..).unwrap_or(&[]);
    }
    assert_eq!(delivered, 2);
}
