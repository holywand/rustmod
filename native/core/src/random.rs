use crate::md5;

const GOLDEN_RATIO_64: u64 = 0x9E37_79B9_7F4A_7C15;
const SILVER_RATIO_64: u64 = 0x6A09_E667_F3BC_C909;

pub fn mix_stafford13(value: u64) -> u64 {
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn upgrade_seed_to_128bit(seed: i64) -> (u64, u64) {
    let low = (seed as u64) ^ SILVER_RATIO_64;
    let high = low.wrapping_add(GOLDEN_RATIO_64);
    (mix_stafford13(low), mix_stafford13(high))
}

pub fn seed_from_hash_of(name: &str) -> (u64, u64) {
    let hash = md5::digest(name.as_bytes());
    let mut low = [0u8; 8];
    let mut high = [0u8; 8];
    low.copy_from_slice(&hash[..8]);
    high.copy_from_slice(&hash[8..]);
    (u64::from_be_bytes(low), u64::from_be_bytes(high))
}

pub fn position_seed(x: i32, y: i32, z: i32) -> i64 {
    let mixed = i64::from(x.wrapping_mul(3_129_871))
        ^ i64::from(z).wrapping_mul(116_129_781)
        ^ i64::from(y);
    let scrambled = mixed
        .wrapping_mul(mixed)
        .wrapping_mul(42_317_861)
        .wrapping_add(mixed.wrapping_mul(11));
    scrambled >> 16
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xoroshiro {
    low: u64,
    high: u64,
}

impl Xoroshiro {
    pub fn from_seed(seed: i64) -> Self {
        let (low, high) = upgrade_seed_to_128bit(seed);
        Self::from_seeds(low, high)
    }

    pub fn from_seeds(low: u64, high: u64) -> Self {
        if low | high == 0 {
            Self {
                low: GOLDEN_RATIO_64,
                high: SILVER_RATIO_64,
            }
        } else {
            Self { low, high }
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let low = self.low;
        let mut high = self.high;
        let result = low.wrapping_add(high).rotate_left(17).wrapping_add(low);
        high ^= low;
        self.low = low.rotate_left(49) ^ high ^ (high << 21);
        self.high = high.rotate_left(28);
        result
    }

    pub fn next_long(&mut self) -> i64 {
        self.next_u64() as i64
    }

    pub fn next_int(&mut self) -> i32 {
        self.next_u64() as i32
    }

    pub fn next_int_bounded(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let bound_wide = bound as u64;
        let mut sample = u64::from(self.next_u64() as u32);
        let mut product = sample * bound_wide;
        let mut remainder = product & 0xFFFF_FFFF;
        if remainder < bound_wide {
            let threshold = u64::from((!(bound as u32)).wrapping_add(1) % (bound as u32));
            while remainder < threshold {
                sample = u64::from(self.next_u64() as u32);
                product = sample * bound_wide;
                remainder = product & 0xFFFF_FFFF;
            }
        }
        (product >> 32) as i32
    }

    pub fn next_boolean(&mut self) -> bool {
        self.next_u64() & 1 != 0
    }

    pub fn next_float(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * 5.960_464_5e-8_f32
    }

    pub fn next_double(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * 1.110_223_024_625_156_5e-16_f64
    }

    pub fn consume(&mut self, count: usize) {
        for _ in 0..count {
            self.next_u64();
        }
    }

    pub fn fork(&mut self) -> Self {
        let low = self.next_u64();
        let high = self.next_u64();
        Self::from_seeds(low, high)
    }

    pub fn fork_positional(&mut self) -> PositionalFactory {
        let low = self.next_u64();
        let high = self.next_u64();
        PositionalFactory { low, high }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PositionalFactory {
    low: u64,
    high: u64,
}

impl PositionalFactory {
    pub fn new(low: u64, high: u64) -> Self {
        Self { low, high }
    }

    pub fn at(&self, x: i32, y: i32, z: i32) -> Xoroshiro {
        let mixed = position_seed(x, y, z) as u64;
        Xoroshiro::from_seeds(mixed ^ self.low, self.high)
    }

    pub fn from_hash_of(&self, name: &str) -> Xoroshiro {
        let (low, high) = seed_from_hash_of(name);
        Xoroshiro::from_seeds(low ^ self.low, high ^ self.high)
    }

    pub fn seeds(&self) -> (u64, u64) {
        (self.low, self.high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut first = Xoroshiro::from_seed(0x1234_5678_9ABC_DEF0);
        let mut second = Xoroshiro::from_seed(0x1234_5678_9ABC_DEF0);
        for _ in 0..1000 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }

    #[test]
    fn zero_state_is_replaced() {
        let mut generator = Xoroshiro::from_seeds(0, 0);
        assert_ne!(generator.next_u64(), 0);
    }

    #[test]
    fn stafford_mix_known_values() {
        assert_eq!(mix_stafford13(0), 0);
        assert_ne!(mix_stafford13(1), mix_stafford13(2));
    }

    #[test]
    fn bounded_ints_stay_in_range() {
        let mut generator = Xoroshiro::from_seed(77);
        for bound in [1, 2, 3, 7, 16, 255, 256, 1000, i32::MAX] {
            for _ in 0..200 {
                let value = generator.next_int_bounded(bound);
                assert!(value >= 0 && value < bound);
            }
        }
        assert_eq!(generator.next_int_bounded(0), 0);
    }

    #[test]
    fn floats_stay_in_unit_interval() {
        let mut generator = Xoroshiro::from_seed(-1);
        for _ in 0..10_000 {
            let double = generator.next_double();
            let float = generator.next_float();
            assert!((0.0..1.0).contains(&double));
            assert!((0.0..1.0).contains(&float));
        }
    }

    #[test]
    fn position_seed_matches_manual_expansion() {
        let (x, y, z) = (-17_i32, 63_i32, 4_096_i32);
        let mut expected = i64::from(x.wrapping_mul(3_129_871))
            ^ i64::from(z).wrapping_mul(116_129_781)
            ^ i64::from(y);
        expected = expected
            .wrapping_mul(expected)
            .wrapping_mul(42_317_861)
            .wrapping_add(expected.wrapping_mul(11));
        assert_eq!(position_seed(x, y, z), expected >> 16);
    }

    #[test]
    fn positional_factory_is_stateless() {
        let factory = Xoroshiro::from_seed(42).fork_positional();
        let mut first = factory.at(10, 20, 30);
        let mut second = factory.at(10, 20, 30);
        assert_eq!(first.next_u64(), second.next_u64());
        assert_ne!(factory.at(10, 20, 30), factory.at(10, 20, 31));
    }

    #[test]
    fn hash_seed_is_big_endian_md5_halves() {
        let (low, high) = seed_from_hash_of("minecraft:offset");
        let digest = crate::md5::digest(b"minecraft:offset");
        assert_eq!(low, u64::from_be_bytes(digest[..8].try_into().unwrap()));
        assert_eq!(high, u64::from_be_bytes(digest[8..].try_into().unwrap()));
        let factory = PositionalFactory::new(1, 2);
        assert_eq!(factory.from_hash_of("a"), factory.from_hash_of("a"));
        assert_ne!(factory.from_hash_of("a"), factory.from_hash_of("b"));
    }
}
