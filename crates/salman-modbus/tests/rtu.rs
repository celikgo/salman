// SPDX-License-Identifier: Apache-2.0
//! Serial application data units.
//!
//! The tests that matter most here are the ones about what a server must
//! **not** do: a corrupted frame is answered with silence, never with an
//! exception. Answering an exception to a bad CRC tells the client that a
//! device at that address received something, which is exactly what a
//! corrupted frame does not establish — and it is the most common defect in a
//! hand-written Modbus server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_modbus::crc::Crc16;
use salman_modbus::limits::MAX_PDU;
use salman_modbus::pdu::{Pdu, Request};
use salman_modbus::rtu::{AddressKind, BROADCAST, RtuAdu, RtuError, Timing, classify_address};

fn read_holding() -> Request {
    Request::ReadHoldingRegisters {
        start: 0x006B,
        quantity: 0x0003,
    }
}

#[test]
fn an_adu_is_the_address_the_pdu_and_the_crc_low_byte_first() {
    let adu = RtuAdu::new(0x11, read_holding().encode().unwrap());
    let bytes = adu.to_vec();
    assert_eq!(bytes[..6], [0x11, 0x03, 0x00, 0x6B, 0x00, 0x03]);
    // salman's own arithmetic over those six bytes, low byte first.
    assert_eq!(Crc16::of(&bytes[..6]), Crc16(0x8776));
    assert_eq!(bytes[6..], [0x76, 0x87]);
    assert_eq!(bytes.len(), 8);
}

#[test]
fn an_adu_survives_the_round_trip() {
    for address in [BROADCAST, 1, 0x11, 247] {
        let adu = RtuAdu::new(address, read_holding().encode().unwrap());
        assert_eq!(RtuAdu::decode(&adu.to_vec()).unwrap(), adu);
    }
}

#[test]
fn a_corrupted_frame_is_refused_rather_than_answered() {
    let adu = RtuAdu::new(0x11, read_holding().encode().unwrap());
    let mut bytes = adu.to_vec();
    bytes[3] ^= 0x01;
    let error = RtuAdu::decode(&bytes).unwrap_err();
    assert!(matches!(error, RtuError::CrcMismatch { .. }));
    assert!(
        error.requires_silence(),
        "SL §2.4.2: a bad frame is discarded in silence, not answered"
    );
}

#[test]
fn every_single_bit_error_in_a_frame_is_caught() {
    let adu = RtuAdu::new(0x11, read_holding().encode().unwrap());
    let bytes = adu.to_vec();
    for byte in 0..bytes.len() {
        for bit in 0..8 {
            let mut corrupted = bytes.clone();
            corrupted[byte] ^= 1 << bit;
            assert!(
                RtuAdu::decode(&corrupted).is_err(),
                "bit {bit} of byte {byte} flipped and the frame still decoded"
            );
        }
    }
}

#[test]
fn every_error_a_serial_frame_can_have_requires_silence() {
    // Not one of them may produce a response. A client that gets an answer to
    // a frame that was never intact learns something untrue about the bus.
    let cases = [
        RtuAdu::decode(&[]).unwrap_err(),
        RtuAdu::decode(&[0x11, 0x03, 0x00]).unwrap_err(),
        RtuAdu::decode(&vec![0x00; 300]).unwrap_err(),
    ];
    for error in cases {
        assert!(error.requires_silence(), "{error}");
    }
}

#[test]
fn a_frame_too_short_to_hold_a_crc_is_refused() {
    for length in 0..4_usize {
        let frame = vec![0x11_u8; length];
        assert!(matches!(
            RtuAdu::decode(&frame).unwrap_err(),
            RtuError::TooShort { .. }
        ));
    }
}

#[test]
fn a_frame_longer_than_a_serial_frame_may_be_is_refused() {
    let frame = vec![0x00_u8; 257];
    assert!(matches!(
        RtuAdu::decode(&frame).unwrap_err(),
        RtuError::TooLong { length: 257 }
    ));
}

