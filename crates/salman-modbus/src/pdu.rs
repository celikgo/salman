// SPDX-License-Identifier: Apache-2.0
//! Protocol data units: what a Modbus request and response mean, and how they
//! are written on the wire.
//!
//! A PDU is a function code and its data. It is the same on every transport —
//! that is the whole design of Modbus — so nothing in this module knows about
//! sockets, serial ports or capture files.
//!
//! # A response is not self-describing
//!
//! This is the fact that shapes the API, and it surprises people.
//!
//! A Read Coils response carries a byte count and the packed bits. It does
//! **not** carry how many coils were asked for. Eight coils and five coils
//! both produce one byte, and nothing in the response distinguishes them: the
//! quantity lives only in the request. So [`Response::decode`] takes the
//! request it answers, and a response whose request was never seen — a capture
//! that started mid-conversation, say — cannot be fully decoded by anyone.
//! salman says so rather than guessing at a count.
//!
//! # No allocation
//!
//! Every payload here is a fixed-size buffer sized by the specification's own
//! limits, so decoding a frame from an untrusted source allocates nothing and
//! a declared length can never be used to reserve memory. The quantity is
//! checked against [`limits`](crate::limits) before any byte is copied.

use core::fmt;

use crate::function::{ExceptionCode, FunctionCode};
use crate::limits::{
    COIL_OFF, COIL_ON, MAX_PDU, MAX_READ_BITS, MAX_READ_REGISTERS, MAX_WRITE_BITS,
    MAX_WRITE_REGISTERS, packed_bytes,
};

/// A packed run of coils or discrete inputs.
///
/// Bits are packed as APS §6.1 defines: the least significant bit of the first
/// byte is the item at the starting address, ascending to the most significant
/// bit and then into the next byte. Unused high bits of the final byte are
/// zero.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Bits {
    packed: [u8; Self::CAPACITY],
    count: u16,
}

impl Bits {
    /// Bytes needed for the largest quantity APS permits in one read.
    const CAPACITY: usize = packed_bytes(MAX_READ_BITS);

    /// An all-zero run of `count` bits, or `None` if `count` exceeds what one
    /// PDU can carry.
    #[must_use]
    pub const fn zeroed(count: u16) -> Option<Self> {
        if count > MAX_READ_BITS {
            return None;
        }
        Some(Self {
            packed: [0; Self::CAPACITY],
            count,
        })
    }

    /// Takes `count` bits from already-packed bytes.
    ///
    /// Returns `None` if `count` is beyond the PDU limit, or if `packed` is
    /// not exactly the number of bytes `count` needs. Being strict about the
    /// byte count is deliberate: a byte count that disagrees with the quantity
    /// is the shape of a malformed frame, and accepting it quietly would hide
    /// a real fault on the wire.
    #[must_use]
    pub fn from_packed(packed: &[u8], count: u16) -> Option<Self> {
        if count > MAX_READ_BITS || packed.len() != packed_bytes(count) {
            return None;
        }
        let mut bits = Self::zeroed(count)?;
        bits.packed.get_mut(..packed.len())?.copy_from_slice(packed);
        // A conforming sender zeroes the unused high bits of the last byte.
        // Not every sender conforms, and a stray bit there would make an
        // equality comparison between two identical readings fail, so they are
        // cleared here rather than trusted.
        bits.clear_padding();
        Some(bits)
    }

    /// Builds from one bit per element.
    #[must_use]
    pub fn from_iter_of(values: impl IntoIterator<Item = bool>) -> Option<Self> {
        let mut packed = [0_u8; Self::CAPACITY];
        let mut count: u16 = 0;
        for value in values {
            if count >= MAX_READ_BITS {
                return None;
            }
            if value {
                *packed.get_mut(count as usize / 8)? |= 1 << (count % 8);
            }
            count += 1;
        }
        Some(Self { packed, count })
    }

    /// Clears the bits above `count` in the final byte.
    fn clear_padding(&mut self) {
        let used = self.count as usize % 8;
        if used == 0 {
            return;
        }
        if let Some(last) = self.packed.get_mut(self.count as usize / 8) {
            *last &= (1_u8 << used) - 1;
        }
    }

    /// How many bits this holds.
    #[must_use]
    pub const fn count(&self) -> u16 {
        self.count
    }

    /// The packed bytes, exactly as they go on the wire.
    #[must_use]
    pub fn packed(&self) -> &[u8] {
        self.packed.get(..packed_bytes(self.count)).unwrap_or(&[])
    }

