// SPDX-License-Identifier: Apache-2.0
//! Program memory: variable slots, the process image, and the force list.
//!
//! # The process image is the whole point
//!
//! A PLC does not read its inputs when the program asks for them. It
//! **snapshots** every input at the start of the scan, runs the program against
//! that snapshot, and writes the accumulated outputs at the end. A variable read
//! twice in one scan therefore reads the same value twice, even if the physical
//! input changed in between.
//!
//! Getting this wrong makes every simulation subtly lie — a program that works
//! in the simulator because it saw an input change mid-scan will not work on
//! the plant. The tests for this behaviour in this module were written before
//! the implementation was, and they are the first thing to check if a scan ever
//! looks wrong.
//!
//! # Bit, byte and word addresses overlay each other
//!
//! `%QX0.0` is bit 0 of the byte that `%QB0` names, and `%QW0` covers that byte
//! and the one after it. Writing one changes the others, exactly as it does on
//! a real controller, which is why the image is bytes rather than a map of
//! independent variables.
//!
//! # Two things the standard leaves open, which salman therefore configures
//!
//! 1. **What `%IW4` means.** Whether the number in a byte, word, double-word or
//!    long-word address is a *byte offset* or an *element index* is not fixed
//!    by IEC 61131-3 — and vendors genuinely differ, so the same source text
//!    addresses different memory on different systems. It is an explicit
//!    setting here, and every address resolution can say which rule it used.
//! 2. **Byte order within the image.** Also unfixed, also divergent between
//!    vendors. Explicit setting, little-endian by default.

use std::collections::BTreeMap;

use salman_core::value::{ElementaryType, Value};
use salman_lang::address::{AddressLocation, AddressSize, DirectAddress};

/// Whether the number in `%IW4` counts bytes or words.
///
/// Not fixed by IEC 61131-3, and vendors differ: some read `%IW4` as the word
/// at byte offset 4, others as the fourth word, at byte offset 8. Getting this
/// wrong silently addresses the wrong memory, so salman makes it a choice
/// rather than an assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddressGranularity {
    /// `%IW4` is the fourth word: byte offset 8.
    #[default]
    ElementIndex,
    /// `%IW4` is the word at byte offset 4.
    ByteOffset,
}

/// Byte order within the process image.
///
/// Also not fixed by IEC 61131-3, and also divergent between vendors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageByteOrder {
    /// Least significant byte first.
    #[default]
    LittleEndian,
    /// Most significant byte first.
    BigEndian,
}

/// How a process image is laid out and addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImageLayout {
    /// What the number in a sized address counts.
    pub granularity: AddressGranularity,
    /// Byte order for multi-byte reads and writes.
    pub byte_order: ImageByteOrder,
}

/// Why an address could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressError {
    /// The address is past the end of its area.
    OutOfRange {
        /// The address as written.
        address: String,
        /// The byte the address resolved to.
        byte: u64,
        /// How large the area is.
        area_len: usize,
    },
    /// A bit number above 7 was written, as in `%IX0.9`.
    BitOutOfRange {
        /// The address as written.
        address: String,
        /// The bit number.
        bit: u32,
    },
    /// The address has no index path, because it was written `%I*`.
    PartlySpecified {
        /// The address as written.
        address: String,
    },
    /// The address nests deeper than salman resolves.
    ///
    /// IEC 61131-3 permits arbitrarily deep hierarchical addresses and leaves
    /// their meaning to the configuration. salman resolves one level, and two
    /// for a bit address, and says so rather than guessing at the rest.
    UnsupportedDepth {
        /// The address as written.
        address: String,
        /// How many levels it has.
        depth: usize,
    },
}

impl std::fmt::Display for AddressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange {
                address,
                byte,
                area_len,
            } => write!(
                f,
                "{address} resolves to byte {byte}, past the end of a {area_len} byte area"
            ),
            Self::BitOutOfRange { address, bit } => {
                write!(f, "{address} names bit {bit}; a byte has bits 0 to 7")
            }
            Self::PartlySpecified { address } => write!(
                f,
                "{address} is partly specified and must be given a location by the configuration"
            ),
            Self::UnsupportedDepth { address, depth } => write!(
                f,
                "{address} is {depth} levels deep; salman resolves one level, or two for a bit"
            ),
        }
    }
}

impl std::error::Error for AddressError {}

