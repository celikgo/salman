// SPDX-License-Identifier: Apache-2.0
//! A client and a simulator driving each other over a real socket.
//!
//! No hardware, and no mock either: these are real TCP connections on
//! loopback, with the operating system choosing the port. What is being tested
//! is the part that only appears once bytes actually move — a response
//! arriving in two segments, a connection closing under a client, a write
//! being refused before it reaches the wire.
//!
//! The tests about **refusal** are the ones that matter most. This is the
//! first code path in salman that can change a device, and the posture model
//! was written before it for exactly this moment.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use salman_core::posture::{
    ConfirmationPrompt, ConfirmationRequest, Decision, Effect, PostureState, UserConfirmation,
};
use salman_modbus::device::{BitTable, Device, WordTable};
use salman_modbus::function::ExceptionCode;
use salman_modbus::pdu::{Request, Response, Words};
use salman_modbus_net::client::{Client, ClientError, DEFAULT_TIMEOUT};
use salman_modbus_net::server::{Server, ServerError, ServerHandle};

/// A prompt that approves, so that a test can obtain a `UserConfirmation`.
///
/// Its existence here rather than in the library is the point: production code
/// cannot get a confirmation without something that can put the question to a
/// person.
struct Approve;
impl ConfirmationPrompt for Approve {
    fn confirm(&mut self, _request: &ConfirmationRequest) -> Decision {
        Decision::Approved
    }
}

struct Refuse;
impl ConfirmationPrompt for Refuse {
    fn confirm(&mut self, _request: &ConfirmationRequest) -> Decision {
        Decision::Refused
    }
}

fn confirmation() -> UserConfirmation {
    ConfirmationRequest {
        effect: Effect::WriteLiveDevice,
        device: "the simulator on loopback".to_string(),
        address: "holding register 0 (PDU address, 0-based)".to_string(),
        current_value: Some("0".to_string()),
        new_value: Some("42".to_string()),
        declared_intent: "a test of the write path".to_string(),
    }
    .ask(&mut Approve)
    .expect("Approve approves")
}

/// A posture that permits simulated writes.
fn simulating() -> PostureState {
    let mut posture = PostureState::new();
    posture.simulate();
    posture
}

/// A posture armed for live writes.
fn armed() -> PostureState {
    let mut posture = PostureState::new();
    posture.arm(confirmation(), 0, 60_000);
    posture
}

fn device() -> Device {
    Device::empty()
        .with_bits(BitTable::Coils, 0, 100)
        .with_bits(BitTable::DiscreteInputs, 0, 100)
        .with_registers(WordTable::HoldingRegisters, 0, 100)
        .with_registers(WordTable::InputRegisters, 0, 100)
}

/// Starts a simulator on a port the operating system picks.
fn simulator(device: Device) -> ServerHandle {
    Server::bind("127.0.0.1:0", device, &simulating(), 0)
        .expect("a simulator may run while simulating")
        .spawn()
        .expect("the accept loop starts")
}

fn client_for(handle: &ServerHandle) -> Client {
    Client::connect(handle.address(), DEFAULT_TIMEOUT).expect("the simulator is listening")
}

// -- reading -------------------------------------------------------------

#[test]
fn a_read_crosses_a_real_socket_and_comes_back() {
    let mut device = device();
    device.set_register(WordTable::HoldingRegisters, 7, 0x1234);
    let handle = simulator(device);
    let mut client = client_for(&handle);

    let response = client
        .read_from(
            1,
            &Request::ReadHoldingRegisters {
                start: 7,
                quantity: 1,
            },
        )
        .expect("the simulator answers");

    let Response::ReadHoldingRegisters(words) = response else {
        panic!("{response:?}")
    };
    assert_eq!(words.values(), [0x1234]);
    assert_eq!(handle.served(), 1);
}

#[test]
fn many_requests_on_one_connection_each_get_their_own_answer() {
    // Transaction identifiers have to advance and have to be matched. A client
    // that ignored them would appear to work here and would return the
    // previous answer the moment anything was slow.
    let handle = simulator(device());
    let mut client = client_for(&handle);
    for address in 0..20_u16 {
        client
            .write(
                1,
                &Request::WriteSingleRegister {
                    address,
                    value: address * 3,
                },
                &armed(),
                0,
                confirmation(),
            )
            .expect("the write is authorised and succeeds");
    }
    for address in 0..20_u16 {
        let response = client
            .read_from(
                1,
                &Request::ReadHoldingRegisters {
                    start: address,
                    quantity: 1,
                },
            )
            .expect("the read succeeds");
        let Response::ReadHoldingRegisters(words) = response else {
            panic!("{response:?}")
        };
        assert_eq!(words.values(), [address * 3], "at address {address}");
    }
    assert_eq!(client.unmatched_responses(), 0);
}