    /// One bit, or `None` past the end.
    #[must_use]
    pub fn get(&self, index: u16) -> Option<bool> {
        if index >= self.count {
            return None;
        }
        let byte = self.packed.get(index as usize / 8)?;
        Some(byte & (1 << (index % 8)) != 0)
    }

    /// Sets one bit. Returns `false` if the index is past the end.
    pub fn set(&mut self, index: u16, value: bool) -> bool {
        if index >= self.count {
            return false;
        }
        let Some(byte) = self.packed.get_mut(index as usize / 8) else {
            return false;
        };
        let mask = 1 << (index % 8);
        if value {
            *byte |= mask;
        } else {
            *byte &= !mask;
        }
        true
    }

    /// Every bit, in address order.
    pub fn iter(&self) -> impl Iterator<Item = bool> + '_ {
        (0..self.count).filter_map(|index| self.get(index))
    }
}

impl fmt::Debug for Bits {
    /// Renders as the bits themselves rather than as 250 mostly-zero bytes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bits[{}]", self.count)?;
        f.debug_list().entries(self.iter().map(u8::from)).finish()
    }
}

/// A run of 16-bit registers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Words {
    values: [u16; Self::CAPACITY],
    count: u8,
}

impl Words {
    /// Registers in the largest quantity APS permits in one read.
    const CAPACITY: usize = MAX_READ_REGISTERS as usize;

    /// Takes `values` as a register run, or `None` if there are too many.
    #[must_use]
    pub fn new(values: &[u16]) -> Option<Self> {
        if values.len() > Self::CAPACITY {
            return None;
        }
        let mut words = Self {
            values: [0; Self::CAPACITY],
            count: values.len() as u8,
        };
        words
            .values
            .get_mut(..values.len())?
            .copy_from_slice(values);
        Some(words)
    }

    /// How many registers this holds.
    #[must_use]
    pub const fn count(&self) -> u16 {
        self.count as u16
    }

    /// The registers.
    #[must_use]
    pub fn values(&self) -> &[u16] {
        self.values.get(..self.count as usize).unwrap_or(&[])
    }

    /// One register, or `None` past the end.
    #[must_use]
    pub fn get(&self, index: u16) -> Option<u16> {
        if index >= self.count() {
            return None;
        }
        self.values.get(index as usize).copied()
    }
}

impl fmt::Debug for Words {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Words[{}]", self.count)?;
        f.debug_list().entries(self.values()).finish()
    }
}

/// A request, decoded.
///
/// Addresses are the PDU addresses that appear on the wire: zero-based,
/// `0x0000` to `0xFFFF`. salman applies no offset anywhere — see
/// `docs/adr/ADR-0012-modbus-addressing.md` for why the `4xxxx` convention is
/// not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Read Coils. APS §6.1.
    ReadCoils {
        /// Zero-based PDU address of the first coil.
        start: u16,
        /// How many to read, 1 to 2000.
        quantity: u16,
    },
    /// Read Discrete Inputs. APS §6.2.
    ReadDiscreteInputs {
        /// Zero-based PDU address of the first input.
        start: u16,
        /// How many to read, 1 to 2000.
        quantity: u16,
    },
    /// Read Holding Registers. APS §6.3.
    ReadHoldingRegisters {
        /// Zero-based PDU address of the first register.
        start: u16,
        /// How many to read, 1 to 125.
        quantity: u16,
    },
    /// Read Input Registers. APS §6.4.
    ReadInputRegisters {
        /// Zero-based PDU address of the first register.
        start: u16,
        /// How many to read, 1 to 125.
        quantity: u16,
    },
    /// Write Single Coil. APS §6.5.
    WriteSingleCoil {
        /// Zero-based PDU address.
        address: u16,
        /// The state to write.
        on: bool,
    },
    /// Write Single Register. APS §6.6.
    WriteSingleRegister {
        /// Zero-based PDU address.
        address: u16,
        /// The value to write.
        value: u16,
    },
    /// Write Multiple Coils. APS §6.11.
    WriteMultipleCoils {
        /// Zero-based PDU address of the first coil.
        start: u16,
        /// The states to write, 1 to 1968 of them.
        values: Bits,
    },
    /// Write Multiple Registers. APS §6.12.
    WriteMultipleRegisters {
        /// Zero-based PDU address of the first register.
        start: u16,
        /// The values to write, 1 to 123 of them.
        values: Words,
    },
}

