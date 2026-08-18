// SPDX-License-Identifier: Apache-2.0
//! A Modbus server's data model, and what it does with a request.
//!
//! Still no transport: this decides what the *answer* is, and something else
//! carries it. That separation is what lets the whole of a server's behaviour
//! — including the parts that are hard to provoke on real equipment, like an
//! address one past the end of a map — be tested without a socket.
//!
//! # Four tables
//!
//! APS §4.3 gives a server four tables: discrete inputs (one bit, read only),
//! coils (one bit, read and write), input registers (16 bits, read only) and
//! holding registers (16 bits, read and write). The specification permits them
//! to overlay one another and says the mapping onto a device's application is
//! "totally vendor device specific".
//!
//! salman models each table as a **declared range** rather than a full 65536
//! items. That is not a simplification for its own sake: a real device answers
//! exception 02 for an address outside its map, and a model that always
//! succeeded could never produce the single most common Modbus error.
//!
//! # The order the checks run in
//!
//! APS disagrees with itself here. Figure 9 orders the address check (02)
//! before the value check (03); every per-function figure in §6 orders the
//! value check first. salman follows the per-function order — function code,
//! then quantity and value, then address, then execution — and records the
//! choice as **salman's**, because the specification does not settle it.

use crate::function::ExceptionCode;
use crate::limits::{MAX_READ_BITS, MAX_READ_REGISTERS, MAX_WRITE_BITS, MAX_WRITE_REGISTERS};
use crate::pdu::{Bits, Request, Response, Words};

/// Which of the four tables an address refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Table {
    /// One bit, read only. APS §4.3.
    DiscreteInputs,
    /// One bit, read and write. APS §4.3.
    Coils,
    /// Sixteen bits, read only. APS §4.3.
    InputRegisters,
    /// Sixteen bits, read and write. APS §4.3.
    HoldingRegisters,
}

impl Table {
    /// The table a bit table refers to.
    #[must_use]
    pub const fn of_bits(table: BitTable) -> Self {
        match table {
            BitTable::DiscreteInputs => Self::DiscreteInputs,
            BitTable::Coils => Self::Coils,
        }
    }

    /// The table a register table refers to.
    #[must_use]
    pub const fn of_words(table: WordTable) -> Self {
        match table {
            WordTable::InputRegisters => Self::InputRegisters,
            WordTable::HoldingRegisters => Self::HoldingRegisters,
        }
    }

    /// How the table is named in APS.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::DiscreteInputs => "discrete inputs",
            Self::Coils => "coils",
            Self::InputRegisters => "input registers",
            Self::HoldingRegisters => "holding registers",
        }
    }

    /// Whether a Modbus request may write to it.
    ///
    /// A read-only table is read-only to the *network*. The process behind the
    /// device writes input registers all the time; that is what makes them
    /// inputs.
    #[must_use]
    pub const fn is_writable_over_the_network(self) -> bool {
        matches!(self, Self::Coils | Self::HoldingRegisters)
    }
}

/// One of the two tables addressed one bit at a time.
///
/// Separate from [`Table`] so that asking for a bit from a register table is
/// not a thing that can be written. The alternative — one enum and a runtime
/// answer of "no" — makes a caller mistake into a silent no-op, which is the
/// shape of bug this crate exists to avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BitTable {
    /// One bit, read only over the network. APS §4.3.
    DiscreteInputs,
    /// One bit, read and write. APS §4.3.
    Coils,
}

/// One of the two tables addressed sixteen bits at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WordTable {
    /// Sixteen bits, read only over the network. APS §4.3.
    InputRegisters,
    /// Sixteen bits, read and write. APS §4.3.
    HoldingRegisters,
}

/// A contiguous run of bits a device answers for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BitBlock {
    start: u16,
    values: Vec<bool>,
}

/// A contiguous run of registers a device answers for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WordBlock {
    start: u16,
    values: Vec<u16>,
}

/// Whether `start` and `count` fall inside a block starting at `base` of
/// `len` items, and where in it they start.
///
/// The check is on `start + count`, not on `start` alone: APS §7 defines
/// exception 02 as the *combination* of reference number and transfer length
/// being invalid, and a read of four registers from 96 in a map of 100 is
/// legal where a read of five is not.
fn offset_within(base: u16, len: usize, start: u16, count: u16) -> Option<usize> {
    let offset = usize::from(start).checked_sub(usize::from(base))?;
    let end = offset.checked_add(usize::from(count))?;
    if end > len { None } else { Some(offset) }
}

