// SPDX-License-Identifier: Apache-2.0
//! The serial frame decoder, and the CRC underneath it.
//!
//! The CRC's residue property — computing it over a frame that already carries
//! its CRC yields zero — is asserted here over arbitrary input, which is a far
//! stronger statement than any table of fixed vectors. Only one numeric CRC
//! vector is published in any Modbus Organization document, so a property over
//! generated input is most of the confidence available.

#![no_main]

use libfuzzer_sys::fuzz_target;
use salman_modbus::crc::Crc16;
use salman_modbus::rtu::RtuAdu;

fuzz_target!(|data: &[u8]| {
    if data.len() > 1024 {
        return;
    }

    // Whatever these bytes are, appending their CRC makes a frame that
    // verifies, and the residue over the result is zero.
    let mut framed = data.to_vec();
    framed.extend_from_slice(&Crc16::of(data).to_wire());
    assert!(
        Crc16::residue_ok(&framed),
        "a frame carrying its own CRC failed to verify"
    );
    assert_eq!(Crc16::of(&framed).0, 0, "the residue is not zero");

    // Decoding must never panic, and anything that decodes must re-encode to
    // exactly the bytes it came from — CRC included, low byte first.
    if let Ok(adu) = RtuAdu::decode(data) {
        assert_eq!(
            adu.to_vec(),
            data,
            "a decoded serial frame re-encoded differently: {data:02X?}"
        );
    }

    // Corrupting any single byte of a valid frame must be caught. This is the
    // check the whole CRC exists for, and it is cheap to assert here.
    if let Ok(adu) = RtuAdu::decode(&framed) {
        let _ = adu;
        for index in 0..framed.len() {
            let mut corrupted = framed.clone();
            corrupted[index] ^= 0x01;
            assert!(
                RtuAdu::decode(&corrupted).is_err(),
                "flipping a bit in byte {index} went undetected"
            );
        }
    }
});
