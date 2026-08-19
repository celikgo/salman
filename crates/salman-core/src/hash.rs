// SPDX-License-Identifier: Apache-2.0
//! SHA-256, written out here rather than taken from a dependency.
//!
//! salman fingerprints simulation traces so that CI can assert the same project
//! produces byte-identical output on Linux, macOS and Windows. The fingerprint
//! is the artefact that gate compares, so the code that computes it has to be
//! auditable in one sitting and has to take the same path on every machine.
//!
//! # Why not a crate
//!
//! * **No runtime CPU-feature dispatch.** A hash that selects an AVX2 backend
//!   at runtime computes the digest with different code on different machines,
//!   which is exactly the property the determinism gate exists to detect.
//!   RUSTSEC-2021-0100 is the documented case: `sha2`'s runtime-dispatched AVX2
//!   backend miscomputed digests for multi-block messages. The bug was fixed,
//!   but the shape of the risk is permanent, and salman would rather not have a
//!   dispatch decision inside the thing that decides whether platforms agree.
//! * **No C toolchain.** `blake3` carries an unconditional `cc`
//!   build-dependency, i.e. it wants a working C compiler on all three
//!   platforms salman gates on.
//! * **It is small and pinned down.** SHA-256 is about 150 lines, is completely
//!   specified by FIPS 180-4, and has published known-answer vectors. The tests
//!   in this module check every vector against the published value; that is
//!   what makes the fingerprint trustworthy, not the fact that it is SHA-256.
//!
//! # This is a fingerprint, not a security primitive
//!
//! Read this before reusing it:
//!
//! * salman does not rely on this for security. It answers "are these two byte
//!   strings the same?" for traces salman itself produced.
//! * It is **not constant-time**, and neither is comparing its output with `==`.
//! * It must not be used anywhere an attacker chooses the input and the
//!   comparison is against a secret — MACs, password or token verification,
//!   signature checking. Those need a real cryptographic library with
//!   constant-time comparison and a reviewed threat model.
//!
//! # Determinism
//!
//! One code path everywhere: no `unsafe`, no dependencies, no feature
//! detection, no `#[cfg(target_arch)]`. Every addition is `wrapping_add`, so
//! debug and release builds agree even with overflow checks on.
//!
//! # Message length
//!
//! FIPS 180-4 defines SHA-256 only for messages shorter than 2^64 bits, because
//! the padding ends in a 64-bit big-endian bit count. This implementation
//! counts bytes in a `u64` with `saturating_add` and converts with
//! `saturating_mul`, so a hasher fed more than 2^64 bits pins its length field
//! at `u64::MAX` instead of wrapping quietly to a small number. The digest then
//! stays deterministic but is no longer the FIPS-180-4 digest of that message.
//! Reaching that point means pushing 2 EiB through a single [`Sha256`], which
//! at 1 GiB/s takes about 68 years; the branch is documented because silence
//! about it would be worse, not because anyone will get there.

use std::fmt::Write as _;

/// Bytes in one SHA-256 message block.
const BLOCK_LEN: usize = 64;

/// Bytes in a SHA-256 digest.
pub const DIGEST_LEN: usize = 32;

/// Initial hash value H(0), FIPS 180-4 §5.3.3: the first 32 bits of the
/// fractional parts of the square roots of the first eight primes.
#[rustfmt::skip]
const INITIAL_STATE: [u32; 8] = [
    0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a,
    0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
];

