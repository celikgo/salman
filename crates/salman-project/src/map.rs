// SPDX-License-Identifier: Apache-2.0
//! Binding a device's registers to the process image.
//!
//! A mapping is four things: a table on the device, a run of addresses in it, a
//! place in the process image, and — implied by that place — which way the data
//! moves. Everything this module does is decide whether a mapping means
//! anything, and say precisely why when it does not.
//!
//! # What is checked, and why each one is worth checking
//!
//! * **The widths must agree.** Eight coils are eight bits and go to `%IX`;
//!   four registers are four words and go to `%IW`. A file that mapped coils to
//!   `%IW0` is asking for a bit run to appear as words, and there is no
//!   answer to that which is not an invention.
//! * **The direction must be possible.** `%Q` means salman writes to the
//!   device, and there is no Modbus function that writes a discrete input or an
//!   input register. A mapping that asked for one would fail on the first scan,
//!   against a live plant, rather than when the file was read.
//! * **Two mappings may not overlap in the image.** Whichever ran second would
//!   win, and which one that is would depend on the order they were written in
//!   the file.
//! * **The image range must exist.** A mapping that runs off the end of the
//!   image is a mapping whose last few values go nowhere.

use core::fmt;

use salman_lang::address::{AddressLocation, AddressSize, DirectAddress};
use salman_modbus::device::Table;

/// Which way data moves, and therefore what salman does each scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// Read from the device and present at `%I`, before the program runs.
    Input,
    /// Take from `%Q` and write to the device, after the program has run.
    Output,
}

impl Direction {
    /// The direction an image area implies.
    ///
    /// `%M` has no direction: it is the program's own memory, and a device
    /// mapped onto it would be neither read nor written at a defined point in
    /// the scan. Refusing is better than picking one.
    #[must_use]
    pub const fn of(location: AddressLocation) -> Option<Self> {
        match location {
            AddressLocation::Input => Some(Self::Input),
            AddressLocation::Output => Some(Self::Output),
            AddressLocation::Memory => None,
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Input => "read from the device",
            Self::Output => "written to the device",
        })
    }
}

/// Whether a table is addressed a bit or a word at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// One bit per item.
    Bit,
    /// Sixteen bits per item.
    Word,
}

impl Flow {
    /// How a table is addressed.
    #[must_use]
    pub const fn of(table: Table) -> Self {
        match table {
            Table::DiscreteInputs | Table::Coils => Self::Bit,
            Table::InputRegisters | Table::HoldingRegisters => Self::Word,
        }
    }

    /// How an address size is addressed, or `None` for a size no Modbus table
    /// can fill — `%ID` and `%IL` span more than one register, and which
    /// register is the high half is a question the specification does not
    /// answer. See `docs/adr/ADR-0012-modbus-addressing.md`.
    #[must_use]
    pub const fn of_size(size: AddressSize) -> Option<Self> {
        match size {
            AddressSize::Bit => Some(Self::Bit),
            AddressSize::Word => Some(Self::Word),
            AddressSize::Byte | AddressSize::DoubleWord | AddressSize::LongWord => None,
        }
    }
}

impl fmt::Display for Flow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bit => "a bit at a time",
            Self::Word => "a word at a time",
        })
    }
}

/// One run of device addresses, bound to one place in the process image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    /// Which of the device's four tables.
    pub table: Table,
    /// The first PDU address on the device. Zero-based, as it goes on the
    /// wire, with no offset applied anywhere.
    pub device_start: u16,
    /// How many items.
    pub count: u16,
    /// Where the first item appears in the process image.
    pub image: DirectAddress,
}

impl Mapping {
    /// Which way this mapping moves data.
    ///
    /// # Errors
    ///
    /// Returns [`MappingError::NoDirection`] for an image address in `%M`.
    pub fn direction(&self) -> Result<Direction, MappingError> {
        Direction::of(self.image.location).ok_or_else(|| MappingError::NoDirection {
            image: self.image.to_string(),
        })
    }

