use crate::random::Xoroshiro;

const GRADIENT: [[i32; 3]; 16] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
    [1, 1, 0],
    [0, -1, 1],
    [-1, 1, 0],
    [0, -1, -1],
];

const WRAP_PERIOD: f64 = 33554432.0;
const SECOND_LAYER_SCALE: f64 = 1.0181268882175227;
const DEVIATION_BASE: f64 = 0.16666666666666666;

#[inline]
fn floor_i32(value: f64) -> i32 {
    let truncated = value as i32;
    if value < f64::from(truncated) {
        truncated.wrapping_sub(1)
    } else {
        truncated
    }
}

#[inline]
fn floor_i64(value: f64) -> i64 {
    let truncated = value as i64;
    if value < truncated as f64 {
        truncated.wrapping_sub(1)
    } else {
        truncated
    }
}

#[inline]
fn smoothstep(value: f64) -> f64 {
    value * value * value * (value * (value * 6.0 - 15.0) + 10.0)
}

#[inline]
fn lerp(delta: f64, start: f64, end: f64) -> f64 {
    start + delta * (end - start)
}

#[inline]
fn lerp2(dx: f64, dy: f64, v00: f64, v10: f64, v01: f64, v11: f64) -> f64 {
    lerp(dy, lerp(dx, v00, v10), lerp(dx, v01, v11))
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn lerp3(
    dx: f64,
    dy: f64,
    dz: f64,
    v000: f64,
    v100: f64,
    v010: f64,
    v110: f64,
    v001: f64,
    v101: f64,
    v011: f64,
    v111: f64,
) -> f64 {
    lerp(
        dz,
        lerp2(dx, dy, v000, v100, v010, v110),
        lerp2(dx, dy, v001, v101, v011, v111),
    )
}

#[inline]
pub fn wrap(value: f64) -> f64 {
    value - floor_i64(value / WRAP_PERIOD + 0.5) as f64 * WRAP_PERIOD
}

#[derive(Clone, Debug)]
pub struct ImprovedNoise {
    xo: f64,
    yo: f64,
    zo: f64,
    p: [u8; 256],
}

impl ImprovedNoise {
    pub fn new(random: &mut Xoroshiro) -> Self {
        let xo = random.next_double() * 256.0;
        let yo = random.next_double() * 256.0;
        let zo = random.next_double() * 256.0;
        let mut p = [0u8; 256];
        for (index, slot) in p.iter_mut().enumerate() {
            *slot = index as u8;
        }
        for index in 0..256usize {
            let offset = random.next_int_bounded((256 - index) as i32) as usize;
            p.swap(index, index + offset);
        }
        Self { xo, yo, zo, p }
    }

    #[inline]
    pub fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        self.sample_with_y_limit(x, y, z, 0.0, 0.0)
    }

    pub fn sample_with_y_limit(&self, x: f64, y: f64, z: f64, y_scale: f64, y_max: f64) -> f64 {
        let shifted_x = x + self.xo;
        let shifted_y = y + self.yo;
        let shifted_z = z + self.zo;
        let cell_x = floor_i32(shifted_x);
        let cell_y = floor_i32(shifted_y);
        let cell_z = floor_i32(shifted_z);
        let delta_x = shifted_x - f64::from(cell_x);
        let delta_y = shifted_y - f64::from(cell_y);
        let delta_z = shifted_z - f64::from(cell_z);
        let weird_y = if y_scale != 0.0 {
            let limited = if y_max >= 0.0 && y_max < delta_y {
                y_max
            } else {
                delta_y
            };
            f64::from(floor_i32(limited / y_scale + f64::from(1.0e-7_f32))) * y_scale
        } else {
            0.0
        };
        self.sample_and_lerp(
            cell_x,
            cell_y,
            cell_z,
            delta_x,
            delta_y - weird_y,
            delta_z,
            delta_y,
        )
    }

    #[inline]
    fn perm(&self, index: i32) -> i32 {
        i32::from(self.p[(index & 0xFF) as usize])
    }

    #[inline]
    fn grad_dot(hash: i32, x: f64, y: f64, z: f64) -> f64 {
        let gradient = &GRADIENT[(hash & 15) as usize];
        f64::from(gradient[0]) * x + f64::from(gradient[1]) * y + f64::from(gradient[2]) * z
    }

    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn sample_and_lerp(
        &self,
        grid_x: i32,
        grid_y: i32,
        grid_z: i32,
        delta_x: f64,
        weird_delta_y: f64,
        delta_z: f64,
        delta_y: f64,
    ) -> f64 {
        let i = self.perm(grid_x);
        let j = self.perm(grid_x.wrapping_add(1));
        let k = self.perm(i.wrapping_add(grid_y));
        let l = self.perm(i.wrapping_add(grid_y).wrapping_add(1));
        let m = self.perm(j.wrapping_add(grid_y));
        let n = self.perm(j.wrapping_add(grid_y).wrapping_add(1));

        let d = Self::grad_dot(
            self.perm(k.wrapping_add(grid_z)),
            delta_x,
            weird_delta_y,
            delta_z,
        );
        let e = Self::grad_dot(
            self.perm(m.wrapping_add(grid_z)),
            delta_x - 1.0,
            weird_delta_y,
            delta_z,
        );
        let f = Self::grad_dot(
            self.perm(l.wrapping_add(grid_z)),
            delta_x,
            weird_delta_y - 1.0,
            delta_z,
        );
        let g = Self::grad_dot(
            self.perm(n.wrapping_add(grid_z)),
            delta_x - 1.0,
            weird_delta_y - 1.0,
            delta_z,
        );
        let h = Self::grad_dot(
            self.perm(k.wrapping_add(grid_z).wrapping_add(1)),
            delta_x,
            weird_delta_y,
            delta_z - 1.0,
        );
        let o = Self::grad_dot(
            self.perm(m.wrapping_add(grid_z).wrapping_add(1)),
            delta_x - 1.0,
            weird_delta_y,
            delta_z - 1.0,
        );
        let q = Self::grad_dot(
            self.perm(l.wrapping_add(grid_z).wrapping_add(1)),
            delta_x,
            weird_delta_y - 1.0,
            delta_z - 1.0,
        );
        let r = Self::grad_dot(
            self.perm(n.wrapping_add(grid_z).wrapping_add(1)),
            delta_x - 1.0,
            weird_delta_y - 1.0,
            delta_z - 1.0,
        );

        lerp3(
            smoothstep(delta_x),
            smoothstep(delta_y),
            smoothstep(delta_z),
            d,
            e,
            f,
            g,
            h,
            o,
            q,
            r,
        )
    }
}