#[test]
fn the_unit_identifier_is_echoed_rather_than_replaced() {
    // MG §4.4.1.2 specifies 0xFF for a device that is not a gateway and also
    // accepts 0x00. A client is entitled to send either and to see back what
    // it sent, and a server that normalised it would break transaction
    // matching for a client that checks.
    let handle = simulator(device());
    for unit in [0x00_u8, 0x01, 0x11, 0xFF] {
        let mut client = client_for(&handle);
        client
            .read_from(
                unit,
                &Request::ReadHoldingRegisters {
                    start: 0,
                    quantity: 1,
                },
            )
            .unwrap_or_else(|e| panic!("unit {unit} was not answered: {e}"));
    }
}

#[test]
fn an_address_outside_the_map_comes_back_as_the_exception_the_device_chose() {
    let handle = simulator(Device::empty().with_registers(WordTable::HoldingRegisters, 0, 10));
    let mut client = client_for(&handle);
    let error = client
        .read_from(
            1,
            &Request::ReadHoldingRegisters {
                start: 8,
                quantity: 5,
            },
        )
        .unwrap_err();
    match error {
        ClientError::Exception { code, unit } => {
            assert_eq!(code, ExceptionCode::ILLEGAL_DATA_ADDRESS);
            assert_eq!(unit, 1);
        }
        other => panic!("expected an exception, got {other}"),
    }
}

// -- the posture gate ----------------------------------------------------

#[test]
fn a_write_is_refused_at_every_posture_below_armed() {
    // The whole reason the posture model was written before anything could
    // write. Observing and simulating are not enough: a live write needs the
    // ARMED posture, and salman checks it here as well as wherever the caller
    // checked it, because a check a caller can forget is not a boundary.
    let handle = simulator(device());
    let mut client = client_for(&handle);

    for posture in [PostureState::new(), simulating()] {
        let error = client
            .write(
                1,
                &Request::WriteSingleRegister {
                    address: 0,
                    value: 42,
                },
                &posture,
                0,
                confirmation(),
            )
            .unwrap_err();
        assert!(
            matches!(error, ClientError::Refused { .. }),
            "a write went through at posture {}: {error}",
            posture.posture(0)
        );
    }

    // And nothing reached the device.
    handle
        .with_device(|device| {
            assert_eq!(device.register(WordTable::HoldingRegisters, 0), Some(0));
        })
        .unwrap();
}

#[test]
fn a_write_that_a_person_refused_cannot_be_made_at_all() {
    // There is no confirmation to pass, so there is no call to make. The test
    // is that `ask` returns None: without it there is no value of the type the
    // write path requires, and that is a compile-time fact rather than a
    // runtime check.
    let refused = ConfirmationRequest {
        effect: Effect::WriteLiveDevice,
        device: "a device".to_string(),
        address: "holding register 0".to_string(),
        current_value: None,
        new_value: Some("42".to_string()),
        declared_intent: "a test".to_string(),
    }
    .ask(&mut Refuse);
    assert!(refused.is_none());
}

#[test]
fn an_armed_write_with_a_confirmation_reaches_the_device() {
    let handle = simulator(device());
    let mut client = client_for(&handle);
    client
        .write(
            1,
            &Request::WriteSingleRegister {
                address: 5,
                value: 0xBEEF,
            },
            &armed(),
            0,
            confirmation(),
        )
        .expect("armed, confirmed, and inside the map");
    handle
        .with_device(|device| {
            assert_eq!(
                device.register(WordTable::HoldingRegisters, 5),
                Some(0xBEEF)
            );
        })
        .unwrap();
}

