// SPDX-License-Identifier: Apache-2.0
//! What a Modbus protocol data unit means on the wire.
//!
//! The decoder here reads bytes that arrived from a network salman does not
//! control, so these tests are as interested in what it refuses as in what it
//! accepts. Two of them — `every_prefix_of_a_valid_frame_is_refused` and
//! `no_byte_string_makes_the_decoder_panic` — are properties over generated
//! input rather than fixed cases, because a decoder is only as good as its
//! behaviour on the frame nobody thought of.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_modbus::function::{ExceptionCode, FunctionCode};
use salman_modbus::limits::{MAX_READ_BITS, MAX_READ_REGISTERS, MAX_WRITE_BITS};
use salman_modbus::pdu::{Bits, DecodeError, Request, Response, Words};

/// Every request salman implements, one of each, for the round-trip tests.
fn one_of_each_request() -> Vec<Request> {
    vec![
        Request::ReadCoils {
            start: 0x0013,
            quantity: 0x0013,
        },
        Request::ReadDiscreteInputs {
            start: 0x00C4,
            quantity: 0x0016,
        },
        Request::ReadHoldingRegisters {
            start: 0x006B,
            quantity: 0x0003,
        },
        Request::ReadInputRegisters {
            start: 0x0008,
            quantity: 0x0001,
        },
        Request::WriteSingleCoil {
            address: 0x00AC,
            on: true,
        },
        Request::WriteSingleCoil {
            address: 0x00AC,
            on: false,
        },
        Request::WriteSingleRegister {
            address: 0x0001,
            value: 0x0003,
        },
        Request::WriteMultipleCoils {
            start: 0x0013,
            values: Bits::from_iter_of([true, false, true, true, false, false, true, true, false])
                .unwrap(),
        },
        Request::WriteMultipleRegisters {
            start: 0x0001,
            values: Words::new(&[0x000A, 0x0102]).unwrap(),
        },
    ]
}

/// A response to each of the above, in the same order.
fn one_of_each_response() -> Vec<(Request, Response)> {
    let requests = one_of_each_request();
    let responses = vec![
        Response::ReadCoils(Bits::zeroed(0x0013).unwrap()),
        Response::ReadDiscreteInputs(Bits::zeroed(0x0016).unwrap()),
        Response::ReadHoldingRegisters(Words::new(&[0x022B, 0x0000, 0x0064]).unwrap()),
        Response::ReadInputRegisters(Words::new(&[0x000A]).unwrap()),
        Response::WriteSingleCoil {
            address: 0x00AC,
            on: true,
        },
        Response::WriteSingleCoil {
            address: 0x00AC,
            on: false,
        },
        Response::WriteSingleRegister {
            address: 0x0001,
            value: 0x0003,
        },
        Response::WriteMultipleCoils {
            start: 0x0013,
            quantity: 9,
        },
        Response::WriteMultipleRegisters {
            start: 0x0001,
            quantity: 2,
        },
    ];
    requests.into_iter().zip(responses).collect()
}

// -- the one published frame ---------------------------------------------

#[test]
fn the_conformance_specifications_read_coils_frame() {
    // Conformance Test Specification for Modbus TCP V3.0 §9 publishes a
    // complete request and response. Its PDU halves are these five and three
    // bytes; the MBAP header around them is tested in the framing tests.
    //
    // This is the only complete frame published by any modbus.org document
    // that salman could fetch, which makes it the one test here that checks
    // salman against something other than itself.
    let request = Request::ReadCoils {
        start: 0,
        quantity: 1,
    };
    assert_eq!(request.encode().as_bytes(), [0x01, 0x00, 0x00, 0x00, 0x01]);

    let response = Response::decode(&[0x01, 0x01, 0x01], &request).unwrap();
    let Response::ReadCoils(bits) = response else {
        panic!("expected coils, got {response:?}")
    };
    assert_eq!(bits.count(), 1);
    assert_eq!(bits.get(0), Some(true));
}

// -- round trips ---------------------------------------------------------

#[test]
fn every_request_survives_being_written_and_read_back() {
    for request in one_of_each_request() {
        let encoded = request.encode();
        let decoded = Request::decode(encoded.as_bytes())
            .unwrap_or_else(|e| panic!("{request:?} encoded to {encoded:?} and failed: {e}"));
        assert_eq!(decoded, request);
    }
}

#[test]
fn every_response_survives_being_written_and_read_back() {
    for (request, response) in one_of_each_response() {
        let encoded = response.encode();
        let decoded = Response::decode(encoded.as_bytes(), &request)
            .unwrap_or_else(|e| panic!("{response:?} encoded to {encoded:?} and failed: {e}"));
        assert_eq!(decoded, response);
    }
}

