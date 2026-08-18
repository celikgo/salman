// SPDX-License-Identifier: Apache-2.0
//! What a Modbus server answers, and when it refuses.
//!
//! These are the behaviours that are awkward to provoke on real equipment and
//! easy to get wrong in a simulator — an address one past the end of a map, a
//! multi-register write that fails halfway, a request that is wrong in two
//! ways at once — which is exactly why the decision layer has no transport
//! attached to it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use salman_modbus::device::{BitTable, Device, Table, WordTable};
use salman_modbus::function::ExceptionCode;
use salman_modbus::limits::{MAX_READ_REGISTERS, MAX_WRITE_REGISTERS};
use salman_modbus::pdu::{Bits, Request, Response, Words};

/// A device with a hundred of each thing, based at address zero.
fn device() -> Device {
    Device::empty()
        .with_bits(BitTable::Coils, 0, 100)
        .with_bits(BitTable::DiscreteInputs, 0, 100)
        .with_registers(WordTable::HoldingRegisters, 0, 100)
        .with_registers(WordTable::InputRegisters, 0, 100)
}

// -- reading -------------------------------------------------------------

#[test]
fn a_read_inside_the_map_answers_with_the_values() {
    let mut device = device();
    device.set_register(WordTable::HoldingRegisters, 5, 0x1234);
    let response = device
        .apply(&Request::ReadHoldingRegisters {
            start: 4,
            quantity: 3,
        })
        .unwrap();
    let Response::ReadHoldingRegisters(words) = response else {
        panic!("{response:?}")
    };
    assert_eq!(words.values(), [0x0000, 0x1234, 0x0000]);
}

#[test]
fn the_four_tables_are_independent() {
    // APS permits a device to overlay them and permits it not to. salman's
    // model keeps them separate, so writing a coil cannot change a discrete
    // input that happens to share an address.
    let mut device = device();
    device.set_bit(BitTable::DiscreteInputs, 7, true);
    assert_eq!(device.bit(BitTable::DiscreteInputs, 7), Some(true));
    assert_eq!(device.bit(BitTable::Coils, 7), Some(false));

    device.set_register(WordTable::InputRegisters, 7, 0xBEEF);
    assert_eq!(device.register(WordTable::InputRegisters, 7), Some(0xBEEF));
    assert_eq!(device.register(WordTable::HoldingRegisters, 7), Some(0));
}

// -- the boundary --------------------------------------------------------

#[test]
fn a_read_that_ends_exactly_at_the_end_of_the_map_is_legal() {
    // The canonical case from the conformance material: a map of 100, a read
    // of four from 96 succeeds and a read of five does not. The check is on
    // start plus count, which is what APS §7 says exception 02 means.
    let mut device = device();
    assert!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 96,
                quantity: 4
            })
            .is_ok()
    );
    assert_eq!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 96,
                quantity: 5
            })
            .unwrap_err(),
        ExceptionCode::ILLEGAL_DATA_ADDRESS
    );
}

#[test]
fn an_address_below_the_start_of_the_map_is_outside_it() {
    // A device whose map starts at 1000 does not answer for 999, and an
    // implementation that subtracted without checking would read backwards
    // off the front of its own array.
    let mut device = Device::empty().with_registers(WordTable::HoldingRegisters, 1000, 10);
    assert_eq!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 999,
                quantity: 1
            })
            .unwrap_err(),
        ExceptionCode::ILLEGAL_DATA_ADDRESS
    );
    assert!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 1000,
                quantity: 10
            })
            .is_ok()
    );
    assert_eq!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 1000,
                quantity: 11
            })
            .unwrap_err(),
        ExceptionCode::ILLEGAL_DATA_ADDRESS
    );
}

#[test]
fn a_map_that_would_run_past_the_top_of_the_address_space_is_clamped() {
    // A device declared with 100 registers from 0xFFF0 has sixteen, not a
    // wrapped map that answers for address zero.
    let mut device = Device::empty().with_registers(WordTable::HoldingRegisters, 0xFFF0, 100);
    assert!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 0xFFF0,
                quantity: 16
            })
            .is_ok()
    );
    assert_eq!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 0,
                quantity: 1
            })
            .unwrap_err(),
        ExceptionCode::ILLEGAL_DATA_ADDRESS
    );
}

#[test]
fn an_empty_device_refuses_everything_with_the_address_exception() {
    let mut device = Device::empty();
    for request in [
        Request::ReadCoils {
            start: 0,
            quantity: 1,
        },
        Request::ReadDiscreteInputs {
            start: 0,
            quantity: 1,
        },
        Request::ReadHoldingRegisters {
            start: 0,
            quantity: 1,
        },
        Request::ReadInputRegisters {
            start: 0,
            quantity: 1,
        },
        Request::WriteSingleCoil {
            address: 0,
            on: true,
        },
        Request::WriteSingleRegister {
            address: 0,
            value: 1,
        },
    ] {
        assert_eq!(
            device.apply(&request).unwrap_err(),
            ExceptionCode::ILLEGAL_DATA_ADDRESS,
            "{request:?}"
        );
    }
}