/// A resolved position in a process image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImagePosition {
    /// Byte offset from the start of the area.
    pub byte: u32,
    /// Bit within that byte, for bit-sized addresses.
    pub bit: u8,
    /// How wide the datum is.
    pub size: AddressSize,
}

/// A contiguous area of process memory, addressed as bits, bytes or words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessImage {
    bytes: Vec<u8>,
    layout: ImageLayout,
}

impl ProcessImage {
    /// An area of `len` zeroed bytes.
    #[must_use]
    pub fn new(len: usize, layout: ImageLayout) -> Self {
        Self {
            bytes: vec![0; len],
            layout,
        }
    }

    /// How many bytes the area holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the area is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The raw bytes, for tracing and for the process-image copy.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Sets every byte to zero.
    pub fn clear(&mut self) {
        self.bytes.fill(0);
    }

    /// Copies `other` over this area.
    ///
    /// This is the scan-start input snapshot and the scan-end output flush.
    /// The areas must be the same length; a shorter source copies only what it
    /// has, which cannot happen for images built together.
    pub fn copy_from(&mut self, other: &Self) {
        let n = self.bytes.len().min(other.bytes.len());
        if let (Some(dst), Some(src)) = (self.bytes.get_mut(..n), other.bytes.get(..n)) {
            dst.copy_from_slice(src);
        }
    }

    /// Turns a written address into a position in this area.
    ///
    /// # Errors
    ///
    /// Returns [`AddressError`] when the address is partly specified, nests too
    /// deeply, or names a bit above 7.
    pub fn resolve(&self, address: &DirectAddress) -> Result<ImagePosition, AddressError> {
        let written = address.to_string();
        let Some(path) = &address.path else {
            return Err(AddressError::PartlySpecified { address: written });
        };
        let width_bytes = u64::from(address.size.bits()).div_ceil(8).max(1);

        match (address.size, path.len()) {
            // `%IX7.5` and `%I7.5`: byte then bit.
            (AddressSize::Bit, 2) => {
                let (Some(&byte), Some(&bit)) = (path.first(), path.get(1)) else {
                    return Err(AddressError::UnsupportedDepth {
                        address: written,
                        depth: 2,
                    });
                };
                if bit > 7 {
                    return Err(AddressError::BitOutOfRange {
                        address: written,
                        bit,
                    });
                }
                Ok(ImagePosition {
                    byte,
                    bit: bit as u8,
                    size: AddressSize::Bit,
                })
            }
            // `%IX13`: a flat bit number, so byte 1 bit 5.
            (AddressSize::Bit, 1) => {
                let Some(&index) = path.first() else {
                    return Err(AddressError::UnsupportedDepth {
                        address: written,
                        depth: 1,
                    });
                };
                Ok(ImagePosition {
                    byte: index / 8,
                    bit: (index % 8) as u8,
                    size: AddressSize::Bit,
                })
            }
            (_, 1) => {
                let Some(&index) = path.first() else {
                    return Err(AddressError::UnsupportedDepth {
                        address: written,
                        depth: 1,
                    });
                };
                let byte = match self.layout.granularity {
                    AddressGranularity::ElementIndex => u64::from(index) * width_bytes,
                    AddressGranularity::ByteOffset => u64::from(index),
                };
                let Ok(byte) = u32::try_from(byte) else {
                    return Err(AddressError::OutOfRange {
                        address: written,
                        byte,
                        area_len: self.bytes.len(),
                    });
                };
                Ok(ImagePosition {
                    byte,
                    bit: 0,
                    size: address.size,
                })
            }
            (_, depth) => Err(AddressError::UnsupportedDepth {
                address: written,
                depth,
            }),
        }
    }

    /// Reads the value at a position.
    ///
    /// Reads past the end of the area yield `None` rather than a panic: an
    /// address can come from a source file salman did not write.
    #[must_use]
    pub fn read(&self, at: ImagePosition) -> Option<Value> {
        let start = at.byte as usize;
        Some(match at.size {
            AddressSize::Bit => {
                let byte = self.bytes.get(start).copied()?;
                Value::Bool((byte >> at.bit) & 1 == 1)
            }
            AddressSize::Byte => Value::Byte(self.bytes.get(start).copied()?),
            AddressSize::Word => Value::Word(u16::from_le_bytes(self.read_array::<2>(start)?)),
            AddressSize::DoubleWord => {
                Value::Dword(u32::from_le_bytes(self.read_array::<4>(start)?))
            }
            AddressSize::LongWord => Value::Lword(u64::from_le_bytes(self.read_array::<8>(start)?)),
        })
    }

