// SPDX-License-Identifier: Apache-2.0
//! A salman simulator with known contents, for an independent client to drive.
//!
//! Used by `.github/workflows/interop.yml` and runnable by hand:
//!
//! ```text
//! cargo run -p salman-modbus-net --example simulator -- 15502
//! ```
//!
//! It prints the address it is listening on, serves for `seconds` (twenty by
//! default), then prints what was written to it so the driver can check it.
//! A deadline rather than a signal or a closed pipe, because those behave
//! differently in every runner and this has to work the same on a laptop and
//! in CI.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use std::io::Write;

use salman_core::posture::PostureState;
use salman_modbus::device::{BitTable, Device, WordTable};
use salman_modbus_net::server::Server;

fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(0);
    let seconds: u64 = std::env::args()
        .nth(2)
        .and_then(|a| a.parse().ok())
        .unwrap_or(20);

    let mut device = Device::empty()
        .with_bits(BitTable::Coils, 0, 100)
        .with_bits(BitTable::DiscreteInputs, 0, 100)
        .with_registers(WordTable::HoldingRegisters, 0, 100)
        .with_registers(WordTable::InputRegisters, 0, 100);
    // Values the driver knows and checks. Deliberately different per table, so
    // a client reading the wrong one is obvious rather than plausible.
    for address in 0..20_u16 {
        device.set_register(WordTable::HoldingRegisters, address, 0x1000 + address);
        device.set_register(WordTable::InputRegisters, address, 0x2000 + address);
        device.set_bit(BitTable::DiscreteInputs, address, address % 3 == 0);
    }

    let mut posture = PostureState::new();
    posture.simulate();
    let handle = Server::bind(("127.0.0.1", port), device, &posture, 0)
        .expect("a simulator may run while simulating")
        .spawn()
        .expect("the accept loop starts");
    println!("listening {}", handle.address());
    // Flush before sleeping, or a driver waiting for that line waits for ever.
    let _ = std::io::stdout().flush();

    std::thread::sleep(std::time::Duration::from_secs(seconds));

    handle
        .with_device(|device| {
            let coils: Vec<u8> = (0..10_u16)
                .map(|a| u8::from(device.bit(BitTable::Coils, a).unwrap_or(false)))
                .collect();
            let holding: Vec<u16> = (0..10_u16)
                .map(|a| device.register(WordTable::HoldingRegisters, a).unwrap_or(0))
                .collect();
            println!("coils {coils:?}");
            println!("holding {holding:?}");
        })
        .expect("the device is still there");
}
