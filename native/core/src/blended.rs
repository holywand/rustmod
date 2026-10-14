use crate::noise::{wrap, PerlinNoise};
use crate::random::Xoroshiro;

const BASE_SCALE: f64 = 684.412;

#[inline]
fn clamped_lerp(start: f64, end: f64, delta: f64) -> f64 {
    if delta < 0.0 {
        start
    } else if delta > 1.0 {
        end
    } else {
        start + delta * (end - start)
    }
}

#[derive(Clone, Debug)]
pub struct BlendedNoise {
    min_limit: PerlinNoise,
    max_limit: PerlinNoise,
    main: PerlinNoise,
    xz_multiplier: f64,
    y_multiplier: f64,
    xz_factor: f64,
    y_factor: f64,
    smear_scale_multiplier: f64,
    max_value: f64,
}

impl BlendedNoise {
    pub fn new(
        random: &mut Xoroshiro,
        xz_scale: f64,
        y_scale: f64,
        xz_factor: f64,
        y_factor: f64,
        smear_scale_multiplier: f64,
    ) -> Self {
        let min_limit = PerlinNoise::legacy(random, -15, 16);
        let max_limit = PerlinNoise::legacy(random, -15, 16);
        let main = PerlinNoise::legacy(random, -7, 8);
        let y_multiplier = BASE_SCALE * y_scale;
        let max_value = min_limit.max_broken_value(y_multiplier);
        Self {
            min_limit,
            max_limit,
            main,
            xz_multiplier: BASE_SCALE * xz_scale,
            y_multiplier,
            xz_factor,
            y_factor,
            smear_scale_multiplier,
            max_value,
        }
    }

    pub fn max_value(&self) -> f64 {
        self.max_value
    }

    pub fn compute(&self, block_x: i32, block_y: i32, block_z: i32) -> f64 {
        let d = f64::from(block_x) * self.xz_multiplier;
        let e = f64::from(block_y) * self.y_multiplier;
        let f = f64::from(block_z) * self.xz_multiplier;
        let g = d / self.xz_factor;
        let h = e / self.y_factor;
        let i = f / self.xz_factor;
        let j = self.y_multiplier * self.smear_scale_multiplier;
        let k = j / self.y_factor;

        let mut low = 0.0;
        let mut high = 0.0;
        let mut main = 0.0;
        let mut frequency = 1.0;
        for octave in 0..8 {
            if let Some(noise) = self.main.octave_noise(octave) {
                main += noise.sample_with_y_limit(
                    wrap(g * frequency),
                    wrap(h * frequency),
                    wrap(i * frequency),
                    k * frequency,
                    h * frequency,
                ) / frequency;
            }
            frequency /= 2.0;
        }

        let blend = (main / 10.0 + 1.0) / 2.0;
        let skip_low = blend >= 1.0;
        let skip_high = blend <= 0.0;
        frequency = 1.0;
        for octave in 0..16 {
            let wrapped_x = wrap(d * frequency);
            let wrapped_y = wrap(e * frequency);
            let wrapped_z = wrap(f * frequency);
            let smear = j * frequency;
            if !skip_low {
                if let Some(noise) = self.min_limit.octave_noise(octave) {
                    low += noise.sample_with_y_limit(
                        wrapped_x,
                        wrapped_y,
                        wrapped_z,
                        smear,
                        e * frequency,
                    ) / frequency;
                }
            }
            if !skip_high {
                if let Some(noise) = self.max_limit.octave_noise(octave) {
                    high += noise.sample_with_y_limit(
                        wrapped_x,
                        wrapped_y,
                        wrapped_z,
                        smear,
                        e * frequency,
                    ) / frequency;
                }
            }
            frequency /= 2.0;
        }

        clamped_lerp(low / 512.0, high / 512.0, blend) / 128.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_noise(seed: i64) -> BlendedNoise {
        BlendedNoise::new(
            &mut Xoroshiro::from_seed(seed),
            0.25,
            0.125,
            80.0,
            160.0,
            8.0,
        )
    }

    #[test]
    fn deterministic_for_same_seed() {
        let first = sample_noise(42);
        let second = sample_noise(42);
        for step in 0..500 {
            let x = step * 7 - 1500;
            assert_eq!(
                first.compute(x, 64 + step % 40, -x),
                second.compute(x, 64 + step % 40, -x)
            );
        }
    }

    #[test]
    fn different_seeds_differ() {
        let first = sample_noise(1);
        let second = sample_noise(2);
        let differs = (0..200).any(|step| {
            first.compute(step * 5, 70, step * 3) != second.compute(step * 5, 70, step * 3)
        });
        assert!(differs);
    }

    #[test]
    fn values_respect_bound() {
        let noise = sample_noise(2026);
        assert!(noise.max_value() > 0.0);
        for step in 0..4000 {
            let x = (step * 37) % 3000 - 1500;
            let z = (step * 91) % 3000 - 1500;
            let y = (step * 13) % 384 - 64;
            let value = noise.compute(x, y, z);
            assert!(value.is_finite());
            assert!(value.abs() <= noise.max_value() + 1.0e-9, "value {value}");
        }
    }

    #[test]
    fn adjacent_blocks_stay_close() {
        let noise = sample_noise(9);
        for step in 0..500 {
            let x = step * 11;
            let a = noise.compute(x, 80, 5);
            let b = noise.compute(x + 1, 80, 5);
            assert!((a - b).abs() < 0.2);
        }
    }

    #[test]
    fn legacy_perlin_octave_order() {
        let noise = PerlinNoise::legacy(&mut Xoroshiro::from_seed(5), -7, 8);
        for octave in 0..8 {
            assert!(noise.octave_noise(octave).is_some());
        }
        assert!(noise.octave_noise(8).is_none());
    }
}