#[test]
fn an_exception_response_survives_the_round_trip_for_every_function() {
    for (request, _) in one_of_each_response() {
        let response = Response::Exception {
            function: request.function(),
            code: ExceptionCode::ILLEGAL_DATA_ADDRESS,
        };
        let encoded = response.encode();
        assert_eq!(encoded.len(), 2, "an exception PDU is two bytes");
        assert_eq!(encoded.function(), request.function().as_exception());
        assert_eq!(
            Response::decode(encoded.as_bytes(), &request).unwrap(),
            response
        );
    }
}

// -- the wire format itself ----------------------------------------------

#[test]
fn multi_byte_fields_are_big_endian() {
    let request = Request::ReadHoldingRegisters {
        start: 0x1234,
        quantity: 0x0056,
    };
    assert_eq!(
        request.encode().as_bytes(),
        [0x03, 0x12, 0x34, 0x00, 0x56],
        "APS §4.2: the high byte first"
    );
}

#[test]
fn bits_are_packed_with_the_first_address_in_the_least_significant_bit() {
    // APS §6.1. Getting this backwards is a defect that survives a round trip
    // against your own implementation and fails against every real device,
    // which is exactly why it is asserted against literal bytes here.
    let bits = Bits::from_iter_of([true, false, false, false, false, false, false, false]).unwrap();
    assert_eq!(bits.packed(), [0x01]);

    let bits = Bits::from_iter_of([false, false, false, false, false, false, false, true]).unwrap();
    assert_eq!(bits.packed(), [0x80]);

    // Nine bits: the ninth goes into the low bit of a second byte.
    let mut nine = [false; 9];
    nine[8] = true;
    let bits = Bits::from_iter_of(nine).unwrap();
    assert_eq!(bits.packed(), [0x00, 0x01]);
}

#[test]
fn the_unused_high_bits_of_the_last_byte_are_zero() {
    let bits = Bits::from_iter_of([true; 3]).unwrap();
    assert_eq!(
        bits.packed(),
        [0x07],
        "bits 3 to 7 are padding and are zero"
    );
}

#[test]
fn padding_bits_a_sender_left_set_do_not_change_the_reading() {
    // Not every device zeroes its padding. Two readings of the same three
    // coils must still compare equal, or a golden test would fail against a
    // device that is not actually wrong about anything.
    let clean = Bits::from_packed(&[0x07], 3).unwrap();
    let dirty = Bits::from_packed(&[0xF7], 3).unwrap();
    assert_eq!(clean, dirty);
    assert_eq!(dirty.packed(), [0x07]);
}

#[test]
fn write_single_coil_uses_the_two_values_the_specification_gives() {
    let on = Request::WriteSingleCoil {
        address: 0x00AC,
        on: true,
    };
    assert_eq!(on.encode().as_bytes(), [0x05, 0x00, 0xAC, 0xFF, 0x00]);
    let off = Request::WriteSingleCoil {
        address: 0x00AC,
        on: false,
    };
    assert_eq!(off.encode().as_bytes(), [0x05, 0x00, 0xAC, 0x00, 0x00]);
}

#[test]
fn a_coil_value_that_is_neither_on_nor_off_is_refused() {
    // APS §6.5 gives this field exactly two legal values. A device that treats
    // any non-zero value as "on" is guessing, and the guess is not portable.
    let error = Request::decode(&[0x05, 0x00, 0xAC, 0x00, 0x01]).unwrap_err();
    assert_eq!(error, DecodeError::CoilValueNotOnOrOff { value: 0x0001 });
    assert!(error.to_string().contains("0xFF00"));
}

// -- refusals ------------------------------------------------------------

#[test]
fn an_empty_frame_carries_no_function_code() {
    assert_eq!(Request::decode(&[]).unwrap_err(), DecodeError::Empty);
}

#[test]
fn a_frame_longer_than_a_protocol_data_unit_is_refused_before_it_is_read() {
    let too_long = vec![0x03_u8; 254];
    assert_eq!(
        Request::decode(&too_long).unwrap_err(),
        DecodeError::TooLong { length: 254 }
    );
}

#[test]
fn a_quantity_of_zero_is_refused() {
    // APS gives every read a minimum of one. Zero is not a degenerate success.
    assert_eq!(
        Request::decode(&[0x03, 0x00, 0x00, 0x00, 0x00]).unwrap_err(),
        DecodeError::QuantityOutOfRange {
            quantity: 0,
            min: 1,
            max: MAX_READ_REGISTERS,
        }
    );
}

