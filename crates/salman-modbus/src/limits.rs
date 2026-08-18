// SPDX-License-Identifier: Apache-2.0
//! The size limits Modbus imposes, and where each one comes from.
//!
//! Every limit here fits in a protocol data unit, and the tests below check
//! the arithmetic rather than trusting the transcription. Two of them are
//! *exactly* the largest value that fits — one more register does not — and
//! two are not.
//!
//! The register limits are forced: 125 read and 123 written are what a PDU
//! holds, and they could not have been anything else. The **bit** limits are
//! round numbers APS chose below the ceiling: 2000 coils fit in a read
//! response with three bytes to spare, and so would 2008. salman uses the
//! specification's numbers rather than the arithmetic maxima, because a server
//! that accepted 2008 would be accepting what the standard does not permit.
//!
//! An earlier version of this paragraph claimed all four limits were forced
//! arithmetic, which the test `the_bit_limits_are_round_numbers_below_what_would_fit`
//! twelve lines below disproves.

/// The largest protocol data unit, in bytes. APS §4.1.
///
/// 253 = 256 − 1 address byte − 2 CRC bytes. The serial line's frame size is
/// where the number comes from, and TCP inherited it so that a gateway never
/// has to fragment.
pub const MAX_PDU: usize = 253;

/// The largest data field, in bytes: the PDU less its function code.
pub const MAX_PDU_DATA: usize = MAX_PDU - 1;

/// The largest RTU application data unit: address + PDU + CRC. APS §4.1.
pub const MAX_RTU_ADU: usize = 1 + MAX_PDU + 2;

/// The MBAP header, in bytes: transaction, protocol, length, unit. MG §3.1.3.
pub const MBAP_HEADER: usize = 7;

/// The largest Modbus TCP application data unit: MBAP header + PDU.
pub const MAX_TCP_ADU: usize = MBAP_HEADER + MAX_PDU;

/// The registered port for Modbus TCP. MG §4.2, IANA service name `mbap`.
pub const TCP_PORT: u16 = 502;

/// The largest number of coils or discrete inputs one read may ask for.
/// APS §6.1 and §6.2.
pub const MAX_READ_BITS: u16 = 2000;

/// The largest number of holding or input registers one read may ask for.
/// APS §6.3 and §6.4.
pub const MAX_READ_REGISTERS: u16 = 125;

/// The largest number of coils one write may carry. APS §6.11.
pub const MAX_WRITE_BITS: u16 = 1968;

/// The largest number of registers one write may carry. APS §6.12.
pub const MAX_WRITE_REGISTERS: u16 = 123;

/// The value that turns a coil on in Write Single Coil. APS §6.5.
///
/// The function takes exactly two values and no others: `0xFF00` and
/// `0x0000`. Anything else is an illegal data value, not a truthy test.
pub const COIL_ON: u16 = 0xFF00;

/// The value that turns a coil off in Write Single Coil. APS §6.5.
pub const COIL_OFF: u16 = 0x0000;

/// How many bytes hold `count` packed bits.
#[must_use]
pub const fn packed_bytes(count: u16) -> usize {
    (count as usize).div_ceil(8)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        MAX_PDU, MAX_READ_BITS, MAX_READ_REGISTERS, MAX_RTU_ADU, MAX_TCP_ADU, MAX_WRITE_BITS,
        MAX_WRITE_REGISTERS, MBAP_HEADER, packed_bytes,
    };

    /// A read response: function code, byte count, then the data.
    const fn read_response_size(data_bytes: usize) -> usize {
        1 + 1 + data_bytes
    }

    /// A multiple-write request: function code, address, quantity, byte count,
    /// then the data.
    const fn write_request_size(data_bytes: usize) -> usize {
        1 + 2 + 2 + 1 + data_bytes
    }

    #[test]
    fn every_quantity_limit_fits_in_a_pdu() {
        // A limit that did not fit would be a mistranscription, and it would
        // show up as a truncated frame on a real wire rather than here.
        assert!(read_response_size(packed_bytes(MAX_READ_BITS)) <= MAX_PDU);
        assert!(read_response_size(2 * MAX_READ_REGISTERS as usize) <= MAX_PDU);
        assert!(write_request_size(packed_bytes(MAX_WRITE_BITS)) <= MAX_PDU);
        assert!(write_request_size(2 * MAX_WRITE_REGISTERS as usize) <= MAX_PDU);
    }

    #[test]
    fn the_register_limits_are_exactly_what_the_pdu_size_allows() {
        // One more register does not fit. These two limits are arithmetic:
        // they could not have been anything else.
        assert!(read_response_size(2 * (MAX_READ_REGISTERS as usize + 1)) > MAX_PDU);
        assert!(write_request_size(2 * (MAX_WRITE_REGISTERS as usize + 1)) > MAX_PDU);
    }

    #[test]
    fn the_bit_limits_are_round_numbers_below_what_would_fit() {
        // Worth its own test because it contradicts a plausible assumption.
        // 2000 and 1968 are not the largest quantities that fit: 2008 bits
        // would still fit in a read response (251 data bytes, 253 total), and
        // 1976 in a write request. APS chose round numbers, and salman uses
        // the specification's numbers rather than the arithmetic maximum —
        // a server that accepted 2008 would be accepting what the standard
        // does not permit.
        assert!(read_response_size(packed_bytes(MAX_READ_BITS + 8)) <= MAX_PDU);
        assert!(write_request_size(packed_bytes(MAX_WRITE_BITS + 8)) <= MAX_PDU);

        // But not without limit: the round number is close to the ceiling.
        assert!(read_response_size(packed_bytes(MAX_READ_BITS + 16)) > MAX_PDU);
        assert!(write_request_size(packed_bytes(MAX_WRITE_BITS + 16)) > MAX_PDU);
    }

    #[test]
    fn the_frame_sizes_agree_with_the_pdu_size() {
        assert_eq!(MAX_RTU_ADU, 256);
        assert_eq!(MAX_TCP_ADU, 260);
        assert_eq!(MAX_PDU, MAX_RTU_ADU - 3);
        assert_eq!(MAX_TCP_ADU, MBAP_HEADER + MAX_PDU);
    }

    #[test]
    fn packing_rounds_up_and_zero_bits_take_no_bytes() {
        assert_eq!(packed_bytes(0), 0);
        assert_eq!(packed_bytes(1), 1);
        assert_eq!(packed_bytes(8), 1);
        assert_eq!(packed_bytes(9), 2);
        assert_eq!(packed_bytes(2000), 250);
    }
}