#[derive(Clone, Debug)]
pub struct PerlinNoise {
    levels: Vec<Option<ImprovedNoise>>,
    amplitudes: Vec<f64>,
    first_octave: i32,
    lowest_freq_input_factor: f64,
    lowest_freq_value_factor: f64,
    max_value: f64,
}

impl PerlinNoise {
    pub fn new(random: &mut Xoroshiro, first_octave: i32, amplitudes: &[f64]) -> Self {
        let count = amplitudes.len();
        let factory = random.fork_positional();
        let mut levels = Vec::with_capacity(count);
        for (index, amplitude) in amplitudes.iter().enumerate() {
            if *amplitude != 0.0 {
                let octave = first_octave + index as i32;
                let mut octave_random = factory.from_hash_of(&format!("octave_{octave}"));
                levels.push(Some(ImprovedNoise::new(&mut octave_random)));
            } else {
                levels.push(None);
            }
        }
        let lowest_freq_input_factor = 2.0_f64.powi(first_octave);
        let lowest_freq_value_factor = if count == 0 {
            0.0
        } else {
            2.0_f64.powi(count as i32 - 1) / (2.0_f64.powi(count as i32) - 1.0)
        };
        let mut noise = Self {
            levels,
            amplitudes: amplitudes.to_vec(),
            first_octave,
            lowest_freq_input_factor,
            lowest_freq_value_factor,
            max_value: 0.0,
        };
        noise.max_value = noise.edge_value(2.0);
        noise
    }

    pub fn legacy(random: &mut Xoroshiro, first_octave: i32, octave_count: usize) -> Self {
        let amplitudes = vec![1.0; octave_count];
        let mut levels: Vec<Option<ImprovedNoise>> = (0..octave_count).map(|_| None).collect();
        let anchor = (-first_octave).max(0) as usize;
        let anchor_noise = ImprovedNoise::new(random);
        if anchor < octave_count {
            levels[anchor] = Some(anchor_noise);
        }
        for index in (0..anchor).rev() {
            if index < octave_count {
                levels[index] = Some(ImprovedNoise::new(random));
            } else {
                random.consume(262);
            }
        }
        let lowest_freq_input_factor = 2.0_f64.powi(first_octave);
        let lowest_freq_value_factor = if octave_count == 0 {
            0.0
        } else {
            2.0_f64.powi(octave_count as i32 - 1) / (2.0_f64.powi(octave_count as i32) - 1.0)
        };
        let mut noise = Self {
            levels,
            amplitudes,
            first_octave,
            lowest_freq_input_factor,
            lowest_freq_value_factor,
            max_value: 0.0,
        };
        noise.max_value = noise.edge_value(2.0);
        noise
    }