#[test]
fn a_quantity_above_the_limit_is_refused_at_the_limit_and_not_one_past_it() {
    // The boundary in both directions, because an off-by-one here is a frame
    // that a real device answers with exception 03.
    let at_limit = Request::ReadHoldingRegisters {
        start: 0,
        quantity: MAX_READ_REGISTERS,
    };
    assert!(Request::decode(at_limit.encode().as_bytes()).is_ok());

    let over = [
        0x03,
        0x00,
        0x00,
        ((MAX_READ_REGISTERS + 1) >> 8) as u8,
        ((MAX_READ_REGISTERS + 1) & 0xFF) as u8,
    ];
    assert!(matches!(
        Request::decode(&over).unwrap_err(),
        DecodeError::QuantityOutOfRange { .. }
    ));

    let bits_at_limit = Request::ReadCoils {
        start: 0,
        quantity: MAX_READ_BITS,
    };
    assert!(Request::decode(bits_at_limit.encode().as_bytes()).is_ok());
}

#[test]
fn a_byte_count_that_disagrees_with_the_quantity_is_refused() {
    // Write two registers, declare five data bytes. Some implementations trust
    // the byte count and some trust the quantity; salman trusts neither and
    // says they disagree.
    let error = Request::decode(&[0x10, 0x00, 0x01, 0x00, 0x02, 0x05, 0, 0, 0, 0, 0]).unwrap_err();
    assert_eq!(
        error,
        DecodeError::ByteCountDisagreesWithQuantity {
            declared: 5,
            expected: 4,
            quantity: 2,
        }
    );
}

#[test]
fn trailing_bytes_after_a_complete_frame_are_refused() {
    // A frame with something after it is not a frame salman read correctly.
    // Accepting it would hide a framing fault — two ADUs run together, most
    // often — which is precisely what the stream framer exists to catch.
    let mut bytes = Request::ReadHoldingRegisters {
        start: 0,
        quantity: 1,
    }
    .encode()
    .as_bytes()
    .to_vec();
    bytes.push(0xFF);
    assert_eq!(
        Request::decode(&bytes).unwrap_err(),
        DecodeError::TrailingBytes { extra: 1 }
    );
}

#[test]
fn a_function_code_the_specification_names_and_salman_does_not_implement_says_so() {
    // Read FIFO Queue. salman does not implement it, and the message says that
    // rather than pretending the code is meaningless.
    let error = Request::decode(&[0x18, 0x00, 0x00]).unwrap_err();
    assert_eq!(
        error,
        DecodeError::FunctionNotImplemented {
            function: FunctionCode::READ_FIFO_QUEUE
        }
    );
    assert!(error.to_string().contains("Read FIFO Queue"));
}

#[test]
fn a_function_code_the_specification_does_not_name_is_a_different_message() {
    let error = Request::decode(&[0x63, 0x00]).unwrap_err();
    assert_eq!(
        error,
        DecodeError::FunctionUnknown {
            function: FunctionCode(0x63)
        }
    );
    assert!(error.to_string().contains("0x63"));
}

#[test]
fn function_code_zero_is_not_a_request() {
    assert!(matches!(
        Request::decode(&[0x00, 0x00, 0x00]).unwrap_err(),
        DecodeError::FunctionUnknown { .. }
    ));
}

#[test]
fn a_response_that_answers_a_different_request_is_refused() {
    // The transaction identifier matches, the connection is right, and the
    // answer is to a different question. On a pipelined connection this is a
    // real and confusing fault, so it gets its own error rather than being
    // reported as a malformed frame.
    let request = Request::ReadHoldingRegisters {
        start: 0,
        quantity: 1,
    };
    let error = Response::decode(&[0x04, 0x02, 0x00, 0x0A], &request).unwrap_err();
    assert_eq!(
        error,
        DecodeError::FunctionDoesNotAnswerRequest {
            expected: FunctionCode::READ_HOLDING_REGISTERS,
            found: FunctionCode::READ_INPUT_REGISTERS,
        }
    );
}

#[test]
fn a_write_beyond_the_write_limit_is_refused_even_though_a_read_would_allow_it() {
    // 2000 coils may be read and only 1968 written. The two limits differ, and
    // a decoder that used one limit for both would accept a frame no device
    // accepts.
    let quantity = MAX_WRITE_BITS + 8;
    let mut bytes = vec![0x0F, 0x00, 0x00];
    bytes.extend_from_slice(&quantity.to_be_bytes());
    bytes.push(0);
    assert!(matches!(
        Request::decode(&bytes).unwrap_err(),
        DecodeError::QuantityOutOfRange { max, .. } if max == MAX_WRITE_BITS
    ));
}

// -- properties ----------------------------------------------------------