#[test]
fn an_arming_grant_that_has_expired_does_not_authorise_a_write() {
    // ARMED times out. A session left armed and forgotten is the failure mode
    // the timeout exists for, and it has to be enforced where the write
    // happens rather than only where the arming did.
    let handle = simulator(device());
    let mut client = client_for(&handle);
    let mut posture = PostureState::new();
    posture.arm(confirmation(), 0, 1_000);

    assert!(
        client
            .write(
                1,
                &Request::WriteSingleRegister {
                    address: 0,
                    value: 1
                },
                &posture,
                999,
                confirmation(),
            )
            .is_ok(),
        "still inside the grant"
    );

    let error = client
        .write(
            1,
            &Request::WriteSingleRegister {
                address: 0,
                value: 2,
            },
            &posture,
            1_001,
            confirmation(),
        )
        .unwrap_err();
    assert!(matches!(error, ClientError::Refused { .. }), "{error}");
    handle
        .with_device(|device| {
            assert_eq!(device.register(WordTable::HoldingRegisters, 0), Some(1));
        })
        .unwrap();
}

#[test]
fn a_write_offered_to_the_read_path_is_refused_rather_than_quietly_sent() {
    // The read path does not check permissions, because reads do not need
    // them. So it must refuse anything that is not a read, rather than trust
    // that no caller would.
    let handle = simulator(device());
    let mut client = client_for(&handle);
    let error = client
        .read_from(
            1,
            &Request::WriteSingleRegister {
                address: 0,
                value: 42,
            },
        )
        .unwrap_err();
    assert!(matches!(error, ClientError::NotARead { .. }), "{error}");
    handle
        .with_device(|device| {
            assert_eq!(device.register(WordTable::HoldingRegisters, 0), Some(0));
        })
        .unwrap();
}

#[test]
fn a_read_offered_to_the_write_path_is_refused() {
    let handle = simulator(device());
    let mut client = client_for(&handle);
    let error = client
        .write(
            1,
            &Request::ReadHoldingRegisters {
                start: 0,
                quantity: 1,
            },
            &armed(),
            0,
            confirmation(),
        )
        .unwrap_err();
    assert!(matches!(error, ClientError::NotAWrite { .. }), "{error}");
}

#[test]
fn a_simulator_will_not_run_at_the_observing_posture() {
    // Read-only by default means the simulator does not start either. Its
    // whole purpose is to accept writes.
    let error = Server::bind("127.0.0.1:0", device(), &PostureState::new(), 0).unwrap_err();
    assert!(matches!(error, ServerError::Refused { .. }), "{error}");
}

// -- what happens when the network misbehaves ----------------------------

#[test]
fn a_response_split_across_two_segments_is_reassembled() {
    // A hand-written server that writes its header and its body in two calls.
    // The client must not care.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 12];
        stream.read_exact(&mut request).unwrap();
        let transaction = &request[..2];
        // 03 06 02 2B 00 00 00 64, in two writes with a pause between them.
        let mut header = transaction.to_vec();
        header.extend_from_slice(&[0x00, 0x00, 0x00, 0x09, 0x01, 0x03, 0x06, 0x02]);
        stream.write_all(&header).unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        stream.write_all(&[0x2B, 0x00, 0x00, 0x00, 0x64]).unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(50));
    });

    let mut client = Client::connect(address, DEFAULT_TIMEOUT).unwrap();
    let response = client
        .read_from(
            1,
            &Request::ReadHoldingRegisters {
                start: 0,
                quantity: 3,
            },
        )
        .expect("a split response is still a response");
    let Response::ReadHoldingRegisters(words) = response else {
        panic!("{response:?}")
    };
    assert_eq!(words.values(), [0x022B, 0x0000, 0x0064]);
    server.join().unwrap();
}

#[test]
fn a_server_that_closes_the_connection_is_reported_as_such() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        drop(stream);
    });
    let mut client = Client::connect(address, DEFAULT_TIMEOUT).unwrap();
    let error = client
        .read_from(
            1,
            &Request::ReadHoldingRegisters {
                start: 0,
                quantity: 1,
            },
        )
        .unwrap_err();
    assert!(
        matches!(
            error,
            ClientError::ConnectionClosed | ClientError::Io(_) | ClientError::Framing(_)
        ),
        "{error}"
    );
    server.join().unwrap();
}

