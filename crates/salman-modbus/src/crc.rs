// SPDX-License-Identifier: Apache-2.0
//! CRC-16/MODBUS, for RTU framing.
//!
//! Parameters, from MODBUS over Serial Line Specification and Implementation
//! Guide V1.02 (20 December 2006) §2.5.1.2 and Annex B §6.2.2:
//!
//! | | |
//! |---|---|
//! | width | 16 |
//! | polynomial | `0x8005`, used reflected as `0xA001` |
//! | initial value | `0xFFFF` |
//! | reflect in, reflect out | both |
//! | final XOR | none |
//!
//! Two properties of this algorithm are worth stating because they are the two
//! things implementations get wrong.
//!
//! **The CRC is transmitted low byte first**, while every other multi-byte
//! field in Modbus is big-endian. That is not a mistake in this file; it is the
//! specification. [`Crc16::to_wire`] is the only place the order is applied.
//!
//! **Recomputing the CRC over a frame that already carries its CRC yields
//! zero.** A receiver therefore does not have to split the frame to check it —
//! see [`Crc16::residue_ok`] — and the property is asserted in the tests over
//! generated input, which is a stronger check than any fixed vector.
//!
//! This is a **checksum against accidental corruption on a serial line**. It is
//! not a security primitive: it is trivially forgeable, and salman never uses
//! it to decide that data is authentic.

/// The CRC-16/MODBUS of a byte string.
///
/// Held as the 16-bit register value. Converting to the two bytes that go on
/// the wire is [`Crc16::to_wire`], which is where the low-byte-first order
/// lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Crc16(pub u16);

impl Crc16 {
    /// The initial register value, before any byte is fed in.
    pub const INIT: Self = Self(0xFFFF);

    /// The reflected polynomial. `0xA001` is `0x8005` bit-reversed, which is
    /// what a right-shifting implementation uses.
    const POLY: u16 = 0xA001;

    /// Computes the CRC of `bytes`.
    #[must_use]
    pub const fn of(bytes: &[u8]) -> Self {
        let mut crc = Self::INIT.0;
        let mut index = 0;
        while index < bytes.len() {
            crc ^= bytes[index] as u16;
            let mut bit = 0;
            while bit < 8 {
                // The polynomial is applied when the bit shifted out is set.
                if crc & 1 == 1 {
                    crc = (crc >> 1) ^ Self::POLY;
                } else {
                    crc >>= 1;
                }
                bit += 1;
            }
            index += 1;
        }
        Self(crc)
    }

    /// The two bytes as they appear on the wire: **low byte first**.
    ///
    /// Every other multi-byte field in Modbus is big-endian. This one is not,
    /// and MODBUS over Serial Line V1.02 §2.5.1.2 says so explicitly.
    #[must_use]
    pub const fn to_wire(self) -> [u8; 2] {
        [(self.0 & 0x00FF) as u8, (self.0 >> 8) as u8]
    }

    /// Reads the two CRC bytes from the wire, low byte first.
    #[must_use]
    pub const fn from_wire(bytes: [u8; 2]) -> Self {
        Self((bytes[0] as u16) | ((bytes[1] as u16) << 8))
    }