/// Round constants K, FIPS 180-4 §4.2.2: the first 32 bits of the fractional
/// parts of the cube roots of the first sixty-four primes.
// The grid is four constants per line so it can be read against the table in
// FIPS 180-4 §4.2.2; rustfmt would otherwise put each on its own line.
#[rustfmt::skip]
const K: [u32; 64] = [
    0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5,
    0x3956_c25b, 0x59f1_11f1, 0x923f_82a4, 0xab1c_5ed5,
    0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3,
    0x72be_5d74, 0x80de_b1fe, 0x9bdc_06a7, 0xc19b_f174,
    0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc,
    0x2de9_2c6f, 0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da,
    0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7,
    0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967,
    0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc, 0x5338_0d13,
    0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85,
    0xa2bf_e8a1, 0xa81a_664b, 0xc24b_8b70, 0xc76c_51a3,
    0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070,
    0x19a4_c116, 0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5,
    0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
    0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208,
    0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7, 0xc671_78f2,
];

/// A streaming SHA-256 hasher.
///
/// Feed it with [`update`](Sha256::update) as many times as you like and take
/// the digest with [`finalize`](Sha256::finalize). How the input is split into
/// calls never changes the answer, which is what lets a trace writer hash
/// records as it emits them instead of buffering the whole trace.
///
/// Read the module documentation before using this for anything but a
/// fingerprint: it is not a constant-time implementation.
#[derive(Debug, Clone)]
pub struct Sha256 {
    /// The eight working hash words, H(i) in FIPS 180-4.
    state: [u32; 8],
    /// Bytes of a partial block waiting for the rest of their block.
    buffer: [u8; BLOCK_LEN],
    /// How much of `buffer` is occupied; always below `BLOCK_LEN` between
    /// calls, because a full block is compressed immediately.
    buffered: usize,
    /// Total bytes fed in, saturating. See the module docs on length overflow.
    len_bytes: u64,
}

impl Sha256 {
    /// A hasher over the empty message.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: INITIAL_STATE,
            buffer: [0; BLOCK_LEN],
            buffered: 0,
            len_bytes: 0,
        }
    }

    /// Appends `bytes` to the message being hashed.
    pub fn update(&mut self, bytes: &[u8]) {
        self.len_bytes = self.len_bytes.saturating_add(bytes.len() as u64);

        let mut rest = bytes;

        // Top up a partial block first, so that the block-at-a-time loop below
        // always starts on a block boundary of the message.
        if self.buffered > 0 {
            let take = (BLOCK_LEN - self.buffered).min(rest.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&rest[..take]);
            self.buffered += take;
            rest = &rest[take..];
            if self.buffered < BLOCK_LEN {
                // `take` was capped by `rest.len()`, so falling short of a full
                // block means `rest` is now empty and there is nothing more to
                // do. Returning here matters: the tail handling below assumes
                // it owns `buffered`, and would otherwise reset the partial
                // block that was just added to.
                return;
            }
            compress(&mut self.state, &self.buffer);
            self.buffered = 0;
        }

        let mut blocks = rest.chunks_exact(BLOCK_LEN);
        for block in blocks.by_ref() {
            let mut full = [0u8; BLOCK_LEN];
            full.copy_from_slice(block);
            compress(&mut self.state, &full);
        }

        let tail = blocks.remainder();
        self.buffer[..tail.len()].copy_from_slice(tail);
        self.buffered = tail.len();
    }

    /// Pads the message and returns the 32-byte digest.
    ///
    /// Consumes the hasher: SHA-256 padding mutates the state, so a hasher that
    /// has been finalised is not a hasher any more. Clone it first if you need
    /// the digest of a prefix and then want to keep going.
    #[must_use]
    pub fn finalize(mut self) -> [u8; DIGEST_LEN] {
        // FIPS 180-4 §5.1.1: append 0x80, then the fewest zero bytes that leave
        // room for a 64-bit big-endian bit count at the end of a block.
        let bits = self.len_bytes.saturating_mul(8);
        let mut pad = self.buffered;
        self.buffer[pad] = 0x80;
        pad += 1;
        self.buffer[pad..].fill(0);
        if pad > BLOCK_LEN - 8 {
            // The length no longer fits in this block, so this block goes out
            // as all-padding and the length lands in the next one.
            compress(&mut self.state, &self.buffer);
            self.buffer = [0; BLOCK_LEN];
        }
        self.buffer[BLOCK_LEN - 8..].copy_from_slice(&bits.to_be_bytes());
        compress(&mut self.state, &self.buffer);

        let mut digest = [0u8; DIGEST_LEN];
        for (out, word) in digest.chunks_exact_mut(4).zip(self.state) {
            out.copy_from_slice(&word.to_be_bytes());
        }
        digest
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

/// The SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; DIGEST_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize()
}