/// A Modbus server's data, and the rules for answering a request about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    discrete_inputs: BitBlock,
    coils: BitBlock,
    input_registers: WordBlock,
    holding_registers: WordBlock,
}

impl Device {
    /// A device with nothing mapped, which answers exception 02 to everything.
    ///
    /// A real device with an empty map exists — it is a device whose
    /// configuration has not been loaded — and being able to simulate one is
    /// the point.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            discrete_inputs: BitBlock {
                start: 0,
                values: Vec::new(),
            },
            coils: BitBlock {
                start: 0,
                values: Vec::new(),
            },
            input_registers: WordBlock {
                start: 0,
                values: Vec::new(),
            },
            holding_registers: WordBlock {
                start: 0,
                values: Vec::new(),
            },
        }
    }

    /// Declares a run of bits, all off.
    ///
    /// `count` is clamped to what remains below `0x10000`, because an address
    /// cannot exceed sixteen bits and a map that wrapped would answer for
    /// addresses it does not hold.
    #[must_use]
    pub fn with_bits(mut self, table: BitTable, start: u16, count: u16) -> Self {
        let len = usize::from(count).min(0x1_0000 - usize::from(start));
        let block = BitBlock {
            start,
            values: vec![false; len],
        };
        *self.bit_block_mut(table) = block;
        self
    }

    /// Declares a run of registers, all zero.
    #[must_use]
    pub fn with_registers(mut self, table: WordTable, start: u16, count: u16) -> Self {
        let len = usize::from(count).min(0x1_0000 - usize::from(start));
        let block = WordBlock {
            start,
            values: vec![0; len],
        };
        *self.word_block_mut(table) = block;
        self
    }

    fn bit_block(&self, table: BitTable) -> &BitBlock {
        match table {
            BitTable::DiscreteInputs => &self.discrete_inputs,
            BitTable::Coils => &self.coils,
        }
    }

    fn bit_block_mut(&mut self, table: BitTable) -> &mut BitBlock {
        match table {
            BitTable::DiscreteInputs => &mut self.discrete_inputs,
            BitTable::Coils => &mut self.coils,
        }
    }

    fn word_block(&self, table: WordTable) -> &WordBlock {
        match table {
            WordTable::InputRegisters => &self.input_registers,
            WordTable::HoldingRegisters => &self.holding_registers,
        }
    }

    fn word_block_mut(&mut self, table: WordTable) -> &mut WordBlock {
        match table {
            WordTable::InputRegisters => &mut self.input_registers,
            WordTable::HoldingRegisters => &mut self.holding_registers,
        }
    }

    /// Reads one bit, or `None` if the address is outside the map.
    #[must_use]
    pub fn bit(&self, table: BitTable, address: u16) -> Option<bool> {
        let block = self.bit_block(table);
        let offset = offset_within(block.start, block.values.len(), address, 1)?;
        block.values.get(offset).copied()
    }

    /// Reads one register, or `None` if the address is outside the map.
    #[must_use]
    pub fn register(&self, table: WordTable, address: u16) -> Option<u16> {
        let block = self.word_block(table);
        let offset = offset_within(block.start, block.values.len(), address, 1)?;
        block.values.get(offset).copied()
    }

    /// Sets one bit from the process side, ignoring whether the network may
    /// write it. Returns `false` if the address is outside the map.
    ///
    /// This is how a simulation drives a discrete input: the device's own
    /// process writes it, and the network only reads it.
    pub fn set_bit(&mut self, table: BitTable, address: u16, value: bool) -> bool {
        let block = self.bit_block_mut(table);
        let Some(offset) = offset_within(block.start, block.values.len(), address, 1) else {
            return false;
        };
        match block.values.get_mut(offset) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
    }

    /// Sets one register from the process side. Returns `false` if the address
    /// is outside the map.
    pub fn set_register(&mut self, table: WordTable, address: u16, value: u16) -> bool {
        let block = self.word_block_mut(table);
        let Some(offset) = offset_within(block.start, block.values.len(), address, 1) else {
            return false;
        };
        match block.values.get_mut(offset) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
    }

    /// Answers a request.
    ///
    /// # Errors
    ///
    /// Returns the [`ExceptionCode`] a conforming server would send. The
    /// checks run in the order recorded in this module's documentation:
    /// quantity and value before address, which is the order every
    /// per-function figure in APS §6 uses, and which contradicts Figure 9.
    /// That contradiction is the specification's; the choice between them is
    /// salman's.
    pub fn apply(&mut self, request: &Request) -> Result<Response, ExceptionCode> {
        match request {
            Request::ReadCoils { start, quantity } => self
                .read_bits(BitTable::Coils, *start, *quantity)
                .map(Response::ReadCoils),
            Request::ReadDiscreteInputs { start, quantity } => self
                .read_bits(BitTable::DiscreteInputs, *start, *quantity)
                .map(Response::ReadDiscreteInputs),
            Request::ReadHoldingRegisters { start, quantity } => self
                .read_registers(WordTable::HoldingRegisters, *start, *quantity)
                .map(Response::ReadHoldingRegisters),
            Request::ReadInputRegisters { start, quantity } => self
                .read_registers(WordTable::InputRegisters, *start, *quantity)
                .map(Response::ReadInputRegisters),
            Request::WriteSingleCoil { address, on } => {
                if !self.set_bit(BitTable::Coils, *address, *on) {
                    return Err(ExceptionCode::ILLEGAL_DATA_ADDRESS);
                }
                Ok(Response::WriteSingleCoil {
                    address: *address,
                    on: *on,
                })
            }
            Request::WriteSingleRegister { address, value } => {
                if !self.set_register(WordTable::HoldingRegisters, *address, *value) {
                    return Err(ExceptionCode::ILLEGAL_DATA_ADDRESS);
                }
                Ok(Response::WriteSingleRegister {
                    address: *address,
                    value: *value,
                })
            }
            Request::WriteMultipleCoils { start, values } => {
                let quantity = values.count();
                check_range(quantity, MAX_WRITE_BITS)?;
                let offset =
                    offset_within(self.coils.start, self.coils.values.len(), *start, quantity)
                        .ok_or(ExceptionCode::ILLEGAL_DATA_ADDRESS)?;
                // Every address is known good before the first byte moves. A
                // multi-write that failed halfway would leave the device in a
                // state the client has no way to learn about.
                for index in 0..quantity {
                    if let (Some(slot), Some(value)) = (
                        self.coils.values.get_mut(offset + usize::from(index)),
                        values.get(index),
                    ) {
                        *slot = value;
                    }
                }
                Ok(Response::WriteMultipleCoils {
                    start: *start,
                    quantity,
                })
            }
            Request::WriteMultipleRegisters { start, values } => {
                let quantity = values.count();
                check_range(quantity, MAX_WRITE_REGISTERS)?;
                let offset = offset_within(
                    self.holding_registers.start,
                    self.holding_registers.values.len(),
                    *start,
                    quantity,
                )
                .ok_or(ExceptionCode::ILLEGAL_DATA_ADDRESS)?;
                for index in 0..quantity {
                    if let (Some(slot), Some(value)) = (
                        self.holding_registers
                            .values
                            .get_mut(offset + usize::from(index)),
                        values.get(index),
                    ) {
                        *slot = value;
                    }
                }
                Ok(Response::WriteMultipleRegisters {
                    start: *start,
                    quantity,
                })
            }
        }
    }

    fn read_bits(&self, table: BitTable, start: u16, quantity: u16) -> Result<Bits, ExceptionCode> {
        check_range(quantity, MAX_READ_BITS)?;
        let block = self.bit_block(table);
        let offset = offset_within(block.start, block.values.len(), start, quantity)
            .ok_or(ExceptionCode::ILLEGAL_DATA_ADDRESS)?;
        let taken = block
            .values
            .get(offset..offset + usize::from(quantity))
            .ok_or(ExceptionCode::ILLEGAL_DATA_ADDRESS)?;
        Bits::from_iter_of(taken.iter().copied()).ok_or(ExceptionCode::SERVER_DEVICE_FAILURE)
    }

    fn read_registers(
        &self,
        table: WordTable,
        start: u16,
        quantity: u16,
    ) -> Result<Words, ExceptionCode> {
        check_range(quantity, MAX_READ_REGISTERS)?;
        let block = self.word_block(table);
        let offset = offset_within(block.start, block.values.len(), start, quantity)
            .ok_or(ExceptionCode::ILLEGAL_DATA_ADDRESS)?;
        let taken = block
            .values
            .get(offset..offset + usize::from(quantity))
            .ok_or(ExceptionCode::ILLEGAL_DATA_ADDRESS)?;
        Words::new(taken).ok_or(ExceptionCode::SERVER_DEVICE_FAILURE)
    }
}

/// Checks a quantity before any address is looked at.
///
/// The order is salman's decision: APS Figure 9 puts the address check first
/// and every per-function figure in §6 puts this one first. See the module
/// documentation.
fn check_range(quantity: u16, max: u16) -> Result<(), ExceptionCode> {
    if quantity == 0 || quantity > max {
        return Err(ExceptionCode::ILLEGAL_DATA_VALUE);
    }
    Ok(())
}
