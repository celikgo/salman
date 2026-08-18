// SPDX-License-Identifier: Apache-2.0
//! Function codes and exception codes.
//!
//! Both are held as newtypes over `u8` rather than as closed enumerations,
//! because the wire can carry values no enumeration anticipates and salman has
//! to be able to say *what* it saw. A closed enum forces every unknown byte
//! into one `Other` arm and loses the number, which is the one thing a person
//! reading a diagnostic needs.

use core::fmt;

/// A Modbus function code, as it appears on the wire.
///
/// Codes are classified by APS §4.4 and Annex A into public, user-defined and
/// reserved ranges. `salman` implements a few of them, decodes the rest by
/// number, and never guesses at a meaning it cannot cite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FunctionCode(pub u8);

impl FunctionCode {
    /// Read Coils. APS §6.1.
    pub const READ_COILS: Self = Self(0x01);
    /// Read Discrete Inputs. APS §6.2.
    pub const READ_DISCRETE_INPUTS: Self = Self(0x02);
    /// Read Holding Registers. APS §6.3.
    pub const READ_HOLDING_REGISTERS: Self = Self(0x03);
    /// Read Input Registers. APS §6.4.
    pub const READ_INPUT_REGISTERS: Self = Self(0x04);
    /// Write Single Coil. APS §6.5.
    pub const WRITE_SINGLE_COIL: Self = Self(0x05);
    /// Write Single Register. APS §6.6.
    pub const WRITE_SINGLE_REGISTER: Self = Self(0x06);
    /// Read Exception Status. Serial line only. APS §6.7.
    pub const READ_EXCEPTION_STATUS: Self = Self(0x07);
    /// Diagnostics. Serial line only. APS §6.8.
    pub const DIAGNOSTICS: Self = Self(0x08);
    /// Get Comm Event Counter. Serial line only. APS §6.9.
    pub const GET_COMM_EVENT_COUNTER: Self = Self(0x0B);
    /// Get Comm Event Log. Serial line only. APS §6.10.
    pub const GET_COMM_EVENT_LOG: Self = Self(0x0C);
    /// Write Multiple Coils. APS §6.11.
    pub const WRITE_MULTIPLE_COILS: Self = Self(0x0F);
    /// Write Multiple Registers. APS §6.12.
    pub const WRITE_MULTIPLE_REGISTERS: Self = Self(0x10);
    /// Report Server ID. Serial line only. APS §6.13.
    pub const REPORT_SERVER_ID: Self = Self(0x11);
    /// Read File Record. APS §6.14.
    pub const READ_FILE_RECORD: Self = Self(0x14);
    /// Write File Record. APS §6.15.
    pub const WRITE_FILE_RECORD: Self = Self(0x15);
    /// Mask Write Register. APS §6.16.
    pub const MASK_WRITE_REGISTER: Self = Self(0x16);
    /// Read/Write Multiple Registers. APS §6.17.
    pub const READ_WRITE_MULTIPLE_REGISTERS: Self = Self(0x17);
    /// Read FIFO Queue. APS §6.18.
    pub const READ_FIFO_QUEUE: Self = Self(0x18);
    /// Encapsulated Interface Transport. APS §6.19.
    pub const ENCAPSULATED_INTERFACE: Self = Self(0x2B);

    /// The high bit that marks a response as an exception. APS §7.
    const EXCEPTION_FLAG: u8 = 0x80;

    /// Whether this code marks an exception response.
    ///
    /// An exception response carries the request's function code with the high
    /// bit set, so `0x83` is the exception form of `0x03`.
    #[must_use]
    pub const fn is_exception(self) -> bool {
        self.0 & Self::EXCEPTION_FLAG != 0
    }

    /// This code with the exception bit set.
    #[must_use]
    pub const fn as_exception(self) -> Self {
        Self(self.0 | Self::EXCEPTION_FLAG)
    }

    /// The code this exception response is a reply to, if it is one.
    #[must_use]
    pub const fn without_exception_flag(self) -> Self {
        Self(self.0 & !Self::EXCEPTION_FLAG)
    }

    /// Whether the code falls in a range APS reserves for user definition.
    ///
    /// APS §4.4 sets aside 65..=72 and 100..=110. A device may put anything
    /// there, so salman decodes the number and nothing else.
    #[must_use]
    pub const fn is_user_defined(self) -> bool {
        matches!(self.0, 65..=72 | 100..=110)
    }