impl Request {
    /// The function code this request carries.
    #[must_use]
    pub const fn function(&self) -> FunctionCode {
        match self {
            Self::ReadCoils { .. } => FunctionCode::READ_COILS,
            Self::ReadDiscreteInputs { .. } => FunctionCode::READ_DISCRETE_INPUTS,
            Self::ReadHoldingRegisters { .. } => FunctionCode::READ_HOLDING_REGISTERS,
            Self::ReadInputRegisters { .. } => FunctionCode::READ_INPUT_REGISTERS,
            Self::WriteSingleCoil { .. } => FunctionCode::WRITE_SINGLE_COIL,
            Self::WriteSingleRegister { .. } => FunctionCode::WRITE_SINGLE_REGISTER,
            Self::WriteMultipleCoils { .. } => FunctionCode::WRITE_MULTIPLE_COILS,
            Self::WriteMultipleRegisters { .. } => FunctionCode::WRITE_MULTIPLE_REGISTERS,
        }
    }

    /// The first address this request touches.
    #[must_use]
    pub const fn start(&self) -> u16 {
        match self {
            Self::ReadCoils { start, .. }
            | Self::ReadDiscreteInputs { start, .. }
            | Self::ReadHoldingRegisters { start, .. }
            | Self::ReadInputRegisters { start, .. }
            | Self::WriteMultipleCoils { start, .. }
            | Self::WriteMultipleRegisters { start, .. } => *start,
            Self::WriteSingleCoil { address, .. } | Self::WriteSingleRegister { address, .. } => {
                *address
            }
        }
    }

    /// How many items this request touches.
    #[must_use]
    pub const fn quantity(&self) -> u16 {
        match self {
            Self::ReadCoils { quantity, .. }
            | Self::ReadDiscreteInputs { quantity, .. }
            | Self::ReadHoldingRegisters { quantity, .. }
            | Self::ReadInputRegisters { quantity, .. } => *quantity,
            Self::WriteSingleCoil { .. } | Self::WriteSingleRegister { .. } => 1,
            Self::WriteMultipleCoils { values, .. } => values.count(),
            Self::WriteMultipleRegisters { values, .. } => values.count(),
        }
    }

    /// Whether answering this request changes the device.
    ///
    /// The distinction that decides whether salman may issue it at all: a read
    /// is permitted at every posture, and a write to a real device needs an
    /// armed posture and a human's confirmation of that specific call. See
    /// `salman_core::posture`.
    #[must_use]
    pub const fn is_write(&self) -> bool {
        match self {
            Self::ReadCoils { .. }
            | Self::ReadDiscreteInputs { .. }
            | Self::ReadHoldingRegisters { .. }
            | Self::ReadInputRegisters { .. } => false,
            Self::WriteSingleCoil { .. }
            | Self::WriteSingleRegister { .. }
            | Self::WriteMultipleCoils { .. }
            | Self::WriteMultipleRegisters { .. } => true,
        }
    }

    /// Writes the request as a PDU.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] if the request carries more items than the
    /// function permits. That is possible because [`Bits`] and [`Words`] are
    /// sized for the largest **read**, and every write has a smaller limit —
    /// 123 registers against 125, and 1968 coils against 2000. A request built
    /// by hand can therefore hold more than one frame can carry, and the
    /// alternative to refusing here is a PDU whose declared byte count is
    /// larger than the data behind it: a frame that is a lie, which every
    /// reader resolves differently.
    pub fn encode(&self) -> Result<Pdu, EncodeError> {
        self.check_encodable()?;
        let mut pdu = Pdu::new(self.function());
        match self {
            Self::ReadCoils { start, quantity }
            | Self::ReadDiscreteInputs { start, quantity }
            | Self::ReadHoldingRegisters { start, quantity }
            | Self::ReadInputRegisters { start, quantity } => {
                pdu.push_u16(*start);
                pdu.push_u16(*quantity);
            }
            Self::WriteSingleCoil { address, on } => {
                pdu.push_u16(*address);
                pdu.push_u16(if *on { COIL_ON } else { COIL_OFF });
            }
            Self::WriteSingleRegister { address, value } => {
                pdu.push_u16(*address);
                pdu.push_u16(*value);
            }
            Self::WriteMultipleCoils { start, values } => {
                pdu.push_u16(*start);
                pdu.push_u16(values.count());
                pdu.push_u8(values.packed().len() as u8);
                pdu.push_bytes(values.packed());
            }
            Self::WriteMultipleRegisters { start, values } => {
                pdu.push_u16(*start);
                pdu.push_u16(values.count());
                pdu.push_u8((values.count() * 2) as u8);
                for value in values.values() {
                    pdu.push_u16(*value);
                }
            }
        }
        if pdu.overflowed() {
            // Cannot be reached once `check_encodable` has passed, and checked
            // anyway: a silent overflow here is the exact failure this method
            // exists to prevent, so it is not left to an argument about
            // reachability.
            return Err(EncodeError::TooLongForAFrame {
                function: self.function(),
            });
        }
        Ok(pdu)
    }

