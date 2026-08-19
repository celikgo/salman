// SPDX-License-Identifier: Apache-2.0
//! The deterministic pseudo-random generator salman records the seed of.
//!
//! # Why this is written out here rather than pulled from a crate
//!
//! salman promises that the same project, with the same inputs and the same
//! seed, produces an identical trace on Linux, macOS and Windows. A generator
//! is part of that promise the moment anything stochastic exists — jitter on a
//! simulated scan time, packet loss in a fieldbus model, a fuzzer's choice of
//! the next mutation.
//!
//! The obvious dependency does not carry that guarantee. The Rand book
//! designates `rand::rngs::StdRng` and `rand::rngs::SmallRng` as **not
//! portable**: their underlying algorithms are explicitly allowed to change
//! between releases, and `SmallRng` may differ between a 32-bit and a 64-bit
//! target in the same release. Either change is invisible at the type level and
//! would silently break every recorded trace salman had published. Reference:
//! <https://rust-random.github.io/book/guide-rngs.html>
//!
//! So salman names its algorithm, pins it, and holds it to committed
//! known-answer tests:
//!
//! * **splitmix64** expands the one-word seed into generator state. Reference:
//!   <https://prng.di.unimi.it/splitmix64.c>
//! * **xoshiro256++ 1.0** is the generator itself. Reference:
//!   <https://prng.di.unimi.it/xoshiro256plusplus.c>
//!
//! Both are by Blackman and Vigna and are dedicated to the public domain
//! (CC0). Both are pure 64-bit integer arithmetic — rotations, shifts, xors,
//! wrapping adds and multiplies — so nothing here can differ between targets.
//!
//! # What this is not
//!
//! Not cryptographic. xoshiro256++ is trivially predictable from a handful of
//! outputs, and its state can be recovered. Nothing in salman may use it to
//! generate a key, a nonce, a token or a password. It exists to make a
//! *simulation* repeatable, which is the opposite requirement.

/// A deterministic, seeded, portable pseudo-random generator.
///
/// The algorithm is **xoshiro256++ 1.0**, seeded by four successive splitmix64
/// outputs. It is pinned: changing it changes every trace salman has ever
/// emitted, so it may only change under an ADR and a version bump.
///
/// The original seed is kept alongside the expanded state because state
/// expansion is one-way — you cannot recover the seed from `[u64; 4]` — and
/// every salman trace header records the seed that produced it. Without
/// [`Rng::seed`] a run could not describe how to reproduce itself.
///
/// There is deliberately **no `Default`**. A default-seeded generator is the
/// mechanism by which a run stops being reproducible: it makes the seed
/// something the caller forgot rather than something the caller chose. Callers
/// must name a seed, and salman writes that seed down.
///
/// `Copy` is deliberately not derived either. Copying a generator silently
/// forks the stream, so that two call sites draw the identical sequence while
/// appearing independent. [`Clone`] is available for the cases where that fork
/// is the point — checkpointing a simulation, say — where it has to be typed
/// out.
#[derive(Debug, Clone)]
pub struct Rng {
    /// The seed as the caller gave it, for the trace header.
    seed: u64,
    /// xoshiro256++ state. Never all zero; see [`Rng::from_seed`].
    s: [u64; 4],
}