// -- the order the checks run in -----------------------------------------

#[test]
fn a_quantity_that_is_wrong_is_reported_before_an_address_that_is_also_wrong() {
    // This is the case that settles which order salman follows, and it is the
    // case APS answers two different ways: Figure 9 says 02 and every
    // per-function figure in §6 says 03. The request below is wrong in both
    // ways at once — a quantity of zero at an address outside the map — so
    // whichever code comes back names the order.
    //
    // salman follows the per-function order. The choice is salman's, because
    // the specification does not settle it, and CONFORMANCE §26 says so.
    let mut device = device();
    assert_eq!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 50_000,
                quantity: 0
            })
            .unwrap_err(),
        ExceptionCode::ILLEGAL_DATA_VALUE
    );
}

#[test]
fn a_quantity_of_zero_is_an_illegal_value_and_not_an_empty_success() {
    let mut device = device();
    assert_eq!(
        device
            .apply(&Request::ReadCoils {
                start: 0,
                quantity: 0
            })
            .unwrap_err(),
        ExceptionCode::ILLEGAL_DATA_VALUE
    );
}

#[test]
fn a_quantity_above_what_the_function_permits_is_an_illegal_value() {
    let mut device = Device::empty().with_registers(WordTable::HoldingRegisters, 0, 1000);
    assert!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 0,
                quantity: MAX_READ_REGISTERS
            })
            .is_ok()
    );
    assert_eq!(
        device
            .apply(&Request::ReadHoldingRegisters {
                start: 0,
                quantity: MAX_READ_REGISTERS + 1
            })
            .unwrap_err(),
        ExceptionCode::ILLEGAL_DATA_VALUE,
        "the quantity is checked against the function's limit, not against the map"
    );
}

// -- writing -------------------------------------------------------------

#[test]
fn a_single_write_takes_effect_and_is_echoed() {
    let mut device = device();
    let response = device
        .apply(&Request::WriteSingleRegister {
            address: 3,
            value: 0xABCD,
        })
        .unwrap();
    assert_eq!(
        response,
        Response::WriteSingleRegister {
            address: 3,
            value: 0xABCD
        }
    );
    assert_eq!(
        device.register(WordTable::HoldingRegisters, 3),
        Some(0xABCD)
    );
}

#[test]
fn a_multiple_write_takes_effect_across_its_whole_range() {
    let mut device = device();
    let response = device
        .apply(&Request::WriteMultipleRegisters {
            start: 10,
            values: Words::new(&[1, 2, 3]).unwrap(),
        })
        .unwrap();
    assert_eq!(
        response,
        Response::WriteMultipleRegisters {
            start: 10,
            quantity: 3
        }
    );
    assert_eq!(device.register(WordTable::HoldingRegisters, 10), Some(1));
    assert_eq!(device.register(WordTable::HoldingRegisters, 12), Some(3));
    assert_eq!(device.register(WordTable::HoldingRegisters, 13), Some(0));
}

#[test]
fn a_multiple_write_that_runs_off_the_end_changes_nothing_at_all() {
    // The one that matters. A write that applied what fitted and then failed
    // would leave the device in a state the client has no way to learn about:
    // it gets an exception, and half its values are in place. Every address is
    // checked before the first byte moves.
    let mut device = Device::empty().with_registers(WordTable::HoldingRegisters, 0, 10);
    device.set_register(WordTable::HoldingRegisters, 8, 0x1111);
    device.set_register(WordTable::HoldingRegisters, 9, 0x2222);

    let error = device
        .apply(&Request::WriteMultipleRegisters {
            start: 8,
            values: Words::new(&[0xAAAA, 0xBBBB, 0xCCCC]).unwrap(),
        })
        .unwrap_err();

    assert_eq!(error, ExceptionCode::ILLEGAL_DATA_ADDRESS);
    assert_eq!(
        device.register(WordTable::HoldingRegisters, 8),
        Some(0x1111),
        "the first register of a failed write was modified"
    );
    assert_eq!(
        device.register(WordTable::HoldingRegisters, 9),
        Some(0x2222)
    );
}

#[test]
fn a_multiple_coil_write_that_runs_off_the_end_changes_nothing_at_all() {
    let mut device = Device::empty().with_bits(BitTable::Coils, 0, 10);
    device.set_bit(BitTable::Coils, 9, true);
    let error = device
        .apply(&Request::WriteMultipleCoils {
            start: 9,
            values: Bits::from_iter_of([false, false, false]).unwrap(),
        })
        .unwrap_err();
    assert_eq!(error, ExceptionCode::ILLEGAL_DATA_ADDRESS);
    assert_eq!(
        device.bit(BitTable::Coils, 9),
        Some(true),
        "a coil was cleared by a write that was refused"
    );
}