    /// Whether this request fits the limits its function gives.
    ///
    /// Every function, not only the ones whose payload could outgrow a frame.
    /// A `ReadCoils` asking for zero coils encodes to five bytes and is
    /// perfectly shaped, and salman's own decoder refuses it — so writing one
    /// would mean salman putting a frame on the wire that it would not accept
    /// back. The two sets of limits have to be the same set.
    fn check_encodable(&self) -> Result<(), EncodeError> {
        let (quantity, max) = match self {
            Self::ReadCoils { quantity, .. } | Self::ReadDiscreteInputs { quantity, .. } => {
                (*quantity, MAX_READ_BITS)
            }
            Self::ReadHoldingRegisters { quantity, .. }
            | Self::ReadInputRegisters { quantity, .. } => (*quantity, MAX_READ_REGISTERS),
            Self::WriteMultipleCoils { values, .. } => (values.count(), MAX_WRITE_BITS),
            Self::WriteMultipleRegisters { values, .. } => (values.count(), MAX_WRITE_REGISTERS),
            // A single write carries one item and a pair of 16-bit fields,
            // which no quantity governs.
            Self::WriteSingleCoil { .. } | Self::WriteSingleRegister { .. } => return Ok(()),
        };
        if quantity < 1 || quantity > max {
            return Err(EncodeError::QuantityOutOfRange {
                function: self.function(),
                quantity,
                min: 1,
                max,
            });
        }
        Ok(())
    }

    /// Reads a request from PDU bytes.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`] naming what was wrong. Every structural rule
    /// APS states for these function codes is checked here; whether the
    /// address exists on a particular device is not a decoding question and is
    /// decided by the server.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut reader = Reader::new(bytes)?;
        let function = reader.function;
        let request = match function {
            FunctionCode::READ_COILS | FunctionCode::READ_DISCRETE_INPUTS => {
                let start = reader.u16()?;
                let quantity = reader.u16()?;
                check_quantity(quantity, 1, MAX_READ_BITS)?;
                if function == FunctionCode::READ_COILS {
                    Self::ReadCoils { start, quantity }
                } else {
                    Self::ReadDiscreteInputs { start, quantity }
                }
            }
            FunctionCode::READ_HOLDING_REGISTERS | FunctionCode::READ_INPUT_REGISTERS => {
                let start = reader.u16()?;
                let quantity = reader.u16()?;
                check_quantity(quantity, 1, MAX_READ_REGISTERS)?;
                if function == FunctionCode::READ_HOLDING_REGISTERS {
                    Self::ReadHoldingRegisters { start, quantity }
                } else {
                    Self::ReadInputRegisters { start, quantity }
                }
            }
            FunctionCode::WRITE_SINGLE_COIL => {
                let address = reader.u16()?;
                let raw = reader.u16()?;
                // APS §6.5 gives this field exactly two legal values. A device
                // that treats anything non-zero as "on" is guessing, and the
                // guess differs between devices.
                let on = match raw {
                    COIL_ON => true,
                    COIL_OFF => false,
                    other => return Err(DecodeError::CoilValueNotOnOrOff { value: other }),
                };
                Self::WriteSingleCoil { address, on }
            }
            FunctionCode::WRITE_SINGLE_REGISTER => Self::WriteSingleRegister {
                address: reader.u16()?,
                value: reader.u16()?,
            },
            FunctionCode::WRITE_MULTIPLE_COILS => {
                let start = reader.u16()?;
                let quantity = reader.u16()?;
                check_quantity(quantity, 1, MAX_WRITE_BITS)?;
                let declared = usize::from(reader.u8()?);
                let expected = packed_bytes(quantity);
                if declared != expected {
                    return Err(DecodeError::ByteCountDisagreesWithQuantity {
                        declared,
                        expected,
                        quantity,
                    });
                }
                let data = reader.bytes(declared)?;
                let values =
                    Bits::from_packed(data, quantity).ok_or(DecodeError::QuantityOutOfRange {
                        quantity,
                        min: 1,
                        max: MAX_WRITE_BITS,
                    })?;
                Self::WriteMultipleCoils { start, values }
            }
            FunctionCode::WRITE_MULTIPLE_REGISTERS => {
                let start = reader.u16()?;
                let quantity = reader.u16()?;
                check_quantity(quantity, 1, MAX_WRITE_REGISTERS)?;
                let declared = usize::from(reader.u8()?);
                let expected = usize::from(quantity) * 2;
                if declared != expected {
                    return Err(DecodeError::ByteCountDisagreesWithQuantity {
                        declared,
                        expected,
                        quantity,
                    });
                }
                let mut values = [0_u16; MAX_WRITE_REGISTERS as usize];
                for index in 0..usize::from(quantity) {
                    let value = reader.u16()?;
                    if let Some(slot) = values.get_mut(index) {
                        *slot = value;
                    }
                }
                let values = Words::new(values.get(..usize::from(quantity)).unwrap_or(&[])).ok_or(
                    DecodeError::QuantityOutOfRange {
                        quantity,
                        min: 1,
                        max: MAX_WRITE_REGISTERS,
                    },
                )?;
                Self::WriteMultipleRegisters { start, values }
            }
            other => {
                return Err(if other.name().is_some() {
                    DecodeError::FunctionNotImplemented { function: other }
                } else {
                    DecodeError::FunctionUnknown { function: other }
                });
            }
        };
        reader.finish()?;
        Ok(request)
    }
}