    /// Writes a value at a position, returning whether it fitted.
    ///
    /// The value's width must match the position's; a mismatch is a compiler
    /// bug, and this returns `false` rather than silently truncating.
    pub fn write(&mut self, at: ImagePosition, value: &Value) -> bool {
        let start = at.byte as usize;
        match (at.size, value) {
            (AddressSize::Bit, Value::Bool(v)) => {
                let Some(byte) = self.bytes.get_mut(start) else {
                    return false;
                };
                let mask = 1u8 << at.bit;
                if *v {
                    *byte |= mask;
                } else {
                    *byte &= !mask;
                }
                true
            }
            (AddressSize::Byte, Value::Byte(v) | Value::Usint(v)) => {
                let Some(byte) = self.bytes.get_mut(start) else {
                    return false;
                };
                *byte = *v;
                true
            }
            (AddressSize::Word, Value::Word(v) | Value::Uint(v)) => {
                self.write_array(start, &self.to_image_order_2(*v))
            }
            (AddressSize::DoubleWord, Value::Dword(v) | Value::Udint(v)) => {
                self.write_array(start, &self.to_image_order_4(*v))
            }
            (AddressSize::LongWord, Value::Lword(v) | Value::Ulint(v)) => {
                self.write_array(start, &self.to_image_order_8(*v))
            }
            _ => false,
        }
    }

    fn read_array<const N: usize>(&self, start: usize) -> Option<[u8; N]> {
        let slice = self.bytes.get(start..start.checked_add(N)?)?;
        let mut out = [0u8; N];
        out.copy_from_slice(slice);
        if self.layout.byte_order == ImageByteOrder::BigEndian {
            out.reverse();
        }
        Some(out)
    }

    fn write_array<const N: usize>(&mut self, start: usize, value: &[u8; N]) -> bool {
        let Some(end) = start.checked_add(N) else {
            return false;
        };
        let Some(slice) = self.bytes.get_mut(start..end) else {
            return false;
        };
        slice.copy_from_slice(value);
        true
    }

    fn to_image_order_2(&self, v: u16) -> [u8; 2] {
        let mut b = v.to_le_bytes();
        if self.layout.byte_order == ImageByteOrder::BigEndian {
            b.reverse();
        }
        b
    }

    fn to_image_order_4(&self, v: u32) -> [u8; 4] {
        let mut b = v.to_le_bytes();
        if self.layout.byte_order == ImageByteOrder::BigEndian {
            b.reverse();
        }
        b
    }

    fn to_image_order_8(&self, v: u64) -> [u8; 8] {
        let mut b = v.to_le_bytes();
        if self.layout.byte_order == ImageByteOrder::BigEndian {
            b.reverse();
        }
        b
    }
}

/// Identifies one variable slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotId(pub u32);

impl SlotId {
    /// The index this slot addresses.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// How a slot survives a simulated power cycle. IEC 61131-3:2013 §6.5.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Persistence {
    /// Re-initialised on any restart.
    #[default]
    Volatile,
    /// `RETAIN`: kept across a warm restart.
    Retain,
    /// `PERSISTENT`: kept across a cold restart too.
    Persistent,
}

/// One forced variable.
///
/// Forces are how people get hurt. salman keeps them in one list, never hides
/// them, and makes the count available so that no interface can show a running
/// program without showing that it is being lied to.
#[derive(Debug, Clone, PartialEq)]
pub struct Force {
    /// The slot being forced.
    pub slot: SlotId,
    /// The value reads see.
    pub value: Value,
    /// What the program last tried to write, if it has tried.
    ///
    /// Kept so an interface can show the difference between what the logic
    /// wants and what the force is imposing — which is the question anyone
    /// looking at a force list is actually asking.
    pub suppressed_write: Option<Value>,
}

/// All of a program's memory.
#[derive(Debug, Clone)]
pub struct Memory {
    slots: Vec<Value>,
    initial: Vec<Value>,
    persistence: Vec<Persistence>,
    physical_inputs: ProcessImage,
    input_image: ProcessImage,
    output_image: ProcessImage,
    physical_outputs: ProcessImage,
    markers: ProcessImage,
    forces: BTreeMap<SlotId, Force>,
}

