//! A seeded random number generator that stays the same across versions.
//!
//! SplitMix64, written out here rather than taken from `rand`, so a seed in a
//! failing test's name produces the same log on every machine and after
//! every dependency bump.

/// SplitMix64: one `u64` of state, a full period of 2^64, and good enough
/// spread for picking test input. Not for anything that needs to be secure.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// An independent stream for the `index`th child of this seed, so session
    /// 3 of 8 does not change when session 2 asks for more bytes.
    pub fn derive(seed: u64, index: u64) -> Self {
        let mut r = Self(seed ^ index.wrapping_mul(0xa076_1d64_78bd_642f));
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`; `n` of zero gives zero. The modulo bias is far below
    /// anything a test could notice for the small ranges used here.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        debug_assert!(lo <= hi);
        lo + self.below(hi - lo + 1)
    }

    /// `usize` in `0..n`.
    pub fn index(&mut self, n: usize) -> usize {
        // n fits in u64 on every supported target, and the result is < n.
        self.below(n as u64) as usize
    }

    /// True with probability `num / den`.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }

    pub fn pick<'a, T: ?Sized>(&mut self, xs: &[&'a T]) -> &'a T {
        xs[self.index(xs.len())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values_never_change() {
        // The reference SplitMix64 output for seed 0: a log generated from a
        // seed today must be the same log next year.
        let mut r = Rng::new(0);
        assert_eq!(r.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(r.next_u64(), 0x6e78_9e6a_a1b9_65f4);
    }

    #[test]
    fn derived_streams_differ() {
        let a = Rng::derive(7, 0).next_u64();
        let b = Rng::derive(7, 1).next_u64();
        assert_ne!(a, b);
        assert_eq!(a, Rng::derive(7, 0).next_u64());
    }

    #[test]
    fn ranges_stay_inside() {
        let mut r = Rng::new(3);
        for _ in 0..10_000 {
            let v = r.range(5, 9);
            assert!((5..=9).contains(&v));
        }
        assert_eq!(r.below(0), 0);
    }
}