#[test]
fn coils_written_over_the_network_read_back_the_same_way() {
    let mut device = device();
    let pattern = [true, false, true, true, false, false, false, true, true];
    device
        .apply(&Request::WriteMultipleCoils {
            start: 20,
            values: Bits::from_iter_of(pattern).unwrap(),
        })
        .unwrap();
    let response = device
        .apply(&Request::ReadCoils {
            start: 20,
            quantity: 9,
        })
        .unwrap();
    let Response::ReadCoils(bits) = response else {
        panic!("{response:?}")
    };
    assert_eq!(bits.iter().collect::<Vec<_>>(), pattern);
}

#[test]
fn the_write_limits_differ_from_the_read_limits() {
    // 125 registers may be read and 123 written. A device that used one limit
    // for both would accept a write no real device accepts.
    let mut device = Device::empty().with_registers(WordTable::HoldingRegisters, 0, 200);
    let at_limit = Words::new(&[0; MAX_WRITE_REGISTERS as usize]).unwrap();
    assert!(
        device
            .apply(&Request::WriteMultipleRegisters {
                start: 0,
                values: at_limit
            })
            .is_ok()
    );
    let over = Words::new(&[0; MAX_WRITE_REGISTERS as usize + 1]).unwrap();
    assert_eq!(
        device
            .apply(&Request::WriteMultipleRegisters {
                start: 0,
                values: over
            })
            .unwrap_err(),
        ExceptionCode::ILLEGAL_DATA_VALUE
    );
}

// -- what the network may not touch --------------------------------------

#[test]
fn the_process_can_write_an_input_that_the_network_can_only_read() {
    // A discrete input is read-only to the *network*. The device's own process
    // writes it constantly — that is what makes it an input — and a simulator
    // that could not drive one would be useless.
    let mut device = device();
    assert!(device.set_bit(BitTable::DiscreteInputs, 4, true));
    let response = device
        .apply(&Request::ReadDiscreteInputs {
            start: 4,
            quantity: 1,
        })
        .unwrap();
    let Response::ReadDiscreteInputs(bits) = response else {
        panic!("{response:?}")
    };
    assert_eq!(bits.get(0), Some(true));
}

#[test]
fn no_request_type_can_write_a_read_only_table() {
    // Not a runtime check: Modbus has no function code that writes a discrete
    // input or an input register, so the request type cannot be constructed.
    // Recorded as a test so that the guarantee is stated somewhere a reader
    // will find it.
    assert!(!Table::of_bits(BitTable::DiscreteInputs).is_writable_over_the_network());
    assert!(!Table::of_words(WordTable::InputRegisters).is_writable_over_the_network());
    assert!(Table::of_bits(BitTable::Coils).is_writable_over_the_network());
    assert!(Table::of_words(WordTable::HoldingRegisters).is_writable_over_the_network());
}

// -- end to end ----------------------------------------------------------

#[test]
fn a_request_encoded_decoded_applied_and_answered_round_trips() {
    // The whole pure path: bytes to a request, a request to a response, a
    // response back to bytes, and back to a typed response again. Every layer
    // that will later have a socket in front of it, exercised without one.
    let mut device = device();
    device.set_register(WordTable::HoldingRegisters, 0, 0x022B);
    device.set_register(WordTable::HoldingRegisters, 1, 0x0000);
    device.set_register(WordTable::HoldingRegisters, 2, 0x0064);

    let request = Request::ReadHoldingRegisters {
        start: 0,
        quantity: 3,
    };
    let on_the_wire = request.encode();
    let received = Request::decode(on_the_wire.as_bytes()).unwrap();
    assert_eq!(received, request);

    let answer = device.apply(&received).unwrap();
    let answer_bytes = answer.encode();
    assert_eq!(
        answer_bytes.as_bytes(),
        [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]
    );
    assert_eq!(
        Response::decode(answer_bytes.as_bytes(), &request).unwrap(),
        answer
    );
}

#[test]
fn an_exception_is_a_response_like_any_other() {
    let mut device = Device::empty();
    let request = Request::ReadHoldingRegisters {
        start: 0,
        quantity: 1,
    };
    let code = device.apply(&request).unwrap_err();
    let response = Response::Exception {
        function: request.function(),
        code,
    };
    assert_eq!(response.encode().as_bytes(), [0x83, 0x02]);
    assert_eq!(
        Response::decode(response.encode().as_bytes(), &request).unwrap(),
        response
    );
}

#[test]
fn no_request_makes_the_device_panic() {
    // Requests reaching a server come off a network. They are all structurally
    // valid by the time they get here — the decoder saw to that — but every
    // address and quantity a decoder accepts must be survivable.
    let mut device = device();
    let mut seed = 0xF00D_BABE_DEAD_BEEF_u64;
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for _ in 0..20_000 {
        let start = (next() >> 17) as u16;
        let quantity = (next() >> 17) as u16;
        let requests = [
            Request::ReadCoils { start, quantity },
            Request::ReadDiscreteInputs { start, quantity },
            Request::ReadHoldingRegisters { start, quantity },
            Request::ReadInputRegisters { start, quantity },
            Request::WriteSingleCoil {
                address: start,
                on: quantity.is_multiple_of(2),
            },
            Request::WriteSingleRegister {
                address: start,
                value: quantity,
            },
        ];
        for request in requests {
            let _ = device.apply(&request);
        }
    }
}