/// A response, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// The coils that were read.
    ReadCoils(Bits),
    /// The discrete inputs that were read.
    ReadDiscreteInputs(Bits),
    /// The holding registers that were read.
    ReadHoldingRegisters(Words),
    /// The input registers that were read.
    ReadInputRegisters(Words),
    /// Write Single Coil echoes its request. APS §6.5.
    WriteSingleCoil {
        /// The address written.
        address: u16,
        /// The state written.
        on: bool,
    },
    /// Write Single Register echoes its request. APS §6.6.
    WriteSingleRegister {
        /// The address written.
        address: u16,
        /// The value written.
        value: u16,
    },
    /// Write Multiple Coils answers with the address and quantity. APS §6.11.
    WriteMultipleCoils {
        /// The first address written.
        start: u16,
        /// How many were written.
        quantity: u16,
    },
    /// Write Multiple Registers answers with the address and quantity.
    /// APS §6.12.
    WriteMultipleRegisters {
        /// The first address written.
        start: u16,
        /// How many were written.
        quantity: u16,
    },
    /// The server refused, and said why. APS §7.
    Exception {
        /// The function code the request carried, without the exception bit.
        function: FunctionCode,
        /// Why the server refused.
        code: ExceptionCode,
    },
}

impl Response {
    /// Writes the response as a PDU.
    ///
    /// Infallible, unlike [`Request::encode`]. Every response payload is
    /// sized by a **read** limit, and those are exactly the quantities a frame
    /// holds — so there is no response this type can represent that does not
    /// fit.
    #[must_use]
    pub fn encode(&self) -> Pdu {
        match self {
            Self::ReadCoils(bits) | Self::ReadDiscreteInputs(bits) => {
                let function = if matches!(self, Self::ReadCoils(_)) {
                    FunctionCode::READ_COILS
                } else {
                    FunctionCode::READ_DISCRETE_INPUTS
                };
                let mut pdu = Pdu::new(function);
                pdu.push_u8(bits.packed().len() as u8);
                pdu.push_bytes(bits.packed());
                pdu
            }
            Self::ReadHoldingRegisters(words) | Self::ReadInputRegisters(words) => {
                let function = if matches!(self, Self::ReadHoldingRegisters(_)) {
                    FunctionCode::READ_HOLDING_REGISTERS
                } else {
                    FunctionCode::READ_INPUT_REGISTERS
                };
                let mut pdu = Pdu::new(function);
                pdu.push_u8((words.count() * 2) as u8);
                for value in words.values() {
                    pdu.push_u16(*value);
                }
                pdu
            }
            Self::WriteSingleCoil { address, on } => {
                let mut pdu = Pdu::new(FunctionCode::WRITE_SINGLE_COIL);
                pdu.push_u16(*address);
                pdu.push_u16(if *on { COIL_ON } else { COIL_OFF });
                pdu
            }
            Self::WriteSingleRegister { address, value } => {
                let mut pdu = Pdu::new(FunctionCode::WRITE_SINGLE_REGISTER);
                pdu.push_u16(*address);
                pdu.push_u16(*value);
                pdu
            }
            Self::WriteMultipleCoils { start, quantity } => {
                let mut pdu = Pdu::new(FunctionCode::WRITE_MULTIPLE_COILS);
                pdu.push_u16(*start);
                pdu.push_u16(*quantity);
                pdu
            }
            Self::WriteMultipleRegisters { start, quantity } => {
                let mut pdu = Pdu::new(FunctionCode::WRITE_MULTIPLE_REGISTERS);
                pdu.push_u16(*start);
                pdu.push_u16(*quantity);
                pdu
            }
            Self::Exception { function, code } => {
                let mut pdu = Pdu::new(function.as_exception());
                pdu.push_u8(code.0);
                pdu
            }
        }
    }