#[test]
fn the_largest_legal_frame_fits() {
    let adu = RtuAdu::new(0x01, Pdu::from_bytes(&[0x03; MAX_PDU]).unwrap());
    let bytes = adu.to_vec();
    assert_eq!(bytes.len(), 256);
    assert_eq!(RtuAdu::decode(&bytes).unwrap(), adu);
}

#[test]
fn addresses_are_classified_as_the_specification_gives_them() {
    assert_eq!(classify_address(0), AddressKind::Broadcast);
    assert_eq!(classify_address(1), AddressKind::Device);
    assert_eq!(classify_address(247), AddressKind::Device);
    assert_eq!(classify_address(248), AddressKind::Reserved);
    assert_eq!(classify_address(255), AddressKind::Reserved);
    assert!(RtuAdu::new(BROADCAST, read_holding().encode().unwrap()).is_broadcast());
    assert!(!RtuAdu::new(1, read_holding().encode().unwrap()).is_broadcast());
}

#[test]
fn the_character_times_at_the_baud_rates_people_actually_use() {
    // 11 bits per character is SL §2.5.1; multiplying it out is salman's
    // arithmetic and is labelled as such in the module documentation.
    // 38.5 bits at 9600 baud is 4.010416 ms, and 16.5 bits is 1.71875 ms.
    let at_9600 = Timing::for_baud(9600).unwrap();
    assert_eq!(at_9600.inter_frame_ns, 4_010_416);
    assert_eq!(at_9600.inter_character_ns, 1_718_750);
    assert!(!at_9600.is_recommended_rather_than_required());

    let at_19200 = Timing::for_baud(19_200).unwrap();
    assert_eq!(at_19200.inter_frame_ns, 2_005_208);
    assert_eq!(at_19200.inter_character_ns, 859_375);
    assert!(!at_19200.is_recommended_rather_than_required());
}

#[test]
fn above_19200_baud_the_fixed_times_are_used_and_marked_as_a_recommendation() {
    // SL states these as a recommendation rather than a requirement, and the
    // distinction has to survive into the API or a reader will take salman's
    // behaviour for the standard's.
    let fast = Timing::for_baud(115_200).unwrap();
    assert_eq!(fast.inter_frame_ns, 1_750_000);
    assert_eq!(fast.inter_character_ns, 750_000);
    assert!(fast.is_recommended_rather_than_required());

    // The switch happens above 19200, not at it.
    assert!(
        !Timing::for_baud(19_200)
            .unwrap()
            .is_recommended_rather_than_required()
    );
    assert!(
        Timing::for_baud(19_201)
            .unwrap()
            .is_recommended_rather_than_required()
    );
}

#[test]
fn the_multiplication_comes_before_the_division() {
    // Dividing 1e9 by the baud rate first truncates the bit time, and 38.5
    // then scales that error up. At 9600 baud the wrong order gives
    // 4_010_391 ns against the correct 4_010_416 — 25 ns, which sounds like
    // nothing and is the difference between a derived constant that is right
    // and one that is merely close.
    let bit_ns_first = (1_000_000_000_u64 / 9600) * 385 / 10;
    assert_ne!(Timing::for_baud(9600).unwrap().inter_frame_ns, bit_ns_first);
}

#[test]
fn a_baud_rate_of_zero_has_no_character_time() {
    assert_eq!(Timing::for_baud(0), None);
}

#[test]
fn no_byte_string_makes_the_serial_decoder_panic() {
    let mut seed = 0x0DDB_1A5E_5BAD_5EED_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for _ in 0..20_000 {
        let length = (next() % 300) as usize;
        let bytes: Vec<u8> = (0..length).map(|_| (next() >> 33) as u8).collect();
        let _ = RtuAdu::decode(&bytes);
    }
}

#[test]
fn a_frame_that_decodes_re_encodes_to_the_bytes_it_came_from() {
    let adu = RtuAdu::new(0x11, read_holding().encode().unwrap());
    let bytes = adu.to_vec();
    assert_eq!(RtuAdu::decode(&bytes).unwrap().to_vec(), bytes);
}