impl Memory {
    /// Builds memory for a program.
    ///
    /// `slot_types` gives every variable's type, in slot order; each starts at
    /// that type's default initial value. The three image areas are sized in
    /// bytes.
    #[must_use]
    pub fn new(slot_types: &[ElementaryType], image_bytes: usize, layout: ImageLayout) -> Self {
        let initial: Vec<Value> = slot_types.iter().map(|t| t.default_value()).collect();
        Self {
            slots: initial.clone(),
            initial,
            persistence: vec![Persistence::Volatile; slot_types.len()],
            physical_inputs: ProcessImage::new(image_bytes, layout),
            input_image: ProcessImage::new(image_bytes, layout),
            output_image: ProcessImage::new(image_bytes, layout),
            physical_outputs: ProcessImage::new(image_bytes, layout),
            markers: ProcessImage::new(image_bytes, layout),
            forces: BTreeMap::new(),
        }
    }

    /// Sets a slot's initial value, used for declared initialisers.
    pub fn set_initial(&mut self, slot: SlotId, value: Value) {
        if let Some(cell) = self.initial.get_mut(slot.index()) {
            cell.clone_from(&value);
        }
        if let Some(cell) = self.slots.get_mut(slot.index()) {
            *cell = value;
        }
    }

    /// Declares how a slot survives a restart.
    pub fn set_persistence(&mut self, slot: SlotId, persistence: Persistence) {
        if let Some(cell) = self.persistence.get_mut(slot.index()) {
            *cell = persistence;
        }
    }

    /// How many slots there are.
    #[must_use]
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Reads a slot, honouring any force on it.
    #[must_use]
    pub fn read_slot(&self, slot: SlotId) -> Option<&Value> {
        if let Some(force) = self.forces.get(&slot) {
            return Some(&force.value);
        }
        self.slots.get(slot.index())
    }

    /// Reads a slot as the program's logic left it, ignoring any force.
    ///
    /// This is what a debugger shows next to the forced value.
    #[must_use]
    pub fn read_slot_unforced(&self, slot: SlotId) -> Option<&Value> {
        self.slots.get(slot.index())
    }

    /// Writes a slot. A forced slot records the attempt and keeps its force.
    pub fn write_slot(&mut self, slot: SlotId, value: Value) -> bool {
        if let Some(force) = self.forces.get_mut(&slot) {
            force.suppressed_write = Some(value);
            return true;
        }
        match self.slots.get_mut(slot.index()) {
            Some(cell) => {
                *cell = value;
                true
            }
            None => false,
        }
    }

    /// Forces a slot to a value until it is released.
    pub fn force(&mut self, slot: SlotId, value: Value) -> bool {
        if slot.index() >= self.slots.len() {
            return false;
        }
        self.forces.insert(
            slot,
            Force {
                slot,
                value,
                suppressed_write: None,
            },
        );
        true
    }

    /// Releases one force.
    pub fn release(&mut self, slot: SlotId) -> bool {
        self.forces.remove(&slot).is_some()
    }

    /// Releases every force.
    pub fn release_all(&mut self) {
        self.forces.clear();
    }

    /// Every active force, in slot order.
    pub fn forces(&self) -> impl Iterator<Item = &Force> {
        self.forces.values()
    }

    /// How many forces are active. Never hidden from an interface.
    #[must_use]
    pub fn force_count(&self) -> usize {
        self.forces.len()
    }

    /// The physical input area, which the outside world writes.
    pub fn physical_inputs_mut(&mut self) -> &mut ProcessImage {
        &mut self.physical_inputs
    }

    /// The physical output area, which the outside world reads.
    #[must_use]
    pub fn physical_outputs(&self) -> &ProcessImage {
        &self.physical_outputs
    }

    /// The snapshot the program reads. Frozen for the whole scan.
    #[must_use]
    pub fn input_image(&self) -> &ProcessImage {
        &self.input_image
    }

    /// What the program has written this scan.
    #[must_use]
    pub fn output_image(&self) -> &ProcessImage {
        &self.output_image
    }

    /// The mutable output image, for the program.
    pub fn output_image_mut(&mut self) -> &mut ProcessImage {
        &mut self.output_image
    }

    /// The `%M` area, which has no image and is written through immediately.
    #[must_use]
    pub fn marker_memory(&self) -> &ProcessImage {
        &self.markers
    }