    /// Reads a response to `request` from PDU bytes.
    ///
    /// The request is needed, not merely convenient: a read response carries a
    /// byte count and never the quantity, so five coils and eight coils are
    /// byte-identical on the wire. See the module documentation.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError`]. A response whose function code is neither the
    /// request's nor its exception form is
    /// [`DecodeError::FunctionDoesNotAnswerRequest`] — which is a real fault,
    /// not a decoding inconvenience: it means the answer belongs to a
    /// different question.
    pub fn decode(bytes: &[u8], request: &Request) -> Result<Self, DecodeError> {
        let mut reader = Reader::new(bytes)?;
        let expected = request.function();
        if reader.function == expected.as_exception() {
            let code = ExceptionCode(reader.u8()?);
            reader.finish()?;
            return Ok(Self::Exception {
                function: expected,
                code,
            });
        }
        if reader.function != expected {
            return Err(DecodeError::FunctionDoesNotAnswerRequest {
                expected,
                found: reader.function,
            });
        }

        let response = match request {
            Request::ReadCoils { quantity, .. } | Request::ReadDiscreteInputs { quantity, .. } => {
                let bits = reader.read_bits(*quantity)?;
                if matches!(request, Request::ReadCoils { .. }) {
                    Self::ReadCoils(bits)
                } else {
                    Self::ReadDiscreteInputs(bits)
                }
            }
            Request::ReadHoldingRegisters { quantity, .. }
            | Request::ReadInputRegisters { quantity, .. } => {
                let words = reader.read_words(*quantity)?;
                if matches!(request, Request::ReadHoldingRegisters { .. }) {
                    Self::ReadHoldingRegisters(words)
                } else {
                    Self::ReadInputRegisters(words)
                }
            }
            Request::WriteSingleCoil { .. } => {
                let address = reader.u16()?;
                let raw = reader.u16()?;
                let on = match raw {
                    COIL_ON => true,
                    COIL_OFF => false,
                    other => return Err(DecodeError::CoilValueNotOnOrOff { value: other }),
                };
                Self::WriteSingleCoil { address, on }
            }
            Request::WriteSingleRegister { .. } => Self::WriteSingleRegister {
                address: reader.u16()?,
                value: reader.u16()?,
            },
            Request::WriteMultipleCoils { .. } => Self::WriteMultipleCoils {
                start: reader.u16()?,
                quantity: reader.u16()?,
            },
            Request::WriteMultipleRegisters { .. } => Self::WriteMultipleRegisters {
                start: reader.u16()?,
                quantity: reader.u16()?,
            },
        };
        reader.finish()?;
        Ok(response)
    }
}

/// The bytes of one protocol data unit.
///
/// Fixed capacity, so building a PDU allocates nothing and cannot outgrow what
/// a frame may carry.
#[derive(Clone, Copy)]
pub struct Pdu {
    bytes: [u8; MAX_PDU],
    len: u16,
    /// Set if anything was written past the end.
    ///
    /// A buffer that quietly stopped accepting bytes would produce a frame
    /// whose declared byte count did not match the data behind it — a lie on
    /// the wire that every reader would resolve differently. Nothing may hand
    /// out such a PDU, so the fact is recorded and [`Request::encode`] turns
    /// it into a refusal.
    overflowed: bool,
}

impl Pdu {
    /// An empty PDU carrying only `function`.
    #[must_use]
    pub const fn new(function: FunctionCode) -> Self {
        let mut bytes = [0; MAX_PDU];
        bytes[0] = function.0;
        Self {
            bytes,
            len: 1,
            overflowed: false,
        }
    }