/// Renders a digest as 64 lowercase hexadecimal characters.
///
/// Lowercase and unseparated, because this string ends up in trace headers and
/// in CI logs that are compared byte-for-byte; one spelling means one answer.
#[must_use]
pub fn to_hex(digest: &[u8; DIGEST_LEN]) -> String {
    let mut out = String::with_capacity(DIGEST_LEN * 2);
    for byte in digest {
        // Writing to a String is infallible; the Result is discarded for that
        // reason and no other.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// One application of the SHA-256 compression function, FIPS 180-4 §6.2.2.
///
/// The working variables are called `a`..`h` and the schedule `w`, because that
/// is what FIPS 180-4 calls them. Renaming them to something clippy likes would
/// make this harder to check against the specification, which is the only thing
/// that makes it auditable.
#[allow(clippy::many_single_char_names, reason = "the names FIPS 180-4 uses")]
fn compress(state: &mut [u32; 8], block: &[u8; BLOCK_LEN]) {
    let mut w = [0u32; 64];
    for (word, chunk) in w.iter_mut().zip(block.chunks_exact(4)) {
        *word = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (kt, wt) in K.iter().zip(w.iter()) {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(*kt)
            .wrapping_add(*wt);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }

    for (word, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *word = word.wrapping_add(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every digest below was checked against `shasum -a 256` before it was
    /// committed. A known-answer test with an answer nobody verified is worse
    /// than no test, because it certifies whatever the code happened to do.
    fn hex_of(bytes: &[u8]) -> String {
        to_hex(&sha256(bytes))
    }

    #[test]
    fn the_published_fips_180_4_vectors_hash_to_their_published_digests() {
        // NIST's SHA-256 examples: the empty message, "abc", the 448-bit
        // two-block message and the 896-bit message.
        assert_eq!(
            hex_of(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex_of(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex_of(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            hex_of(
                b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmn\
                  hijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"
            ),
            "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
        );
    }

    #[test]
    fn one_million_letter_a_hashes_to_the_published_digest() {
        // The long NIST vector, and the one worth being suspicious about: it is
        // frequently misquoted. This value was produced independently by
        // `shasum -a 256` over one million 'a' bytes before being written here.
        let message = vec![b'a'; 1_000_000];
        assert_eq!(
            hex_of(&message),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn splitting_the_input_into_chunks_never_changes_the_digest() {
        // A message long enough to cross several block boundaries, with a byte
        // pattern that would expose a mixed-up buffer offset.
        let message: Vec<u8> = (0..1000u32).map(|i| (i * 7 + 3) as u8).collect();
        let one_shot = sha256(&message);

        // 1 byte at a time, sizes either side of the 64-byte block, and sizes
        // that share no factor with it.
        for chunk in [1usize, 2, 3, 7, 31, 63, 64, 65, 100, 127, 128, 999, 1000] {
            let mut hasher = Sha256::new();
            for part in message.chunks(chunk) {
                hasher.update(part);
            }
            assert_eq!(
                hasher.finalize(),
                one_shot,
                "chunking by {chunk} changed the digest"
            );
        }
    }

    #[test]
    fn an_empty_update_is_a_no_op_wherever_it_falls() {
        let mut hasher = Sha256::new();
        hasher.update(b"");
        hasher.update(b"ab");
        hasher.update(b"");
        hasher.update(b"c");
        hasher.update(b"");
        assert_eq!(to_hex(&hasher.finalize()), hex_of(b"abc"));
    }

    #[test]
    fn padding_is_right_on_both_sides_of_the_55_56_and_64_byte_boundaries() {
        // 55 bytes is the longest message whose padding still fits its own
        // block; 56 forces a second block that is nothing but padding; 64 is a
        // whole block with no room left at all. Each digest below came from
        // `shasum -a 256` over that many 'a' bytes.
        let cases: [(usize, &str); 11] = [
            (
                54,
                "a3f01b6939256127582ac8ae9fb47a382a244680806a3f613a118851c1ca1d47",
            ),
            (
                55,
                "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
            ),
            (
                56,
                "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
            ),
            (
                57,
                "f13b2d724659eb3bf47f2dd6af1accc87b81f09f59f2b75e5c0bed6589dfe8c6",
            ),
            (
                63,
                "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34",
            ),
            (
                64,
                "ffe054fe7ae0cb6dc65c3af9b61d5209f439851db43d0ba5997337df154668eb",
            ),
            (
                65,
                "635361c48bb9eab14198e76ea8ab7f1a41685d6ad62aa9146d301d4f17eb0ae0",
            ),
            (
                119,
                "31eba51c313a5c08226adf18d4a359cfdfd8d2e816b13f4af952f7ea6584dcfb",
            ),
            (
                120,
                "2f3d335432c70b580af0e8e1b3674a7c020d683aa5f73aaaedfdc55af904c21c",
            ),
            (
                127,
                "c57e9278af78fa3cab38667bef4ce29d783787a2f731d4e12200270f0c32320a",
            ),
            (
                128,
                "6836cf13bac400e9105071cd6af47084dfacad4e5e302c94bfed24e013afb73e",
            ),
        ];
        for (len, expected) in cases {
            let message = vec![b'a'; len];
            assert_eq!(hex_of(&message), expected, "{len} bytes hashed wrong");
        }
    }

    #[test]
    fn hashing_the_bytes_0_through_255_gives_a_stable_digest() {
        // Every byte value, in order, so a sign-extension or byte-order slip in
        // the message schedule shows up. Verified with `shasum -a 256`.
        let message: Vec<u8> = (0..=255u8).collect();
        assert_eq!(
            hex_of(&message),
            "40aff2e9d2d8922e47afd4648e6967497158785fbd1da870e7110266bf944880"
        );
    }

    #[test]
    fn to_hex_is_64_lowercase_hex_characters() {
        let hex = to_hex(&sha256(b"salman"));
        assert_eq!(hex.len(), 64);
        assert!(
            hex.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "{hex} is not lowercase hex"
        );
    }

    #[test]
    fn default_is_a_hasher_over_the_empty_message() {
        assert_eq!(Sha256::default().finalize(), sha256(b""));
    }

    #[test]
    fn a_cloned_hasher_gives_the_digest_of_the_prefix_it_was_cloned_at() {
        let mut hasher = Sha256::new();
        hasher.update(b"abc");
        let prefix = hasher.clone().finalize();
        hasher.update(b"def");
        assert_eq!(to_hex(&prefix), hex_of(b"abc"));
        assert_eq!(to_hex(&hasher.finalize()), hex_of(b"abcdef"));
    }

    #[test]
    fn the_length_counter_saturates_instead_of_wrapping_past_2_pow_64_bits() {
        // Reaching this by hashing would take 2 EiB of input, so the state is
        // set directly. The point is that the counter cannot wrap round to a
        // small number and silently produce a well-formed but wrong digest.
        let mut hasher = Sha256::new();
        hasher.len_bytes = u64::MAX - 1;
        hasher.update(&[0u8; 4]);
        assert_eq!(hasher.len_bytes, u64::MAX);

        // And finalising from there still terminates and is deterministic.
        assert_eq!(hasher.clone().finalize(), hasher.finalize());
    }
}