#[test]
fn every_prefix_of_a_valid_frame_is_refused() {
    // A frame that arrives one byte at a time must never look complete early.
    // This is the property behind the stream framer: if a short read could
    // decode, the framer would deliver half a frame and resynchronise onto
    // nothing.
    for request in one_of_each_request() {
        let encoded = request.encode();
        let bytes = encoded.as_bytes();
        for cut in 0..bytes.len() {
            let prefix = &bytes[..cut];
            assert!(
                Request::decode(prefix).is_err(),
                "{request:?} decoded from a {cut}-byte prefix of {bytes:02X?}"
            );
        }
    }
}

#[test]
fn no_byte_string_makes_the_decoder_panic() {
    // Rule 7: input from a network is hostile until proved otherwise. The
    // fuzz target covers this properly; this is the deterministic floor, so a
    // regression fails on every `cargo test` rather than only under nightly.
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move || {
        // xorshift64*, so the sweep is identical on every machine and every
        // run. A random test that cannot be replayed is not a test.
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let reference = Request::ReadHoldingRegisters {
        start: 0,
        quantity: 1,
    };
    for _ in 0..20_000 {
        let length = (next() % 264) as usize;
        let bytes: Vec<u8> = (0..length).map(|_| (next() >> 33) as u8).collect();
        let _ = Request::decode(&bytes);
        let _ = Response::decode(&bytes, &reference);
    }

    // And every function code at every short length, which random bytes reach
    // only by chance.
    for function in 0..=u8::MAX {
        for length in 0..8_usize {
            let mut bytes = vec![function];
            bytes.extend(std::iter::repeat_n(0xFF_u8, length));
            let _ = Request::decode(&bytes);
            let _ = Response::decode(&bytes, &reference);
        }
    }
}

#[test]
fn a_decoded_frame_re_encodes_to_the_bytes_it_came_from() {
    // Stronger than a round trip through the typed form: it says the decoder
    // did not quietly drop a field it failed to model.
    for request in one_of_each_request() {
        let bytes = request.encode();
        let decoded = Request::decode(bytes.as_bytes()).unwrap();
        assert_eq!(decoded.encode(), bytes);
    }
    for (request, response) in one_of_each_response() {
        let bytes = response.encode();
        let decoded = Response::decode(bytes.as_bytes(), &request).unwrap();
        assert_eq!(decoded.encode(), bytes);
    }
}

#[test]
fn a_read_response_cannot_be_decoded_without_the_request_that_asked_for_it() {
    // The fact that shapes the API. Five coils and eight coils produce
    // byte-identical responses; only the request distinguishes them. A tool
    // that decoded a capture without pairing would report one of them wrongly
    // and never know.
    let five = Request::ReadCoils {
        start: 0,
        quantity: 5,
    };
    let eight = Request::ReadCoils {
        start: 0,
        quantity: 8,
    };
    let on_the_wire = [0x01_u8, 0x01, 0xFF];

    let as_five = Response::decode(&on_the_wire, &five).unwrap();
    let as_eight = Response::decode(&on_the_wire, &eight).unwrap();
    assert_ne!(as_five, as_eight);

    let Response::ReadCoils(five_bits) = as_five else {
        panic!()
    };
    let Response::ReadCoils(eight_bits) = as_eight else {
        panic!()
    };
    assert_eq!(five_bits.count(), 5);
    assert_eq!(eight_bits.count(), 8);
    // The same three bits are set in both, and the reading differs in what it
    // claims about coils 5, 6 and 7.
    assert_eq!(five_bits.get(5), None);
    assert_eq!(eight_bits.get(5), Some(true));
}

#[test]
fn the_largest_legal_frame_of_each_kind_fits() {
    // The limits are only useful if a frame at the limit actually encodes. An
    // off-by-one in the buffer size would truncate the largest legal read,
    // which is the frame a device sends when a client is being efficient.
    let coils = Request::ReadCoils {
        start: 0,
        quantity: MAX_READ_BITS,
    };
    let bits = Bits::zeroed(MAX_READ_BITS).unwrap();
    let response = Response::ReadCoils(bits).encode();
    assert_eq!(response.len(), 2 + 250);
    assert_eq!(
        Response::decode(response.as_bytes(), &coils).unwrap(),
        Response::ReadCoils(bits)
    );

    let registers = Request::ReadHoldingRegisters {
        start: 0,
        quantity: MAX_READ_REGISTERS,
    };
    let words = Words::new(&[0xABCD; MAX_READ_REGISTERS as usize]).unwrap();
    let response = Response::ReadHoldingRegisters(words).encode();
    assert_eq!(response.len(), 2 + 250);
    assert_eq!(
        Response::decode(response.as_bytes(), &registers).unwrap(),
        Response::ReadHoldingRegisters(words)
    );
}