    pub fn octave_noise(&self, index: usize) -> Option<&ImprovedNoise> {
        let position = self.levels.len().checked_sub(1)?.checked_sub(index)?;
        self.levels[position].as_ref()
    }

    pub fn max_broken_value(&self, scale: f64) -> f64 {
        self.edge_value(scale + 2.0)
    }

    pub fn first_octave(&self) -> i32 {
        self.first_octave
    }

    pub fn max_value(&self) -> f64 {
        self.max_value
    }

    fn edge_value(&self, scale: f64) -> f64 {
        let mut total = 0.0;
        let mut value_factor = self.lowest_freq_value_factor;
        for (index, level) in self.levels.iter().enumerate() {
            if level.is_some() {
                total += self.amplitudes[index] * scale * value_factor;
            }
            value_factor /= 2.0;
        }
        total
    }

    pub fn value(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut total = 0.0;
        let mut input_factor = self.lowest_freq_input_factor;
        let mut value_factor = self.lowest_freq_value_factor;
        for (index, level) in self.levels.iter().enumerate() {
            if let Some(noise) = level {
                let sampled = noise.sample_with_y_limit(
                    wrap(x * input_factor),
                    wrap(y * input_factor),
                    wrap(z * input_factor),
                    0.0,
                    0.0,
                );
                total += self.amplitudes[index] * sampled * value_factor;
            }
            input_factor *= 2.0;
            value_factor /= 2.0;
        }
        total
    }
}

#[derive(Clone, Debug)]
pub struct NormalNoise {
    first: PerlinNoise,
    second: PerlinNoise,
    value_factor: f64,
    max_value: f64,
}

impl NormalNoise {
    pub fn new(random: &mut Xoroshiro, first_octave: i32, amplitudes: &[f64]) -> Self {
        let first = PerlinNoise::new(random, first_octave, amplitudes);
        let second = PerlinNoise::new(random, first_octave, amplitudes);
        let mut lowest = i32::MAX;
        let mut highest = i32::MIN;
        for (index, amplitude) in amplitudes.iter().enumerate() {
            if *amplitude != 0.0 {
                lowest = lowest.min(index as i32);
                highest = highest.max(index as i32);
            }
        }
        let span = if highest >= lowest {
            highest - lowest
        } else {
            0
        };
        let value_factor = DEVIATION_BASE / expected_deviation(span);
        let max_value = (first.max_value() + second.max_value()) * value_factor;
        Self {
            first,
            second,
            value_factor,
            max_value,
        }
    }

    pub fn max_value(&self) -> f64 {
        self.max_value
    }

    pub fn value(&self, x: f64, y: f64, z: f64) -> f64 {
        let shifted_x = x * SECOND_LAYER_SCALE;
        let shifted_y = y * SECOND_LAYER_SCALE;
        let shifted_z = z * SECOND_LAYER_SCALE;
        (self.first.value(x, y, z) + self.second.value(shifted_x, shifted_y, shifted_z))
            * self.value_factor
    }
}

