//! Directly represented variables — `%IX0.0` and friends.
//!
//! IEC 61131-3 Edition 3 Table 16 gives three locations (`I` input, `Q` output,
//! `M` memory), five sizes (`X` bit, `B` byte, `W` word, `D` double word, `L`
//! long word), an optional size letter meaning a single bit, hierarchical
//! addressing with `.`, and partly specified addresses written `*`.
//!
//! These are lexed as one token rather than reassembled by the parser, because
//! `%QX7.5` would otherwise arrive as an identifier, a dot and a number, and
//! `%IW1.2` would be indistinguishable from a real literal.

use std::fmt;

/// Which part of the process image an address refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AddressLocation {
    /// `%I` — an input.
    Input,
    /// `%Q` — an output.
    Output,
    /// `%M` — internal memory.
    Memory,
}

impl AddressLocation {
    /// The letter used to write it.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::Input => 'I',
            Self::Output => 'Q',
            Self::Memory => 'M',
        }
    }

    /// Parses the location letter, case-insensitively.
    #[must_use]
    pub const fn from_letter(c: u8) -> Option<Self> {
        Some(match c.to_ascii_uppercase() {
            b'I' => Self::Input,
            b'Q' => Self::Output,
            b'M' => Self::Memory,
            _ => return None,
        })
    }
}

/// How wide the addressed datum is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum AddressSize {
    /// `X`, or no size letter at all — a single bit.
    #[default]
    Bit,
    /// `B` — eight bits.
    Byte,
    /// `W` — sixteen bits.
    Word,
    /// `D` — thirty-two bits.
    DoubleWord,
    /// `L` — sixty-four bits.
    LongWord,
}

impl AddressSize {
    /// The letter used to write it; the bit size is written `X`, even though
    /// the standard also permits omitting the letter entirely.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::Bit => 'X',
            Self::Byte => 'B',
            Self::Word => 'W',
            Self::DoubleWord => 'D',
            Self::LongWord => 'L',
        }
    }

    /// Width in bits.
    #[must_use]
    pub const fn bits(self) -> u32 {
        match self {
            Self::Bit => 1,
            Self::Byte => 8,
            Self::Word => 16,
            Self::DoubleWord => 32,
            Self::LongWord => 64,
        }
    }

    /// Parses the size letter, case-insensitively.
    #[must_use]
    pub const fn from_letter(c: u8) -> Option<Self> {
        Some(match c.to_ascii_uppercase() {
            b'X' => Self::Bit,
            b'B' => Self::Byte,
            b'W' => Self::Word,
            b'D' => Self::DoubleWord,
            b'L' => Self::LongWord,
            _ => return None,
        })
    }
}

/// Deepest hierarchical address salman accepts, e.g. `%IX1.2.3.4` is depth 4.
///
/// Bounded because the text is untrusted; four levels is already more than any
/// address seen in the field.
pub const MAX_ADDRESS_DEPTH: usize = 8;

/// A directly represented variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectAddress {
    /// Input, output or memory.
    pub location: AddressLocation,
    /// The datum width.
    pub size: AddressSize,
    /// Whether the size letter was written, or omitted to mean a single bit.
    pub size_letter_written: bool,
    /// The hierarchical index path, or `None` for a partly specified address
    /// written `%I*`, which the configuration is expected to fill in.
    pub path: Option<Vec<u32>>,
}

impl fmt::Display for DirectAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "%{}", self.location.letter())?;
        if self.size_letter_written {
            write!(f, "{}", self.size.letter())?;
        }
        match &self.path {
            None => f.write_str("*"),
            Some(indices) => {
                for (i, index) in indices.iter().enumerate() {
                    if i > 0 {
                        f.write_str(".")?;
                    }
                    write!(f, "{index}")?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_renders_back_the_way_it_was_written() {
        let a = DirectAddress {
            location: AddressLocation::Output,
            size: AddressSize::Bit,
            size_letter_written: true,
            path: Some(vec![7, 5]),
        };
        assert_eq!(a.to_string(), "%QX7.5");
    }

    #[test]
    fn an_omitted_size_letter_stays_omitted_when_rendered() {
        // %I1 and %IX1 mean the same thing, but a formatter that silently
        // rewrote one into the other would produce a diff on every file.
        let a = DirectAddress {
            location: AddressLocation::Input,
            size: AddressSize::Bit,
            size_letter_written: false,
            path: Some(vec![1]),
        };
        assert_eq!(a.to_string(), "%I1");
        assert_eq!(a.size.bits(), 1);
    }

    #[test]
    fn a_partly_specified_address_renders_as_a_star() {
        let a = DirectAddress {
            location: AddressLocation::Memory,
            size: AddressSize::Word,
            size_letter_written: true,
            path: None,
        };
        assert_eq!(a.to_string(), "%MW*");
    }

    #[test]
    fn location_and_size_letters_parse_in_either_case() {
        assert_eq!(
            AddressLocation::from_letter(b'i'),
            Some(AddressLocation::Input)
        );
        assert_eq!(
            AddressLocation::from_letter(b'Q'),
            Some(AddressLocation::Output)
        );
        assert_eq!(AddressLocation::from_letter(b'Z'), None);
        assert_eq!(AddressSize::from_letter(b'w'), Some(AddressSize::Word));
        assert_eq!(AddressSize::from_letter(b'L'), Some(AddressSize::LongWord));
        assert_eq!(AddressSize::from_letter(b'Q'), None);
    }

    #[test]
    fn size_widths_match_table_16() {
        assert_eq!(AddressSize::Bit.bits(), 1);
        assert_eq!(AddressSize::Byte.bits(), 8);
        assert_eq!(AddressSize::Word.bits(), 16);
        assert_eq!(AddressSize::DoubleWord.bits(), 32);
        assert_eq!(AddressSize::LongWord.bits(), 64);
    }
}