#[test]
fn a_stale_response_is_counted_and_skipped_rather_than_returned() {
    // The failure this prevents is the worst kind: an earlier request timed
    // out, its answer arrives late, and a client that matched nothing would
    // hand back last question's answer to this question. It would look
    // entirely plausible.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 12];
        stream.read_exact(&mut request).unwrap();
        // First an answer to transaction 999, which nobody asked for.
        stream
            .write_all(&[
                0x03, 0xE7, 0x00, 0x00, 0x00, 0x05, 0x01, 0x03, 0x02, 0xDE, 0xAD,
            ])
            .unwrap();
        // Then the real one, carrying the transaction that was asked.
        let mut reply = request[..2].to_vec();
        reply.extend_from_slice(&[0x00, 0x00, 0x00, 0x05, 0x01, 0x03, 0x02, 0x00, 0x2A]);
        stream.write_all(&reply).unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(50));
    });

    let mut client = Client::connect(address, DEFAULT_TIMEOUT).unwrap();
    let response = client
        .read_from(
            1,
            &Request::ReadHoldingRegisters {
                start: 0,
                quantity: 1,
            },
        )
        .expect("the matching answer arrives after the stale one");
    let Response::ReadHoldingRegisters(words) = response else {
        panic!("{response:?}")
    };
    assert_eq!(words.values(), [0x002A], "the stale answer was returned");
    assert_eq!(client.unmatched_responses(), 1);
    server.join().unwrap();
}

#[test]
fn a_server_that_never_answers_times_out_rather_than_waiting_for_ever() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        drop(stream);
    });
    let mut client = Client::connect(address, Duration::from_millis(80)).unwrap();
    let started = std::time::Instant::now();
    let error = client
        .read_from(
            1,
            &Request::ReadHoldingRegisters {
                start: 0,
                quantity: 1,
            },
        )
        .unwrap_err();
    assert!(matches!(error, ClientError::Io(_)), "{error}");
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "the timeout was not honoured"
    );
    server.join().unwrap();
}

#[test]
fn an_unimplemented_function_is_answered_with_exception_one() {
    // Not silence. On TCP the transport has already vouched for the bytes, so
    // a frame salman cannot act on still deserves an answer saying so.
    let handle = simulator(device());
    let mut stream = TcpStream::connect(handle.address()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    // Read FIFO Queue, which salman does not implement.
    stream
        .write_all(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x04, 0x01, 0x18, 0x00, 0x00])
        .unwrap();
    stream.flush().unwrap();
    let mut reply = [0_u8; 9];
    stream.read_exact(&mut reply).unwrap();
    assert_eq!(
        reply,
        [0x00, 0x01, 0x00, 0x00, 0x00, 0x03, 0x01, 0x98, 0x01],
        "expected 0x98 0x01: the exception form of 0x18, illegal function"
    );
}

#[test]
fn a_frame_that_cannot_be_framed_closes_the_connection() {
    // A non-zero protocol identifier. There is nothing in the stream that says
    // where the next frame starts, so answering and carrying on would mean
    // answering from the middle of something.
    let handle = simulator(device());
    let mut stream = TcpStream::connect(handle.address()).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    stream
        .write_all(&[
            0x00, 0x01, 0x00, 0x10, 0x00, 0x06, 0x01, 0x03, 0x00, 0x00, 0x00, 0x01,
        ])
        .unwrap();
    stream.flush().unwrap();
    let mut reply = [0_u8; 16];
    let read = stream.read(&mut reply).unwrap_or(0);
    assert_eq!(read, 0, "the server answered a frame it could not frame");
}

#[test]
fn a_simulator_stops_when_its_handle_is_dropped() {
    let address = {
        let handle = simulator(device());
        let address = handle.address();
        let mut client = client_for(&handle);
        client
            .read_from(
                1,
                &Request::ReadHoldingRegisters {
                    start: 0,
                    quantity: 1,
                },
            )
            .expect("it is running");
        address
    };
    // The handle is gone, so the listener is closed and nothing can connect.
    // A simulator a test left listening would collide with the next one.
    assert!(
        TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err(),
        "the simulator is still listening after its handle was dropped"
    );
}

#[test]
fn a_multiple_write_crosses_the_socket_whole() {
    let handle = simulator(device());
    let mut client = client_for(&handle);
    client
        .write(
            1,
            &Request::WriteMultipleRegisters {
                start: 10,
                values: Words::new(&[1, 2, 3, 4, 5]).unwrap(),
            },
            &armed(),
            0,
            confirmation(),
        )
        .expect("armed and confirmed");
    let response = client
        .read_from(
            1,
            &Request::ReadHoldingRegisters {
                start: 10,
                quantity: 5,
            },
        )
        .unwrap();
    let Response::ReadHoldingRegisters(words) = response else {
        panic!("{response:?}")
    };
    assert_eq!(words.values(), [1, 2, 3, 4, 5]);
}
