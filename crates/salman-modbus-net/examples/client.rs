// SPDX-License-Identifier: Apache-2.0
//! salman's client, driving whatever Modbus TCP server it is pointed at.
//!
//! Used by `.github/workflows/interop.yml` against pymodbus, and runnable by
//! hand:
//!
//! ```text
//! cargo run -p salman-modbus-net --example client -- 15502
//! ```
//!
//! It prints one line per exchange in a form the driver script compares
//! against what the other implementation was configured to hold.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use std::time::Duration;

use salman_core::posture::{
    ConfirmationPrompt, ConfirmationRequest, Decision, Effect, PostureState, UserConfirmation,
};
use salman_modbus::pdu::{Request, Response, Words};
use salman_modbus_net::client::Client;

/// Approves, because this is an interoperability harness and the person
/// running it is the one who started it.
///
/// It lives in an example rather than in the library for the reason the
/// posture model exists: production code cannot obtain a confirmation without
/// something that can put the question to a person.
struct Approve;

impl ConfirmationPrompt for Approve {
    fn confirm(&mut self, _request: &ConfirmationRequest) -> Decision {
        Decision::Approved
    }
}

fn confirmation() -> UserConfirmation {
    ConfirmationRequest {
        effect: Effect::WriteLiveDevice,
        device: "the interoperability server".to_string(),
        address: "a holding register".to_string(),
        current_value: None,
        new_value: None,
        declared_intent: "an interoperability check".to_string(),
    }
    .ask(&mut Approve)
    .expect("Approve approves")
}

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .expect("a port");
    let mut client =
        Client::connect(("127.0.0.1", port), Duration::from_secs(5)).expect("the server is up");

    let mut armed = PostureState::new();
    armed.arm(confirmation(), 0, 600_000);

    if let Ok(Response::ReadHoldingRegisters(words)) = client.read_from(
        1,
        &Request::ReadHoldingRegisters {
            start: 0,
            quantity: 5,
        },
    ) {
        println!("holding {:?}", words.values());
    }
    if let Ok(Response::ReadInputRegisters(words)) = client.read_from(
        1,
        &Request::ReadInputRegisters {
            start: 0,
            quantity: 5,
        },
    ) {
        println!("input {:?}", words.values());
    }
    if let Ok(Response::ReadCoils(bits)) = client.read_from(
        1,
        &Request::ReadCoils {
            start: 0,
            quantity: 8,
        },
    ) {
        println!("coils {:?}", bits.iter().map(u8::from).collect::<Vec<_>>());
    }

    client
        .write(
            1,
            &Request::WriteSingleRegister {
                address: 20,
                value: 0xBEEF,
            },
            &armed,
            0,
            confirmation(),
        )
        .expect("a single write");
    client
        .write(
            1,
            &Request::WriteMultipleRegisters {
                start: 30,
                values: Words::new(&[1, 2, 3]).unwrap(),
            },
            &armed,
            0,
            confirmation(),
        )
        .expect("a multiple write");

    if let Ok(Response::ReadHoldingRegisters(words)) = client.read_from(
        1,
        &Request::ReadHoldingRegisters {
            start: 20,
            quantity: 1,
        },
    ) {
        println!("wrote-then-read {:?}", words.values());
    }
    if let Ok(Response::ReadHoldingRegisters(words)) = client.read_from(
        1,
        &Request::ReadHoldingRegisters {
            start: 30,
            quantity: 3,
        },
    ) {
        println!("wrote-then-read-many {:?}", words.values());
    }

    // An address no server in this harness has. It must come back as an
    // exception rather than as a plausible value.
    match client.read_from(
        1,
        &Request::ReadHoldingRegisters {
            start: 60_000,
            quantity: 4,
        },
    ) {
        Err(error) => println!("out-of-range {error}"),
        Ok(response) => println!("out-of-range UNEXPECTEDLY-OK {response:?}"),
    }
}