    /// The mutable `%M` area.
    pub fn marker_memory_mut(&mut self) -> &mut ProcessImage {
        &mut self.markers
    }

    /// Takes the input snapshot. The first half of a scan.
    pub fn latch_inputs(&mut self) {
        self.input_image.copy_from(&self.physical_inputs);
    }

    /// Publishes the outputs. The second half of a scan.
    pub fn publish_outputs(&mut self) {
        self.physical_outputs.copy_from(&self.output_image);
    }

    /// Reads through a written address, using the right area for its location.
    ///
    /// # Errors
    ///
    /// Returns [`AddressError`] if the address cannot be resolved.
    pub fn read_address(&self, address: &DirectAddress) -> Result<Option<Value>, AddressError> {
        let area = self.area_for(address.location);
        let position = area.resolve(address)?;
        Ok(area.read(position))
    }

    /// Writes through a written address.
    ///
    /// Writing to `%I` is refused: inputs are what the world tells the program,
    /// and a program that could write them would be able to fake its own
    /// sensors.
    ///
    /// # Errors
    ///
    /// Returns [`AddressError`] if the address cannot be resolved.
    pub fn write_address(
        &mut self,
        address: &DirectAddress,
        value: &Value,
    ) -> Result<bool, AddressError> {
        if address.location == AddressLocation::Input {
            return Ok(false);
        }
        let position = self.area_for(address.location).resolve(address)?;
        let area = match address.location {
            AddressLocation::Output => &mut self.output_image,
            AddressLocation::Memory => &mut self.markers,
            AddressLocation::Input => return Ok(false),
        };
        Ok(area.write(position, value))
    }

    fn area_for(&self, location: AddressLocation) -> &ProcessImage {
        match location {
            AddressLocation::Input => &self.input_image,
            AddressLocation::Output => &self.output_image,
            AddressLocation::Memory => &self.markers,
        }
    }

    /// Restarts the program, keeping whatever the restart kind keeps.
    ///
    /// A **cold** restart re-initialises everything except `PERSISTENT`
    /// variables. A **warm** restart also keeps `RETAIN` variables. This is how
    /// a retain bug surfaces in a test rather than in the plant.
    pub fn restart(&mut self, kind: Restart) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let persistence = self.persistence.get(index).copied().unwrap_or_default();
            let keep = matches!(
                (kind, persistence),
                (_, Persistence::Persistent) | (Restart::Warm, Persistence::Retain)
            );
            if !keep && let Some(initial) = self.initial.get(index) {
                slot.clone_from(initial);
            }
        }
        self.input_image.clear();
        self.output_image.clear();
        self.physical_outputs.clear();
        // The `%M` area is not cleared on a warm restart: it is where retained
        // markers live on most controllers.
        if kind == Restart::Cold {
            self.markers.clear();
        }
        self.forces.clear();
    }
}

/// What kind of restart is being simulated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    /// Keeps `RETAIN` and `PERSISTENT` variables.
    Warm,
    /// Keeps only `PERSISTENT` variables.
    Cold,
}

#[cfg(test)]
mod tests {
    use super::*;
    use salman_lang::address::{AddressLocation, AddressSize};

    fn address(location: AddressLocation, size: AddressSize, path: &[u32]) -> DirectAddress {
        DirectAddress {
            location,
            size,
            size_letter_written: true,
            path: Some(path.to_vec()),
        }
    }

    fn memory() -> Memory {
        Memory::new(&[ElementaryType::Bool; 8], 16, ImageLayout::default())
    }

    // -----------------------------------------------------------------
    // The process image. These tests were written before the implementation.
    // -----------------------------------------------------------------

    #[test]
    fn an_input_read_mid_scan_sees_the_value_it_had_at_scan_start() {
        // The single most important behaviour in the whole runtime. A PLC
        // snapshots its inputs once per scan; a program that sees an input
        // change part way through a scan is being lied to by the simulator,
        // and will not behave the same way on the plant.
        let mut memory = memory();
        let sensor = address(AddressLocation::Input, AddressSize::Bit, &[0, 0]);

        // The world sets the input, and the scan begins.
        let position = memory.input_image().resolve(&sensor).unwrap();
        memory
            .physical_inputs_mut()
            .write(position, &Value::Bool(true));
        memory.latch_inputs();
        assert_eq!(
            memory.read_address(&sensor).unwrap(),
            Some(Value::Bool(true))
        );

        // The world changes the input while the program is running.
        memory
            .physical_inputs_mut()
            .write(position, &Value::Bool(false));

        // The program still sees the snapshot.
        assert_eq!(
            memory.read_address(&sensor).unwrap(),
            Some(Value::Bool(true)),
            "the program saw an input change part way through a scan"
        );

        // Only the next scan's snapshot shows the change.
        memory.latch_inputs();
        assert_eq!(
            memory.read_address(&sensor).unwrap(),
            Some(Value::Bool(false))
        );
    }