    /// Whether `frame` — which must include its own two CRC bytes — is intact.
    ///
    /// Computing the CRC over a frame that already carries its CRC yields
    /// zero, so this needs no split and no comparison against a stored value.
    /// A frame shorter than the two CRC bytes is not intact by definition.
    #[must_use]
    pub const fn residue_ok(frame: &[u8]) -> bool {
        frame.len() >= 2 && Self::of(frame).0 == 0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::Crc16;

    #[test]
    fn the_one_vector_the_specification_publishes() {
        // MODBUS over Serial Line V1.02, Annex B §6.2.2, the worked example:
        // the message 02 07 has CRC 0x1241, transmitted as 41 12.
        //
        // This is the ONLY numeric CRC vector published in any modbus.org
        // document. Every other vector in this file is salman's own arithmetic
        // and is labelled as such, because a self-generated vector proves the
        // implementation agrees with itself and nothing more.
        assert_eq!(Crc16::of(&[0x02, 0x07]), Crc16(0x1241));
        assert_eq!(Crc16::of(&[0x02, 0x07]).to_wire(), [0x41, 0x12]);
    }

    #[test]
    fn the_catalogue_check_value() {
        // "123456789" -> 0x4B37, the `check` field every CRC catalogue
        // publishes for CRC-16/MODBUS. Not a modbus.org source, but an
        // independent one, which is what makes it worth having: it catches a
        // transcription error that a self-generated vector cannot.
        assert_eq!(Crc16::of(b"123456789"), Crc16(0x4B37));
    }

    #[test]
    fn salman_generated_vectors_for_real_frames() {
        // salman's own arithmetic, not the specification's. They pin the
        // behaviour against accidental change; they do not independently
        // confirm it is right. The two vectors above do that.
        for (frame, expected) in [
            (&[0x01, 0x04, 0x00, 0x00, 0x00, 0x01][..], 0xCA31_u16),
            (&[0x11, 0x03, 0x00, 0x6B, 0x00, 0x03][..], 0x8776),
            (&[0x01, 0x04, 0x02, 0xFF, 0xFF][..], 0x80B8),
        ] {
            assert_eq!(Crc16::of(frame), Crc16(expected), "frame {frame:02X?}");
        }
    }

    #[test]
    fn the_empty_message_is_the_initial_value() {
        assert_eq!(Crc16::of(&[]), Crc16::INIT);
    }

    #[test]
    fn a_frame_carrying_its_own_crc_has_residue_zero() {
        // The property a receiver relies on, checked over a range of inputs
        // rather than at one point. This is the strongest statement available
        // about the implementation without a second published vector.
        for length in 1_usize..=64 {
            let message: Vec<u8> = (0..length).map(|i| (i * 31 + 7) as u8).collect();
            let mut framed = message.clone();
            framed.extend_from_slice(&Crc16::of(&message).to_wire());
            assert!(
                Crc16::residue_ok(&framed),
                "length {length} should verify: {framed:02X?}"
            );
        }
    }

    #[test]
    fn a_single_flipped_bit_is_detected() {
        let message = [0x11_u8, 0x03, 0x00, 0x6B, 0x00, 0x03];
        let mut framed = message.to_vec();
        framed.extend_from_slice(&Crc16::of(&message).to_wire());
        for byte in 0..framed.len() {
            for bit in 0..8 {
                let mut corrupted = framed.clone();
                corrupted[byte] ^= 1 << bit;
                assert!(
                    !Crc16::residue_ok(&corrupted),
                    "flipping bit {bit} of byte {byte} went undetected"
                );
            }
        }
    }

    #[test]
    fn the_wire_order_round_trips() {
        for value in [0x0000_u16, 0x1241, 0x4B37, 0xFFFF, 0x00FF, 0xFF00] {
            assert_eq!(Crc16::from_wire(Crc16(value).to_wire()), Crc16(value));
        }
    }

    #[test]
    fn the_wire_order_is_low_byte_first_and_not_big_endian() {
        // Stated as its own test because it is the single most common defect
        // in a from-scratch implementation: every other field in Modbus is
        // big-endian, and this one is not.
        let crc = Crc16(0x1241);
        assert_eq!(crc.to_wire(), [0x41, 0x12]);
        assert_ne!(crc.to_wire(), crc.0.to_be_bytes());
    }

    #[test]
    fn a_frame_too_short_to_hold_a_crc_is_not_intact() {
        assert!(!Crc16::residue_ok(&[]));
        assert!(!Crc16::residue_ok(&[0x00]));
    }

    #[test]
    fn the_table_driven_form_agrees_with_this_one() {
        // The specification also publishes a table-driven implementation. The
        // two must agree on every byte string; disagreement means one of them
        // was transcribed wrongly. Building the table from the bitwise form
        // and checking it back is the differential check in miniature — the
        // fuzz target does the same over arbitrary input.
        let mut table = [0_u16; 256];
        for (byte, entry) in table.iter_mut().enumerate() {
            let mut crc = byte as u16;
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xA001
                } else {
                    crc >> 1
                };
            }
            *entry = crc;
        }
        let table_driven = |bytes: &[u8]| {
            let mut crc = 0xFFFF_u16;
            for byte in bytes {
                let index = usize::from((crc ^ u16::from(*byte)) as u8);
                crc = (crc >> 8) ^ table[index];
            }
            crc
        };
        for length in 0_usize..=48 {
            let message: Vec<u8> = (0..length).map(|i| (i * 17 + 3) as u8).collect();
            assert_eq!(
                Crc16::of(&message).0,
                table_driven(&message),
                "length {length}"
            );
        }
    }
}