    /// Takes PDU bytes as they arrived, without interpreting them.
    ///
    /// Returns `None` for an empty slice, which carries no function code, and
    /// for one longer than [`MAX_PDU`].
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX_PDU {
            return None;
        }
        let mut pdu = Self {
            bytes: [0; MAX_PDU],
            len: bytes.len() as u16,
            overflowed: false,
        };
        pdu.bytes.get_mut(..bytes.len())?.copy_from_slice(bytes);
        Some(pdu)
    }

    /// The bytes, as they go on the wire.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len as usize).unwrap_or(&[])
    }

    /// How many bytes it holds.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether it holds nothing. A PDU always holds at least its function
    /// code, so this is only ever true of a `Pdu` that was never built.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The function code.
    #[must_use]
    pub const fn function(&self) -> FunctionCode {
        FunctionCode(self.bytes[0])
    }

    /// Whether anything was written past the end of this PDU.
    ///
    /// A PDU that overflowed is not a frame and must never be sent.
    #[must_use]
    pub const fn overflowed(&self) -> bool {
        self.overflowed
    }

    fn push_u8(&mut self, value: u8) {
        match self.bytes.get_mut(self.len as usize) {
            Some(slot) => {
                *slot = value;
                self.len += 1;
            }
            None => self.overflowed = true,
        }
    }

    fn push_u16(&mut self, value: u16) {
        // Every multi-byte field in Modbus is big-endian. APS §4.2. The RTU
        // CRC is the one exception, and it is not part of the PDU.
        for byte in value.to_be_bytes() {
            self.push_u8(byte);
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.push_u8(*byte);
        }
    }
}

impl fmt::Debug for Pdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pdu({:02X?})", self.as_bytes())
    }
}

impl PartialEq for Pdu {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Pdu {}

/// Why a request could not be written as a frame.
///
/// Decoding has many ways to fail and encoding has one shape of failure: the
/// value holds more than the function permits. It has its own type rather than
/// sharing [`DecodeError`] because a caller handles the two in entirely
/// different places — one is about bytes that arrived, the other about a value
/// this program built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeError {
    /// The request carries a quantity outside what the function permits.
    QuantityOutOfRange {
        /// Which function.
        function: FunctionCode,
        /// How many items the request holds.
        quantity: u16,
        /// The smallest the function permits.
        min: u16,
        /// The largest the function permits.
        max: u16,
    },
    /// The encoded form ran past the end of a protocol data unit.
    TooLongForAFrame {
        /// Which function.
        function: FunctionCode,
    },
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuantityOutOfRange {
                function,
                quantity,
                min,
                max,
            } => write!(
                f,
                "{function} carries {quantity} items and permits {min} to {max}"
            ),
            Self::TooLongForAFrame { function } => write!(
                f,
                "a {function} of this size does not fit in the {MAX_PDU} bytes a protocol \
                 data unit may hold"
            ),
        }
    }
}

impl core::error::Error for EncodeError {}

/// Why a PDU could not be decoded.
///
/// Every variant names what salman expected and what it found, because a
/// decoder that says only "malformed" leaves the reader to guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// There were no bytes at all, so there is not even a function code.
    Empty,
    /// The PDU is longer than any Modbus frame may carry.
    TooLong {
        /// How many bytes arrived.
        length: usize,
    },
    /// The PDU ended in the middle of a field.
    Truncated {
        /// How many more bytes the field needed.
        needed: usize,
        /// How many were left.
        available: usize,
    },
    /// There were bytes after the last field the function defines.
    TrailingBytes {
        /// How many were left over.
        extra: usize,
    },
    /// A quantity was outside the range APS gives for the function.
    QuantityOutOfRange {
        /// What the frame asked for.
        quantity: u16,
        /// The smallest APS permits.
        min: u16,
        /// The largest APS permits.
        max: u16,
    },
    /// The declared byte count is not what the quantity implies.
    ByteCountDisagreesWithQuantity {
        /// What the frame declared.
        declared: usize,
        /// What the quantity implies.
        expected: usize,
        /// The quantity the frame carried.
        quantity: u16,
    },
    /// Write Single Coil carried a value other than `0xFF00` or `0x0000`.
    CoilValueNotOnOrOff {
        /// The value found.
        value: u16,
    },
    /// The function code is one APS names and salman does not implement.
    FunctionNotImplemented {
        /// The code found.
        function: FunctionCode,
    },
    /// The function code is not one APS names.
    FunctionUnknown {
        /// The code found.
        function: FunctionCode,
    },
    /// The response's function code answers a different request.
    FunctionDoesNotAnswerRequest {
        /// The code the request carried.
        expected: FunctionCode,
        /// The code the response carried.
        found: FunctionCode,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the frame carries no function code"),
            Self::TooLong { length } => write!(
                f,
                "the frame is {length} bytes and a protocol data unit may be at most {MAX_PDU}"
            ),
            Self::Truncated { needed, available } => write!(
                f,
                "a field needs {needed} more bytes and only {available} are left"
            ),
            Self::TrailingBytes { extra } => {
                write!(f, "{extra} bytes follow the end of the frame")
            }
            Self::QuantityOutOfRange { quantity, min, max } => write!(
                f,
                "a quantity of {quantity} is outside the {min} to {max} this function permits"
            ),
            Self::ByteCountDisagreesWithQuantity {
                declared,
                expected,
                quantity,
            } => write!(
                f,
                "the byte count is {declared} and a quantity of {quantity} needs {expected}"
            ),
            Self::CoilValueNotOnOrOff { value } => write!(
                f,
                "a coil value of 0x{value:04X} is neither 0xFF00 nor 0x0000"
            ),
            Self::FunctionNotImplemented { function } => {
                write!(f, "salman does not implement {function}")
            }
            Self::FunctionUnknown { function } => {
                write!(
                    f,
                    "{function} is not a function code this specification names"
                )
            }
            Self::FunctionDoesNotAnswerRequest { expected, found } => write!(
                f,
                "the response carries {found} and the request carried {expected}"
            ),
        }
    }
}