    /// Whether the code is in the public ranges APS assigns.
    ///
    /// Public does not mean salman implements it, and it does not mean the
    /// code is assigned: it means the range is one the Modbus Organization
    /// controls.
    #[must_use]
    pub const fn is_public(self) -> bool {
        matches!(self.0, 1..=64 | 73..=99 | 111..=127)
    }

    /// Whether the code is one that only exists on a serial line.
    ///
    /// APS gives these codes serial-line headings. They are meaningless over
    /// TCP, where there is no shared bus to diagnose, and salman refuses to
    /// send them over TCP rather than emitting something a gateway may
    /// mistranslate.
    #[must_use]
    pub const fn is_serial_line_only(self) -> bool {
        matches!(self.0, 0x07 | 0x08 | 0x0B | 0x0C | 0x11)
    }

    /// The name APS gives this code, or `None` if salman cannot cite one.
    ///
    /// `None` is a real answer here. A device is free to use a user-defined
    /// code for anything, and inventing a name for it would be the kind of
    /// confident lie this project exists to avoid.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        Some(match self.0 {
            0x01 => "Read Coils",
            0x02 => "Read Discrete Inputs",
            0x03 => "Read Holding Registers",
            0x04 => "Read Input Registers",
            0x05 => "Write Single Coil",
            0x06 => "Write Single Register",
            0x07 => "Read Exception Status",
            0x08 => "Diagnostics",
            0x0B => "Get Comm Event Counter",
            0x0C => "Get Comm Event Log",
            0x0F => "Write Multiple Coils",
            0x10 => "Write Multiple Registers",
            0x11 => "Report Server ID",
            0x14 => "Read File Record",
            0x15 => "Write File Record",
            0x16 => "Mask Write Register",
            0x17 => "Read/Write Multiple Registers",
            0x18 => "Read FIFO Queue",
            0x2B => "Encapsulated Interface Transport",
            _ => return None,
        })
    }
}

impl fmt::Display for FunctionCode {
    /// Renders as `0x03 Read Holding Registers`, or `0x63` when unnamed.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:02X}", self.0)?;
        if let Some(name) = self.name() {
            write!(f, " {name}")?;
        }
        Ok(())
    }
}

/// An exception code, carried in the two-byte exception response. APS §7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExceptionCode(pub u8);

impl ExceptionCode {
    /// The function code is not supported by the server. APS §7, code 01.
    pub const ILLEGAL_FUNCTION: Self = Self(0x01);
    /// The address, or the address together with the quantity, is outside the
    /// server's map. APS §7, code 02.
    pub const ILLEGAL_DATA_ADDRESS: Self = Self(0x02);
    /// A value in the request is structurally wrong — a quantity out of range,
    /// or a byte count that disagrees with the quantity. APS §7, code 03.
    ///
    /// This does **not** mean a register value was outside what the
    /// application expected; APS §7 says so explicitly.
    pub const ILLEGAL_DATA_VALUE: Self = Self(0x03);
    /// The server failed while performing the action. APS §7, code 04.
    pub const SERVER_DEVICE_FAILURE: Self = Self(0x04);
    /// The request was accepted and needs a long time. APS §7, code 05.
    pub const ACKNOWLEDGE: Self = Self(0x05);
    /// The server is busy with a long-running command. APS §7, code 06.
    pub const SERVER_DEVICE_BUSY: Self = Self(0x06);
    /// A parity error was found in the extended file area. APS §7, code 08.
    pub const MEMORY_PARITY_ERROR: Self = Self(0x08);
    /// The gateway could not allocate an internal path. APS §7, code 0A.
    pub const GATEWAY_PATH_UNAVAILABLE: Self = Self(0x0A);
    /// The target device did not respond to the gateway. APS §7, code 0B.
    pub const GATEWAY_TARGET_FAILED_TO_RESPOND: Self = Self(0x0B);