impl Rng {
    /// Builds a generator from a seed.
    ///
    /// The four state words are successive splitmix64 outputs starting from
    /// `seed`, which is the seeding procedure the xoshiro authors recommend:
    /// it turns a seed with little entropy (`0`, `1`, a run number) into state
    /// that is already well mixed, so the first outputs are not visibly
    /// correlated between nearby seeds.
    ///
    /// The all-zero state, which xoshiro cannot escape, is unreachable here.
    /// splitmix64 is a bijection from its internal state to its output, so
    /// exactly one internal state yields `0`; the four internal states used
    /// here are distinct (each is the previous plus a fixed odd increment), so
    /// at most one of the four words can be zero.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        let mut state = seed;
        let a = splitmix64(&mut state);
        let b = splitmix64(&mut state);
        let c = splitmix64(&mut state);
        let d = splitmix64(&mut state);
        Self {
            seed,
            s: [a, b, c, d],
        }
    }

    /// The seed this generator was built from.
    ///
    /// Recorded in every trace header: it is the whole of what a reader needs
    /// in order to reproduce the run.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// The next 64 bits.
    ///
    /// This is xoshiro256++ `next()` transcribed from the reference C at
    /// <https://prng.di.unimi.it/xoshiro256plusplus.c>, with C's implicit
    /// unsigned wrap-around written out as `wrapping_add`.
    pub const fn next_u64(&mut self) -> u64 {
        let result = self.s[0]
            .wrapping_add(self.s[3])
            .rotate_left(23)
            .wrapping_add(self.s[0]);

        let t = self.s[1] << 17;

        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];

        self.s[2] ^= t;

        self.s[3] = self.s[3].rotate_left(45);

        result
    }

    /// The next 32 bits, taken as the **high** half of one 64-bit draw.
    ///
    /// One draw, not two, so that a caller cannot change how far the stream
    /// advances by switching between `next_u32` and `next_u64`. The high half
    /// is the conventional choice: it is the half that stays good for the
    /// `+`-scrambled members of the xoshiro family, and matching that
    /// convention costs nothing here.
    pub const fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// A boolean, from the top bit of one draw.
    ///
    /// The top bit rather than the bottom one for the same reason as
    /// [`Rng::next_u32`]. xoshiro256++ passes BigCrush on every bit position,
    /// so this is convention rather than necessity — but the convention is the
    /// one that survives a change of scrambler.
    pub const fn next_bool(&mut self) -> bool {
        self.next_u64() >> 63 == 1
    }

    /// A uniformly distributed value in `0..bound`, with **no modulo bias**.
    ///
    /// Returns `0` when `bound` is `0`. There is no meaningful value to return
    /// for an empty range, and panicking is not an option salman has: a panic
    /// in a simulation loses the run. `0` is the documented, tested answer, and
    /// a caller that cares must check the bound itself.
    ///
    /// # Why not `next_u64() % bound`
    ///
    /// Because `2^64` is not a multiple of most bounds, so the low residues
    /// come up slightly more often. The bias is tiny for small bounds and
    /// enormous for large ones — but "tiny" is the dangerous case: a simulation
    /// modelling 1 % packet loss would drop a consistently wrong number of
    /// packets, in the same direction, in every run, and the result would look
    /// entirely plausible. Determinism makes a systematic error *reproducible*,
    /// not absent.
    ///
    /// # The method
    ///
    /// Lemire's "nearly divisionless" multiply-and-reject
    /// (<https://arxiv.org/abs/1805.10941>): multiply the draw by `bound` into
    /// 128 bits and take the high half, which maps `2^64` inputs onto `bound`
    /// buckets. The buckets differ in size by at most one input, and the low
    /// half of the product says which inputs are in the overhanging part; those
    /// are rejected and redrawn. The threshold division is computed only on the
    /// rare path where rejection is even possible, so the common case costs one
    /// multiply.
    ///
    /// Rejection means the number of draws consumed depends on the values
    /// drawn. That is still deterministic — the same seed rejects at the same
    /// points on every machine — but a caller cannot assume one call advances
    /// the stream by exactly one draw.
    pub fn next_below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        let mut product = u128::from(self.next_u64()) * u128::from(bound);
        let mut low = product as u64;
        if low < bound {
            // (2^64) % bound, computed without a 128-bit remainder.
            let threshold = bound.wrapping_neg() % bound;
            while low < threshold {
                product = u128::from(self.next_u64()) * u128::from(bound);
                low = product as u64;
            }
        }
        (product >> 64) as u64
    }

    /// A value in `[0, 1)`, built from the top 53 bits of one draw.
    ///
    /// 53 is the number of bits in an `f64` significand. Taking exactly that
    /// many and scaling by `2^-53` gives `2^53` equally likely values, evenly
    /// spaced by exactly `2^-53`, every one of which is representable without
    /// rounding. Both halves of that matter:
    ///
    /// * Using more bits — dividing a full `u64` by `2^64`, say — forces a
    ///   rounding step, which makes some outputs twice as likely as their
    ///   neighbours and can round up to exactly `1.0`, breaking the half-open
    ///   range that every caller assumes.
    /// * Using the *top* bits keeps the spacing uniform. Constructions that
    ///   fill the significand of a fixed exponent and subtract produce values
    ///   that are dense near zero and sparse near one.
    ///
    /// The multiply is exact — both operands are exactly representable and the
    /// product needs at most 53 significant bits — so the result is identical
    /// on every IEEE 754 target regardless of rounding mode surprises.
    pub const fn next_f64_unit(&mut self) -> f64 {
        /// 2^-53, exactly representable.
        const SCALE: f64 = 1.0 / 9_007_199_254_740_992.0;
        (self.next_u64() >> 11) as f64 * SCALE
    }
}