impl core::error::Error for DecodeError {}

/// Checks a quantity against the range APS gives for its function.
fn check_quantity(quantity: u16, min: u16, max: u16) -> Result<(), DecodeError> {
    if quantity < min || quantity > max {
        return Err(DecodeError::QuantityOutOfRange { quantity, min, max });
    }
    Ok(())
}

/// Walks a PDU's bytes, refusing to read past the end.
struct Reader<'a> {
    function: FunctionCode,
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, DecodeError> {
        if bytes.len() > MAX_PDU {
            return Err(DecodeError::TooLong {
                length: bytes.len(),
            });
        }
        let (function, rest) = bytes.split_first().ok_or(DecodeError::Empty)?;
        Ok(Self {
            function: FunctionCode(*function),
            rest,
        })
    }

    fn bytes(&mut self, count: usize) -> Result<&'a [u8], DecodeError> {
        if self.rest.len() < count {
            return Err(DecodeError::Truncated {
                needed: count,
                available: self.rest.len(),
            });
        }
        let (taken, rest) = self.rest.split_at(count);
        self.rest = rest;
        Ok(taken)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        self.bytes(1)?
            .first()
            .copied()
            .ok_or(DecodeError::Truncated {
                needed: 1,
                available: 0,
            })
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.bytes(2)?;
        match bytes {
            [high, low] => Ok(u16::from_be_bytes([*high, *low])),
            _ => Err(DecodeError::Truncated {
                needed: 2,
                available: bytes.len(),
            }),
        }
    }

    /// Reads a byte count and that many packed bits.
    fn read_bits(&mut self, quantity: u16) -> Result<Bits, DecodeError> {
        let declared = usize::from(self.u8()?);
        let expected = packed_bytes(quantity);
        if declared != expected {
            return Err(DecodeError::ByteCountDisagreesWithQuantity {
                declared,
                expected,
                quantity,
            });
        }
        let data = self.bytes(declared)?;
        Bits::from_packed(data, quantity).ok_or(DecodeError::QuantityOutOfRange {
            quantity,
            min: 1,
            max: MAX_READ_BITS,
        })
    }

    /// Reads a byte count and that many registers.
    fn read_words(&mut self, quantity: u16) -> Result<Words, DecodeError> {
        let declared = usize::from(self.u8()?);
        let expected = usize::from(quantity) * 2;
        if declared != expected {
            return Err(DecodeError::ByteCountDisagreesWithQuantity {
                declared,
                expected,
                quantity,
            });
        }
        let mut values = [0_u16; MAX_READ_REGISTERS as usize];
        for index in 0..usize::from(quantity) {
            let value = self.u16()?;
            if let Some(slot) = values.get_mut(index) {
                *slot = value;
            }
        }
        Words::new(values.get(..usize::from(quantity)).unwrap_or(&[])).ok_or(
            DecodeError::QuantityOutOfRange {
                quantity,
                min: 1,
                max: MAX_READ_REGISTERS,
            },
        )
    }

    /// Fails if anything is left. A frame with trailing bytes is not a frame
    /// salman understood; accepting it would hide a framing fault.
    fn finish(self) -> Result<(), DecodeError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes {
                extra: self.rest.len(),
            })
        }
    }
}