    /// How salman describes this code.
    ///
    /// Code `0x07` is the interesting one. It is **not** in the APS §7 table,
    /// but the same document refers to it twice — §6.10 names it, and §6.8
    /// counts it. salman therefore decodes it, and says exactly that: it will
    /// not claim a definition the specification does not give, and it will not
    /// pretend the byte is meaningless when the specification uses it.
    #[must_use]
    pub const fn description(self) -> Option<&'static str> {
        Some(match self.0 {
            0x01 => "Illegal Function",
            0x02 => "Illegal Data Address",
            0x03 => "Illegal Data Value",
            0x04 => "Server Device Failure",
            0x05 => "Acknowledge",
            0x06 => "Server Device Busy",
            0x07 => {
                "Negative Acknowledge (referenced in APS V1.1b3 §6.10 and §6.8, \
                 and absent from the §7 exception table)"
            }
            0x08 => "Memory Parity Error",
            0x0A => "Gateway Path Unavailable",
            0x0B => "Gateway Target Device Failed To Respond",
            _ => return None,
        })
    }

    /// Whether APS §7's table defines this code.
    ///
    /// `0x07` is described and is not defined; the distinction is the point.
    #[must_use]
    pub const fn is_defined_by_the_specification(self) -> bool {
        matches!(self.0, 0x01..=0x06 | 0x08 | 0x0A | 0x0B)
    }
}

impl fmt::Display for ExceptionCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:02X}", self.0)?;
        if let Some(description) = self.description() {
            write!(f, " {description}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ExceptionCode, FunctionCode};

    #[test]
    fn the_exception_flag_round_trips() {
        let read = FunctionCode::READ_HOLDING_REGISTERS;
        assert!(!read.is_exception());
        assert_eq!(read.as_exception(), FunctionCode(0x83));
        assert!(read.as_exception().is_exception());
        assert_eq!(read.as_exception().without_exception_flag(), read);
    }

    #[test]
    fn every_named_code_is_classified_and_every_classification_is_exclusive() {
        for value in 0..=u8::MAX {
            let code = FunctionCode(value);
            assert!(
                !(code.is_public() && code.is_user_defined()),
                "0x{value:02X} claims two ranges at once"
            );
            if code.is_exception() {
                assert!(
                    !code.is_public() && !code.is_user_defined(),
                    "0x{value:02X} is above 127 and cannot be a request code"
                );
            }
        }
    }

    #[test]
    fn function_code_zero_is_neither_public_nor_user_defined() {
        // APS's ranges start at 1. Zero is invalid, and salman must not
        // silently treat it as a public code.
        let zero = FunctionCode(0x00);
        assert!(!zero.is_public());
        assert!(!zero.is_user_defined());
        assert!(zero.name().is_none());
    }

    #[test]
    fn the_serial_only_codes_are_exactly_the_five_aps_gives_serial_headings() {
        let serial: Vec<u8> = (0..=u8::MAX)
            .filter(|v| FunctionCode(*v).is_serial_line_only())
            .collect();
        assert_eq!(serial, [0x07, 0x08, 0x0B, 0x0C, 0x11]);
    }

    #[test]
    fn exception_seven_is_described_and_not_defined() {
        // The whole reason `is_defined_by_the_specification` exists. salman
        // decodes 0x07 because APS uses it, and refuses to claim APS defines
        // it, because APS §7's table does not.
        let nak = ExceptionCode(0x07);
        assert!(nak.description().is_some());
        assert!(!nak.is_defined_by_the_specification());
        let described = nak.description().unwrap();
        assert!(described.contains("absent from the §7 exception table"));
    }

    #[test]
    fn exception_nine_is_neither_described_nor_defined() {
        // 0x09 appears nowhere in APS. An implementation that invented a name
        // for it would be making one up.
        assert!(ExceptionCode(0x09).description().is_none());
        assert!(!ExceptionCode(0x09).is_defined_by_the_specification());
    }

    #[test]
    fn every_defined_code_has_a_description() {
        for value in 0..=u8::MAX {
            let code = ExceptionCode(value);
            if code.is_defined_by_the_specification() {
                assert!(code.description().is_some(), "0x{value:02X}");
            }
        }
    }

    #[test]
    fn display_names_the_code_and_keeps_the_number() {
        assert_eq!(
            FunctionCode::READ_HOLDING_REGISTERS.to_string(),
            "0x03 Read Holding Registers"
        );
        // An unknown code still renders its number: that is the one thing a
        // person reading a capture needs.
        assert_eq!(FunctionCode(0x63).to_string(), "0x63");
        assert_eq!(
            ExceptionCode::ILLEGAL_DATA_ADDRESS.to_string(),
            "0x02 Illegal Data Address"
        );
    }
}