    #[test]
    fn an_output_written_this_scan_reads_back_as_written_before_it_is_published() {
        // The other half of the scan model: the program reads its own output
        // image, so a coil set earlier in the scan is visible later in the same
        // scan. This is what makes seal-in logic work.
        let mut memory = memory();
        let coil = address(AddressLocation::Output, AddressSize::Bit, &[0, 3]);
        memory.write_address(&coil, &Value::Bool(true)).unwrap();
        assert_eq!(memory.read_address(&coil).unwrap(), Some(Value::Bool(true)));
    }

    #[test]
    fn outputs_do_not_reach_the_world_until_the_scan_ends() {
        let mut memory = memory();
        let coil = address(AddressLocation::Output, AddressSize::Bit, &[0, 0]);
        memory.write_address(&coil, &Value::Bool(true)).unwrap();
        assert_eq!(memory.physical_outputs().bytes().first(), Some(&0));
        memory.publish_outputs();
        assert_eq!(memory.physical_outputs().bytes().first(), Some(&1));
    }

    #[test]
    fn a_program_cannot_write_its_own_inputs() {
        let mut memory = memory();
        let sensor = address(AddressLocation::Input, AddressSize::Bit, &[0, 0]);
        assert!(!memory.write_address(&sensor, &Value::Bool(true)).unwrap());
        assert_eq!(
            memory.read_address(&sensor).unwrap(),
            Some(Value::Bool(false))
        );
    }

    #[test]
    fn marker_memory_is_written_through_with_no_image() {
        // %M has no snapshot: it is scratch memory, and a change is visible
        // immediately, including to a higher-priority task.
        let mut memory = memory();
        let flag = address(AddressLocation::Memory, AddressSize::Byte, &[2]);
        memory.write_address(&flag, &Value::Byte(0xAB)).unwrap();
        assert_eq!(memory.read_address(&flag).unwrap(), Some(Value::Byte(0xAB)));
    }

    // -----------------------------------------------------------------
    // Bit, byte and word overlay
    // -----------------------------------------------------------------

    #[test]
    fn bit_byte_and_word_addresses_overlay_each_other_as_they_do_on_a_controller() {
        let mut memory = memory();
        let byte0 = address(AddressLocation::Output, AddressSize::Byte, &[0]);
        memory
            .write_address(&byte0, &Value::Byte(0b1010_0101))
            .unwrap();

        for (bit, expected) in (0..8).zip([true, false, true, false, false, true, false, true]) {
            let b = address(AddressLocation::Output, AddressSize::Bit, &[0, bit]);
            assert_eq!(
                memory.read_address(&b).unwrap(),
                Some(Value::Bool(expected)),
                "bit {bit} of a byte written as 1010_0101"
            );
        }

        // Setting one bit changes the byte the others share.
        let bit1 = address(AddressLocation::Output, AddressSize::Bit, &[0, 1]);
        memory.write_address(&bit1, &Value::Bool(true)).unwrap();
        assert_eq!(
            memory.read_address(&byte0).unwrap(),
            Some(Value::Byte(0b1010_0111))
        );
    }

    #[test]
    fn a_flat_bit_number_addresses_the_bit_within_its_byte() {
        // `%QX13` with no dot is bit 5 of byte 1.
        let mut memory = memory();
        let flat = address(AddressLocation::Output, AddressSize::Bit, &[13]);
        memory.write_address(&flat, &Value::Bool(true)).unwrap();
        let dotted = address(AddressLocation::Output, AddressSize::Bit, &[1, 5]);
        assert_eq!(
            memory.read_address(&dotted).unwrap(),
            Some(Value::Bool(true))
        );
    }