    /// Checks that the mapping means something.
    ///
    /// # Errors
    ///
    /// Returns the first [`MappingError`] that applies. `image_bytes` is how
    /// large each area of the process image is.
    pub fn check(&self, image_bytes: usize) -> Result<(), MappingError> {
        let direction = self.direction()?;

        if self.count == 0 {
            return Err(MappingError::Empty);
        }

        // The widths have to agree, or a run of bits would appear as words.
        let table_flow = Flow::of(self.table);
        let image_flow =
            Flow::of_size(self.image.size).ok_or_else(|| MappingError::SizeUnusable {
                image: self.image.to_string(),
            })?;
        if table_flow != image_flow {
            return Err(MappingError::WidthMismatch {
                table: self.table,
                table_flow,
                image: self.image.to_string(),
                image_flow,
            });
        }

        // There is no Modbus function that writes a discrete input or an input
        // register, so a mapping that asked for one could never run.
        if direction == Direction::Output && !self.table.is_writable_over_the_network() {
            return Err(MappingError::TableNotWritable { table: self.table });
        }

        // The last device address must exist. Addresses are sixteen bits, so a
        // run that would pass 0xFFFF is a run that wraps.
        let last = u32::from(self.device_start) + u32::from(self.count) - 1;
        if last > u32::from(u16::MAX) {
            return Err(MappingError::PastTheAddressSpace {
                start: self.device_start,
                count: self.count,
            });
        }

        // And the image range must exist too.
        let (first_bit, bits) = self.image_bit_range()?;
        let image_bits = image_bytes as u64 * 8;
        if first_bit + bits > image_bits {
            return Err(MappingError::PastTheImage {
                image: self.image.to_string(),
                count: self.count,
                needs_bit: first_bit + bits,
                image_bits,
            });
        }
        Ok(())
    }

    /// The bits of the process image this mapping occupies: the first, and how
    /// many.
    ///
    /// Expressed in bits rather than bytes so that a bit mapping and a word
    /// mapping can be compared for overlap without a special case.
    ///
    /// # Errors
    ///
    /// Returns [`MappingError`] if the image address is one no mapping can
    /// use.
    pub fn image_bit_range(&self) -> Result<(u64, u64), MappingError> {
        let Some(path) = &self.image.path else {
            return Err(MappingError::PartlySpecified {
                image: self.image.to_string(),
            });
        };
        let flow = Flow::of_size(self.image.size).ok_or_else(|| MappingError::SizeUnusable {
            image: self.image.to_string(),
        })?;
        let (index, bit) = match path.as_slice() {
            [index] => (u64::from(*index), 0),
            [index, bit] => (u64::from(*index), u64::from(*bit)),
            _ => {
                return Err(MappingError::HierarchicalAddress {
                    image: self.image.to_string(),
                });
            }
        };
        match flow {
            Flow::Bit => Ok((index * 8 + bit, u64::from(self.count))),
            // A word address counts words, so word 4 begins at bit 64. That is
            // the ElementIndex layout, which is salman's default; a project
            // that needs the ByteOffset layout is not yet expressible, and
            // saying so beats silently applying the wrong one.
            Flow::Word => Ok((index * 16, u64::from(self.count) * 16)),
        }
    }

    /// Whether two mappings claim any of the same image bits.
    ///
    /// # Errors
    ///
    /// Returns [`MappingError`] if either address is one no mapping can use.
    pub fn overlaps(&self, other: &Self) -> Result<bool, MappingError> {
        if self.image.location != other.image.location {
            return Ok(false);
        }
        let (a_start, a_len) = self.image_bit_range()?;
        let (b_start, b_len) = other.image_bit_range()?;
        Ok(a_start < b_start + b_len && b_start < a_start + a_len)
    }
}