/// One splitmix64 step: advances `state` and returns the mixed output.
///
/// Transcribed from the reference C at <https://prng.di.unimi.it/splitmix64.c>,
/// which is the fixed-increment form of Java 8's `SplittableRandom`. Used only
/// to expand a seed into xoshiro state, never as the generator itself.
const fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_reproduces_the_published_vectors_from_state_zero() {
        // Known-answer vectors for the reference splitmix64 starting from
        // state 0. If these ever change, the seeding of every salman run has
        // changed with them.
        let expected = [
            0xe220_a839_7b1d_cdaf_u64,
            0x6e78_9e6a_a1b9_65f4,
            0x06c4_5d18_8009_454f,
            0xf88b_b8a8_724c_81ec,
            0x1b39_896a_51a8_749b,
            0x53cb_9f0c_747e_a2ea,
        ];
        let mut state = 0u64;
        for (i, want) in expected.into_iter().enumerate() {
            assert_eq!(splitmix64(&mut state), want, "splitmix64 output {i}");
        }
    }

    #[test]
    fn xoshiro256plusplus_reproduces_the_published_vectors_for_seed_100() {
        // Known-answer vectors for xoshiro256++ with state seeded by
        // successive splitmix64 outputs from 100. This is the test that pins
        // salman's traces: it covers the seeding and the generator together.
        let expected = [
            16_200_148_097_352_791_549_u64,
            16_785_171_618_027_694_926,
            15_341_217_898_654_479_309,
            6_357_779_920_452_276_603,
            16_218_729_523_867_403_097,
            5_738_169_174_581_742_609,
        ];
        let mut rng = Rng::from_seed(100);
        for (i, want) in expected.into_iter().enumerate() {
            assert_eq!(rng.next_u64(), want, "xoshiro256++ output {i}");
        }
    }

    #[test]
    fn the_same_seed_produces_the_same_sequence() {
        for seed in [0u64, 1, 42, u64::MAX, 0x0123_4567_89ab_cdef] {
            let mut a = Rng::from_seed(seed);
            let mut b = Rng::from_seed(seed);
            for i in 0..1_000 {
                assert_eq!(a.next_u64(), b.next_u64(), "seed {seed}, draw {i}");
            }
        }
    }

    #[test]
    fn a_clone_continues_the_same_stream_from_where_it_was_taken() {
        let mut original = Rng::from_seed(7);
        for _ in 0..10 {
            let _ = original.next_u64();
        }
        let mut forked = original.clone();
        for i in 0..100 {
            assert_eq!(original.next_u64(), forked.next_u64(), "draw {i}");
        }
    }

    #[test]
    fn different_seeds_diverge() {
        // Adjacent seeds are the interesting case: splitmix64 exists precisely
        // so that seeds 1 and 2 do not produce visibly related streams.
        let mut a = Rng::from_seed(1);
        let mut b = Rng::from_seed(2);
        let first: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        let second: Vec<u64> = (0..8).map(|_| b.next_u64()).collect();
        assert_ne!(first, second);
        // Not one value in common, let alone a shifted stream.
        for x in &first {
            assert!(!second.contains(x), "seeds 1 and 2 share the draw {x}");
        }
    }

    #[test]
    fn the_seed_is_readable_back_because_every_trace_header_records_it() {
        let mut rng = Rng::from_seed(0xdead_beef);
        assert_eq!(rng.seed(), 0xdead_beef);
        for _ in 0..100 {
            let _ = rng.next_u64();
        }
        // Drawing must not disturb it: the header is written after the run.
        assert_eq!(rng.seed(), 0xdead_beef);
    }

    #[test]
    fn next_below_never_reaches_its_bound() {
        // Powers of two, primes, awkward non-divisors of 2^64, and the
        // degenerate bound 1.
        let bounds = [
            1u64,
            2,
            3,
            7,
            10,
            17,
            100,
            1_000,
            65_537,
            (1 << 32) + 1,
            u64::MAX / 3,
            u64::MAX - 1,
            u64::MAX,
        ];
        let mut rng = Rng::from_seed(20_240_229);
        for bound in bounds {
            for _ in 0..2_000 {
                let v = rng.next_below(bound);
                assert!(v < bound, "next_below({bound}) returned {v}");
            }
        }
    }

    #[test]
    fn next_below_one_is_always_zero() {
        let mut rng = Rng::from_seed(5);
        for _ in 0..1_000 {
            assert_eq!(rng.next_below(1), 0);
        }
    }

    #[test]
    fn next_below_zero_returns_zero_rather_than_panicking() {
        // An empty range has no correct answer. salman denies panics in
        // library code, so this returns 0 and says so.
        let mut rng = Rng::from_seed(9);
        assert_eq!(rng.next_below(0), 0);
        // The stream is untouched, so a bad bound cannot desynchronise a run.
        let mut reference = Rng::from_seed(9);
        assert_eq!(rng.next_u64(), reference.next_u64());
    }

    #[test]
    fn next_below_is_not_visibly_biased_towards_the_low_residues() {
        // The failure this guards against is `next_u64() % bound`. Three
        // buckets over 300 000 draws: an unbiased generator lands within a
        // couple of per mille of a third each time. The seed is fixed, so the
        // counts are the same on every machine and this cannot flake.
        const DRAWS: u64 = 300_000;
        let mut counts = [0u64; 3];
        let mut rng = Rng::from_seed(31_337);
        for _ in 0..DRAWS {
            let v = rng.next_below(3);
            counts[v as usize] += 1;
        }
        for (bucket, count) in counts.into_iter().enumerate() {
            let expected = DRAWS / 3;
            let deviation = count.abs_diff(expected);
            assert!(
                deviation * 100 < expected,
                "bucket {bucket} got {count}, expected about {expected}"
            );
        }
    }

    #[test]
    fn next_f64_unit_is_always_in_the_half_open_unit_interval() {
        let mut rng = Rng::from_seed(2_024);
        let mut min = f64::MAX;
        let mut max = f64::MIN;
        for _ in 0..200_000 {
            let x = rng.next_f64_unit();
            assert!(x >= 0.0, "next_f64_unit returned {x}");
            assert!(x < 1.0, "next_f64_unit returned {x}");
            assert!(x.is_finite());
            min = min.min(x);
            max = max.max(x);
        }
        // Both ends of the interval are actually reached, so the test above is
        // not passing merely because the values are clustered in the middle.
        assert!(min < 0.01, "smallest draw was {min}");
        assert!(max > 0.99, "largest draw was {max}");
    }

    #[test]
    fn next_f64_unit_produces_values_spaced_by_a_multiple_of_two_to_the_minus_53() {
        // The point of the 53-bit construction: every output is an exact
        // multiple of 2^-53, so no output is a rounded approximation of a
        // value the generator could not represent.
        let mut rng = Rng::from_seed(11);
        for _ in 0..10_000 {
            let x = rng.next_f64_unit();
            let scaled = x * 9_007_199_254_740_992.0;
            assert!(
                (scaled - scaled.round()).abs() < f64::EPSILON,
                "{x} is not a multiple"
            );
        }
    }

    #[test]
    fn next_u32_and_next_bool_take_the_high_bits_of_one_draw() {
        // Pinned because it is observable: it fixes how far the stream
        // advances and which bits a caller sees.
        let mut wide = Rng::from_seed(77);
        let mut narrow = Rng::from_seed(77);
        for i in 0..100 {
            let draw = wide.next_u64();
            assert_eq!(narrow.next_u32(), (draw >> 32) as u32, "draw {i}");
        }
        let mut wide = Rng::from_seed(77);
        let mut flags = Rng::from_seed(77);
        for i in 0..100 {
            let draw = wide.next_u64();
            assert_eq!(flags.next_bool(), draw >> 63 == 1, "draw {i}");
        }
    }

    #[test]
    fn next_bool_returns_both_answers_at_roughly_equal_rates() {
        let mut rng = Rng::from_seed(4_242);
        let trues = (0..100_000).filter(|_| rng.next_bool()).count();
        assert!(
            (49_000..=51_000).contains(&trues),
            "{trues} true out of 100 000"
        );
    }
}