    #[test]
    fn word_addressing_granularity_is_a_setting_because_vendors_disagree() {
        // %QW1 is either the word at byte 1, or the second word, at byte 2.
        // Both are shipped by real vendors, and reading the wrong one silently
        // addresses the wrong memory.
        let word1 = address(AddressLocation::Output, AddressSize::Word, &[1]);

        let element = ProcessImage::new(
            16,
            ImageLayout {
                granularity: AddressGranularity::ElementIndex,
                ..Default::default()
            },
        );
        assert_eq!(element.resolve(&word1).unwrap().byte, 2);

        let offset = ProcessImage::new(
            16,
            ImageLayout {
                granularity: AddressGranularity::ByteOffset,
                ..Default::default()
            },
        );
        assert_eq!(offset.resolve(&word1).unwrap().byte, 1);
    }

    #[test]
    fn image_byte_order_is_a_setting_and_round_trips_either_way() {
        for order in [ImageByteOrder::LittleEndian, ImageByteOrder::BigEndian] {
            let layout = ImageLayout {
                byte_order: order,
                ..Default::default()
            };
            let mut image = ProcessImage::new(8, layout);
            let at = ImagePosition {
                byte: 0,
                bit: 0,
                size: AddressSize::Word,
            };
            assert!(image.write(at, &Value::Word(0x1234)));
            assert_eq!(image.read(at), Some(Value::Word(0x1234)));
            let first = image.bytes().first().copied();
            assert_eq!(
                first,
                Some(if order == ImageByteOrder::LittleEndian {
                    0x34
                } else {
                    0x12
                })
            );
        }
    }

    #[test]
    fn an_address_past_the_end_of_its_area_reads_none_rather_than_panicking() {
        let memory = Memory::new(&[], 4, ImageLayout::default());
        let far = address(AddressLocation::Input, AddressSize::LongWord, &[100]);
        assert_eq!(memory.read_address(&far).unwrap(), None);
    }

    #[test]
    fn a_bit_above_seven_is_rejected_with_a_message_naming_the_address() {
        let memory = memory();
        let bad = address(AddressLocation::Input, AddressSize::Bit, &[0, 9]);
        let err = memory.read_address(&bad).unwrap_err();
        assert!(
            matches!(err, AddressError::BitOutOfRange { bit: 9, .. }),
            "{err}"
        );
        assert!(err.to_string().contains("%IX0.9"), "{err}");
    }

    #[test]
    fn a_partly_specified_address_is_refused_rather_than_defaulted_to_zero() {
        let memory = memory();
        let star = DirectAddress {
            location: AddressLocation::Input,
            size: AddressSize::Word,
            size_letter_written: true,
            path: None,
        };
        assert!(matches!(
            memory.read_address(&star),
            Err(AddressError::PartlySpecified { .. })
        ));
    }

    #[test]
    fn an_address_deeper_than_salman_resolves_says_so_instead_of_guessing() {
        let memory = memory();
        let deep = address(AddressLocation::Input, AddressSize::Word, &[1, 2, 3]);
        let err = memory.read_address(&deep).unwrap_err();
        assert!(
            matches!(err, AddressError::UnsupportedDepth { depth: 3, .. }),
            "{err}"
        );
    }

    // -----------------------------------------------------------------
    // Forcing
    // -----------------------------------------------------------------

    #[test]
    fn a_forced_slot_reads_the_forced_value_and_ignores_the_program() {
        let mut memory = Memory::new(&[ElementaryType::Dint], 0, ImageLayout::default());
        let slot = SlotId(0);
        memory.write_slot(slot, Value::Dint(1));
        assert!(memory.force(slot, Value::Dint(99)));
        assert_eq!(memory.read_slot(slot), Some(&Value::Dint(99)));

        memory.write_slot(slot, Value::Dint(2));
        assert_eq!(memory.read_slot(slot), Some(&Value::Dint(99)));
    }

    #[test]
    fn a_force_records_what_the_program_wanted_so_the_difference_is_visible() {
        // Anyone looking at a force list is asking "what would this be if I
        // released it?". Not recording the suppressed write throws that away.
        let mut memory = Memory::new(&[ElementaryType::Dint], 0, ImageLayout::default());
        let slot = SlotId(0);
        memory.force(slot, Value::Dint(99));
        memory.write_slot(slot, Value::Dint(7));
        let force = memory.forces().next().unwrap();
        assert_eq!(force.value, Value::Dint(99));
        assert_eq!(force.suppressed_write, Some(Value::Dint(7)));
    }