fn expected_deviation(span: i32) -> f64 {
    0.1 * (1.0 + 1.0 / f64::from(span + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn improved(seed: i64) -> ImprovedNoise {
        ImprovedNoise::new(&mut Xoroshiro::from_seed(seed))
    }

    #[test]
    fn permutation_table_is_a_permutation() {
        let noise = improved(1);
        let mut sorted = noise.p;
        sorted.sort_unstable();
        for (index, value) in sorted.iter().enumerate() {
            assert_eq!(*value as usize, index);
        }
    }

    #[test]
    fn offsets_lie_in_0_256() {
        let noise = improved(99);
        for offset in [noise.xo, noise.yo, noise.zo] {
            assert!((0.0..256.0).contains(&offset));
        }
    }

    #[test]
    fn lattice_points_are_zero() {
        let noise = improved(5);
        assert_eq!(noise.sample(-noise.xo, -noise.yo, -noise.zo), 0.0);
    }

    #[test]
    fn improved_noise_is_deterministic_and_bounded() {
        let first = improved(1234);
        let second = improved(1234);
        for step in 0..20_000 {
            let t = f64::from(step) * 0.173;
            let x = t.sin() * 900.0 + t;
            let y = (t * 0.7).cos() * 90.0;
            let z = t * 1.3 - 400.0;
            let value = first.sample(x, y, z);
            assert_eq!(value, second.sample(x, y, z));
            assert!(value.abs() <= 1.2, "value {value} out of range");
        }
    }

    #[test]
    fn improved_noise_is_continuous() {
        let noise = improved(31337);
        let epsilon = 1.0e-6;
        for step in 0..5_000 {
            let t = f64::from(step) * 0.37;
            let here = noise.sample(t, t * 0.5, -t);
            let near = noise.sample(t + epsilon, t * 0.5, -t);
            assert!((here - near).abs() < 1.0e-4);
        }
    }

    #[test]
    fn wrap_is_periodic() {
        for value in [0.0, 1.5, -77.25, 3.0e7, -3.0e7, 1.0e9] {
            let wrapped = wrap(value);
            assert!(wrapped.abs() <= WRAP_PERIOD / 2.0 + 1.0e-6);
            let shifted = wrap(value + WRAP_PERIOD);
            assert!((wrapped - shifted).abs() < 1.0e-3 || value.abs() > 1.0e8);
        }
    }

    #[test]
    fn perlin_noise_respects_max_value() {
        let amplitudes = [1.0, 1.0, 0.0, 0.5, 0.25];
        let noise = PerlinNoise::new(&mut Xoroshiro::from_seed(2024), -7, &amplitudes);
        assert!(noise.max_value() > 0.0);
        assert_eq!(noise.first_octave(), -7);
        for step in 0..20_000 {
            let t = f64::from(step) * 3.1;
            let value = noise.value(t, (t * 0.01).sin() * 64.0, -t * 0.7);
            assert!(value.abs() <= noise.max_value());
        }
    }

    #[test]
    fn perlin_noise_skips_zero_amplitudes() {
        let noise = PerlinNoise::new(&mut Xoroshiro::from_seed(8), -3, &[1.0, 0.0, 1.0]);
        assert!(noise.levels[0].is_some());
        assert!(noise.levels[1].is_none());
        assert!(noise.levels[2].is_some());
    }

    #[test]
    fn perlin_factors_match_definition() {
        let noise = PerlinNoise::new(&mut Xoroshiro::from_seed(3), -2, &[1.0, 1.0, 1.0]);
        assert_eq!(noise.lowest_freq_input_factor, 0.25);
        assert!((noise.lowest_freq_value_factor - 4.0 / 7.0).abs() < 1.0e-15);
    }

    #[test]
    fn normal_noise_value_factor_for_single_octave() {
        let noise = NormalNoise::new(&mut Xoroshiro::from_seed(11), -4, &[1.0]);
        assert!((noise.value_factor - 5.0 / 6.0).abs() < 1.0e-12);
    }

    #[test]
    fn normal_noise_is_deterministic_and_bounded() {
        let amplitudes = [1.0, 1.0, 1.0, 0.0, 1.0, 1.0];
        let first = NormalNoise::new(&mut Xoroshiro::from_seed(777), -9, &amplitudes);
        let second = NormalNoise::new(&mut Xoroshiro::from_seed(777), -9, &amplitudes);
        let other = NormalNoise::new(&mut Xoroshiro::from_seed(778), -9, &amplitudes);
        let mut differs = false;
        for step in 0..10_000 {
            let t = f64::from(step) * 5.7;
            let value = first.value(t, 12.0, -t * 0.3);
            assert_eq!(value, second.value(t, 12.0, -t * 0.3));
            assert!(value.abs() <= first.max_value());
            differs |= value != other.value(t, 12.0, -t * 0.3);
        }
        assert!(differs);
    }

    #[test]
    fn two_layers_use_independent_streams() {
        let noise = NormalNoise::new(&mut Xoroshiro::from_seed(5), -5, &[1.0, 1.0]);
        let mut differs = false;
        for step in 0..200 {
            let t = f64::from(step) * 11.0;
            differs |= noise.first.value(t, 0.0, t) != noise.second.value(t, 0.0, t);
        }
        assert!(differs);
    }
}