/// Why a mapping does not mean anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingError {
    /// The image address is in `%M`, which implies no direction.
    NoDirection {
        /// The address as written.
        image: String,
    },
    /// The mapping covers nothing.
    Empty,
    /// A bit table was mapped to a word address, or the reverse.
    WidthMismatch {
        /// The device table.
        table: Table,
        /// How that table is addressed.
        table_flow: Flow,
        /// The image address as written.
        image: String,
        /// How that address is addressed.
        image_flow: Flow,
    },
    /// The image address is a size no Modbus table can fill.
    SizeUnusable {
        /// The address as written.
        image: String,
    },
    /// The mapping writes to a table Modbus has no function to write.
    TableNotWritable {
        /// The device table.
        table: Table,
    },
    /// The run of device addresses would pass `0xFFFF`.
    PastTheAddressSpace {
        /// The first address.
        start: u16,
        /// How many.
        count: u16,
    },
    /// The run would pass the end of the process image.
    PastTheImage {
        /// The image address as written.
        image: String,
        /// How many items.
        count: u16,
        /// The bit it would need.
        needs_bit: u64,
        /// How many bits the image has.
        image_bits: u64,
    },
    /// The image address was written `%I*`, with no index.
    PartlySpecified {
        /// The address as written.
        image: String,
    },
    /// The image address has a hierarchical path, which a mapping cannot use.
    HierarchicalAddress {
        /// The address as written.
        image: String,
    },
    /// Two mappings claim the same image bits.
    Overlap {
        /// The first, as written.
        first: String,
        /// The second, as written.
        second: String,
    },
}

impl fmt::Display for MappingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDirection { image } => write!(
                f,
                "{image} is in the marker area, so this mapping has no direction: \
                 map a device to %I to read it or to %Q to write it"
            ),
            Self::Empty => f.write_str("a mapping of no items maps nothing"),
            Self::WidthMismatch {
                table,
                table_flow,
                image,
                image_flow,
            } => write!(
                f,
                "{} are addressed {table_flow} and {image} is addressed {image_flow}",
                table.name()
            ),
            Self::SizeUnusable { image } => write!(
                f,
                "{image} is a size no Modbus table fills: a table is bits or 16-bit \
                 registers, so a mapping names %IX or %IW"
            ),
            Self::TableNotWritable { table } => write!(
                f,
                "{} cannot be written: Modbus has no function that writes them, so \
                 mapping them to %Q could never run",
                table.name()
            ),
            Self::PastTheAddressSpace { start, count } => write!(
                f,
                "{count} items from address {start} passes 65535, and a Modbus address \
                 is sixteen bits"
            ),
            Self::PastTheImage {
                image,
                count,
                needs_bit,
                image_bits,
            } => write!(
                f,
                "{count} items at {image} need bit {needs_bit} of the process image, \
                 and it holds {image_bits}"
            ),
            Self::PartlySpecified { image } => write!(
                f,
                "{image} has no index, so there is nowhere for the mapping to put anything"
            ),
            Self::HierarchicalAddress { image } => write!(
                f,
                "{image} names a hierarchical position, and a mapping needs a plain \
                 address such as %IW0 or %IX0.0"
            ),
            Self::Overlap { first, second } => write!(
                f,
                "{first} and {second} claim the same part of the process image, so \
                 whichever ran second would win"
            ),
        }
    }
}

impl core::error::Error for MappingError {}

/// Checks a whole set of mappings, including that none overlaps another.
///
/// # Errors
///
/// Returns every problem found, in the order the mappings were given, rather
/// than only the first: a file with three bad mappings should need one run to
/// find out, not three.
pub fn check_all(mappings: &[Mapping], image_bytes: usize) -> Vec<MappingError> {
    let mut problems = Vec::new();
    for mapping in mappings {
        if let Err(error) = mapping.check(image_bytes) {
            problems.push(error);
        }
    }
    for (index, first) in mappings.iter().enumerate() {
        for second in mappings.iter().skip(index + 1) {
            if first.overlaps(second).unwrap_or(false) {
                problems.push(MappingError::Overlap {
                    first: first.image.to_string(),
                    second: second.image.to_string(),
                });
            }
        }
    }
    problems
}