    #[test]
    fn releasing_a_force_restores_what_the_logic_had_computed() {
        let mut memory = Memory::new(&[ElementaryType::Dint], 0, ImageLayout::default());
        let slot = SlotId(0);
        memory.write_slot(slot, Value::Dint(5));
        memory.force(slot, Value::Dint(99));
        assert_eq!(memory.read_slot_unforced(slot), Some(&Value::Dint(5)));
        assert!(memory.release(slot));
        assert_eq!(memory.read_slot(slot), Some(&Value::Dint(5)));
        assert_eq!(memory.force_count(), 0);
    }

    #[test]
    fn the_force_count_is_always_available_so_no_interface_can_hide_one() {
        let mut memory = Memory::new(&[ElementaryType::Bool; 3], 0, ImageLayout::default());
        assert_eq!(memory.force_count(), 0);
        memory.force(SlotId(0), Value::Bool(true));
        memory.force(SlotId(2), Value::Bool(true));
        assert_eq!(memory.force_count(), 2);
        memory.release_all();
        assert_eq!(memory.force_count(), 0);
    }

    #[test]
    fn forcing_a_slot_that_does_not_exist_fails_rather_than_growing_memory() {
        let mut memory = Memory::new(&[ElementaryType::Bool], 0, ImageLayout::default());
        assert!(!memory.force(SlotId(50), Value::Bool(true)));
        assert_eq!(memory.force_count(), 0);
    }

    // -----------------------------------------------------------------
    // Restart and retention
    // -----------------------------------------------------------------

    #[test]
    fn a_warm_restart_keeps_retain_and_persistent_and_clears_the_rest() {
        let types = [ElementaryType::Dint; 3];
        let mut memory = Memory::new(&types, 0, ImageLayout::default());
        memory.set_persistence(SlotId(1), Persistence::Retain);
        memory.set_persistence(SlotId(2), Persistence::Persistent);
        for i in 0..3 {
            memory.write_slot(SlotId(i), Value::Dint(42));
        }
        memory.restart(Restart::Warm);
        assert_eq!(memory.read_slot(SlotId(0)), Some(&Value::Dint(0)));
        assert_eq!(memory.read_slot(SlotId(1)), Some(&Value::Dint(42)));
        assert_eq!(memory.read_slot(SlotId(2)), Some(&Value::Dint(42)));
    }

    #[test]
    fn a_cold_restart_keeps_only_persistent() {
        let types = [ElementaryType::Dint; 3];
        let mut memory = Memory::new(&types, 0, ImageLayout::default());
        memory.set_persistence(SlotId(1), Persistence::Retain);
        memory.set_persistence(SlotId(2), Persistence::Persistent);
        for i in 0..3 {
            memory.write_slot(SlotId(i), Value::Dint(42));
        }
        memory.restart(Restart::Cold);
        assert_eq!(memory.read_slot(SlotId(0)), Some(&Value::Dint(0)));
        assert_eq!(memory.read_slot(SlotId(1)), Some(&Value::Dint(0)));
        assert_eq!(memory.read_slot(SlotId(2)), Some(&Value::Dint(42)));
    }

    #[test]
    fn a_restart_restores_declared_initial_values_not_merely_zero() {
        let mut memory = Memory::new(&[ElementaryType::Dint], 0, ImageLayout::default());
        memory.set_initial(SlotId(0), Value::Dint(7));
        memory.write_slot(SlotId(0), Value::Dint(99));
        memory.restart(Restart::Cold);
        assert_eq!(memory.read_slot(SlotId(0)), Some(&Value::Dint(7)));
    }

    #[test]
    fn a_restart_releases_every_force() {
        // A force that survived a restart would be a force nobody remembered
        // setting.
        let mut memory = Memory::new(&[ElementaryType::Bool], 0, ImageLayout::default());
        memory.force(SlotId(0), Value::Bool(true));
        memory.restart(Restart::Warm);
        assert_eq!(memory.force_count(), 0);
    }

    #[test]
    fn reading_a_slot_that_does_not_exist_is_none_rather_than_a_panic() {
        let memory = Memory::new(&[ElementaryType::Bool], 0, ImageLayout::default());
        assert_eq!(memory.read_slot(SlotId(9)), None);
        assert_eq!(memory.read_slot_unforced(SlotId(9)), None);
    }
}
