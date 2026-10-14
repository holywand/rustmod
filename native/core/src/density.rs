use std::sync::Arc;

use crate::blended::BlendedNoise;
use crate::graph::{Graph, MapKind, Node, NodeId};
use crate::noise::NormalNoise;
use crate::random::Xoroshiro;

const SHIFT_SCALE: f64 = 0.25;
const SHIFT_AMPLITUDE: f64 = 4.0;
const UNBOUNDED: (f64, f64) = (f64::NEG_INFINITY, f64::INFINITY);

pub trait InterpolatedSource {
    fn value(&self, node: NodeId, x: i32, y: i32, z: i32) -> f64;
}

#[derive(Clone, Copy)]
struct Slot {
    x: i32,
    y: i32,
    z: i32,
    value: f64,
    valid: bool,
}

pub struct Scratch {
    slots: Vec<Slot>,
    quantize_flat_cache: bool,
}

impl Scratch {
    pub fn new(instance: &Instance) -> Self {
        Self {
            slots: vec![
                Slot {
                    x: 0,
                    y: 0,
                    z: 0,
                    value: 0.0,
                    valid: false,
                };
                instance.graph.nodes.len()
            ],
            quantize_flat_cache: true,
        }
    }

    pub fn exact(instance: &Instance) -> Self {
        let mut scratch = Self::new(instance);
        scratch.quantize_flat_cache = false;
        scratch
    }

    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            slot.valid = false;
        }
    }
}

pub struct Instance {
    graph: Arc<Graph>,
    noises: Vec<NormalNoise>,
    blended: Vec<BlendedNoise>,
    bounds: Vec<(f64, f64)>,
}

#[inline]
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

#[inline]
fn lerp(delta: f64, start: f64, end: f64) -> f64 {
    start + delta * (end - start)
}

fn apply_map(kind: MapKind, value: f64) -> f64 {
    match kind {
        MapKind::Abs => value.abs(),
        MapKind::Square => value * value,
        MapKind::Cube => value * value * value,
        MapKind::HalfNegative => {
            if value > 0.0 {
                value
            } else {
                value * 0.5
            }
        }
        MapKind::QuarterNegative => {
            if value > 0.0 {
                value
            } else {
                value * 0.25
            }
        }
        MapKind::Squeeze => {
            let clamped = clamp(value, -1.0, 1.0);
            clamped / 2.0 - clamped * clamped * clamped / 24.0
        }
    }
}

fn map_bounds(kind: MapKind, (low, high): (f64, f64)) -> (f64, f64) {
    match kind {
        MapKind::Abs | MapKind::Square => {
            let a = apply_map(kind, low);
            let b = apply_map(kind, high);
            if low >= 0.0 {
                (a, b)
            } else if high <= 0.0 {
                (b, a)
            } else {
                (0.0, a.max(b))
            }
        }
        MapKind::Cube | MapKind::HalfNegative | MapKind::QuarterNegative | MapKind::Squeeze => {
            (apply_map(kind, low), apply_map(kind, high))
        }
    }
}

fn product_bounds(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    if !(a.0.is_finite() && a.1.is_finite() && b.0.is_finite() && b.1.is_finite()) {
        return UNBOUNDED;
    }
    let products = [a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1];
    let low = products.iter().copied().fold(f64::INFINITY, f64::min);
    let high = products.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (low, high)
}

impl Instance {
    pub fn new(graph: Arc<Graph>, seed: i64) -> Self {
        let factory = Xoroshiro::from_seed(seed).fork_positional();
        let noises: Vec<NormalNoise> = graph
            .noises
            .iter()
            .map(|definition| {
                let mut random = factory.from_hash_of(&definition.name);
                NormalNoise::new(&mut random, definition.first_octave, &definition.amplitudes)
            })
            .collect();
        let blended: Vec<BlendedNoise> = graph
            .blended
            .iter()
            .map(|definition| {
                let mut random = factory.from_hash_of("minecraft:terrain");
                BlendedNoise::new(
                    &mut random,
                    definition.xz_scale,
                    definition.y_scale,
                    definition.xz_factor,
                    definition.y_factor,
                    definition.smear_scale_multiplier,
                )
            })
            .collect();

        let mut instance = Self {
            graph,
            noises,
            blended,
            bounds: Vec::new(),
        };
        instance.bounds = instance.compute_bounds();
        instance
    }

    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn bounds(&self, node: NodeId) -> (f64, f64) {
        self.bounds[node as usize]
    }

    fn compute_bounds(&self) -> Vec<(f64, f64)> {
        let mut bounds: Vec<(f64, f64)> = Vec::with_capacity(self.graph.nodes.len());
        for node in &self.graph.nodes {
            let range = match node {
                Node::Constant(value) => (*value, *value),
                Node::Noise { noise, .. } | Node::ShiftedNoise { noise, .. } => {
                    let limit = self.noises[*noise].max_value();
                    (-limit, limit)
                }
                Node::ShiftA { noise } | Node::ShiftB { noise } | Node::Shift { noise } => {
                    let limit = self.noises[*noise].max_value() * SHIFT_AMPLITUDE;
                    (-limit, limit)
                }
                Node::Blended { index } => {
                    let limit = self.blended[*index].max_value();
                    (-limit, limit)
                }
                Node::Clamp { min, max, .. } => (*min, *max),
                Node::Mapped { kind, input } => map_bounds(*kind, bounds[*input as usize]),
                Node::Add(a, b) => {
                    let (a, b) = (bounds[*a as usize], bounds[*b as usize]);
                    (a.0 + b.0, a.1 + b.1)
                }
                Node::Mul(a, b) => product_bounds(bounds[*a as usize], bounds[*b as usize]),
                Node::Min(a, b) => {
                    let (a, b) = (bounds[*a as usize], bounds[*b as usize]);
                    (a.0.min(b.0), a.1.min(b.1))
                }
                Node::Max(a, b) => {
                    let (a, b) = (bounds[*a as usize], bounds[*b as usize]);
                    (a.0.max(b.0), a.1.max(b.1))
                }
                Node::RangeChoice {
                    in_range,
                    out_of_range,
                    ..
                } => {
                    let (a, b) = (bounds[*in_range as usize], bounds[*out_of_range as usize]);
                    (a.0.min(b.0), a.1.max(b.1))
                }
                Node::YClampedGradient {
                    from_value,
                    to_value,
                    ..
                } => (from_value.min(*to_value), from_value.max(*to_value)),
                Node::Spline(_) => UNBOUNDED,
                Node::WeirdScaledSampler { noise, rarity, .. } => {
                    (0.0, rarity.max_rarity() * self.noises[*noise].max_value())
                }
                Node::Interpolated(inner)
                | Node::FlatCache(inner)
                | Node::Cache2D(inner)
                | Node::CacheOnce(inner)
                | Node::CacheAllInCell(inner) => bounds[*inner as usize],
                Node::BlendAlpha => (1.0, 1.0),
                Node::BlendOffset => (0.0, 0.0),
                Node::BlendDensity(_) | Node::Beardifier => UNBOUNDED,
            };
            bounds.push(range);
        }
        bounds
    }

    pub fn eval(
        &self,
        scratch: &mut Scratch,
        source: Option<&dyn InterpolatedSource>,
        id: NodeId,
        x: i32,
        y: i32,
        z: i32,
    ) -> f64 {
        let slot = scratch.slots[id as usize];
        if slot.valid && slot.x == x && slot.y == y && slot.z == z {
            return slot.value;
        }
        let value = self.compute(scratch, source, id, x, y, z);
        scratch.slots[id as usize] = Slot {
            x,
            y,
            z,
            value,
            valid: true,
        };
        value
    }

    fn compute(
        &self,
        scratch: &mut Scratch,
        source: Option<&dyn InterpolatedSource>,
        id: NodeId,
        x: i32,
        y: i32,
        z: i32,
    ) -> f64 {
        let fx = f64::from(x);
        let fy = f64::from(y);
        let fz = f64::from(z);
        match &self.graph.nodes[id as usize] {
            Node::Constant(value) => *value,
            Node::Noise {
                noise,
                xz_scale,
                y_scale,
            } => self.noises[*noise].value(fx * xz_scale, fy * y_scale, fz * xz_scale),
            Node::ShiftedNoise {
                shift_x,
                shift_y,
                shift_z,
                xz_scale,
                y_scale,
                noise,
            } => {
                let d = fx * xz_scale + self.eval(scratch, source, *shift_x, x, y, z);
                let e = fy * y_scale + self.eval(scratch, source, *shift_y, x, y, z);
                let f = fz * xz_scale + self.eval(scratch, source, *shift_z, x, y, z);
                self.noises[*noise].value(d, e, f)
            }
            Node::ShiftA { noise } => {
                self.noises[*noise].value(fx * SHIFT_SCALE, 0.0, fz * SHIFT_SCALE) * SHIFT_AMPLITUDE
            }
            Node::ShiftB { noise } => {
                self.noises[*noise].value(fz * SHIFT_SCALE, fx * SHIFT_SCALE, 0.0) * SHIFT_AMPLITUDE
            }
            Node::Shift { noise } => {
                self.noises[*noise].value(fx * SHIFT_SCALE, fy * SHIFT_SCALE, fz * SHIFT_SCALE)
                    * SHIFT_AMPLITUDE
            }
            Node::Blended { index } => self.blended[*index].compute(x, y, z),
            Node::Clamp { input, min, max } => {
                clamp(self.eval(scratch, source, *input, x, y, z), *min, *max)
            }
            Node::Mapped { kind, input } => {
                apply_map(*kind, self.eval(scratch, source, *input, x, y, z))
            }
            Node::Add(a, b) => {
                self.eval(scratch, source, *a, x, y, z) + self.eval(scratch, source, *b, x, y, z)
            }
            Node::Mul(a, b) => {
                let first = self.eval(scratch, source, *a, x, y, z);
                if first == 0.0 {
                    0.0
                } else {
                    first * self.eval(scratch, source, *b, x, y, z)
                }
            }
            Node::Min(a, b) => {
                let first = self.eval(scratch, source, *a, x, y, z);
                if first < self.bounds[*b as usize].0 {
                    first
                } else {
                    first.min(self.eval(scratch, source, *b, x, y, z))
                }
            }
            Node::Max(a, b) => {
                let first = self.eval(scratch, source, *a, x, y, z);
                if first > self.bounds[*b as usize].1 {
                    first
                } else {
                    first.max(self.eval(scratch, source, *b, x, y, z))
                }
            }
            Node::RangeChoice {
                input,
                min_inclusive,
                max_exclusive,
                in_range,
                out_of_range,
            } => {
                let value = self.eval(scratch, source, *input, x, y, z);
                if value >= *min_inclusive && value < *max_exclusive {
                    self.eval(scratch, source, *in_range, x, y, z)
                } else {
                    self.eval(scratch, source, *out_of_range, x, y, z)
                }
            }
            Node::YClampedGradient {
                from_y,
                to_y,
                from_value,
                to_value,
            } => {
                let delta = (fy - from_y) / (to_y - from_y);
                if delta < 0.0 {
                    *from_value
                } else if delta > 1.0 {
                    *to_value
                } else {
                    lerp(delta, *from_value, *to_value)
                }
            }
            Node::Spline(index) => {
                let spline = &self.graph.splines[*index];
                let mut coordinate = |node: u32| self.eval(scratch, source, node, x, y, z) as f32;
                f64::from(spline.evaluate(&mut coordinate))
            }
            Node::WeirdScaledSampler {
                input,
                noise,
                rarity,
            } => {
                let scale = rarity.scale(self.eval(scratch, source, *input, x, y, z));
                scale
                    * self.noises[*noise]
                        .value(fx / scale, fy / scale, fz / scale)
                        .abs()
            }
            Node::Interpolated(inner) => match source {
                Some(provider) => provider.value(id, x, y, z),
                None => self.eval(scratch, source, *inner, x, y, z),
            },
            Node::FlatCache(inner) => {
                if scratch.quantize_flat_cache {
                    self.eval(scratch, source, *inner, (x >> 2) << 2, 0, (z >> 2) << 2)
                } else {
                    self.eval(scratch, source, *inner, x, y, z)
                }
            }
            Node::Cache2D(inner)
            | Node::CacheOnce(inner)
            | Node::CacheAllInCell(inner)
            | Node::BlendDensity(inner) => self.eval(scratch, source, *inner, x, y, z),
            Node::BlendAlpha => 1.0,
            Node::BlendOffset => 0.0,
            Node::Beardifier => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: i64 = 4_242_424_242;

    fn build(document: &str) -> (Instance, Scratch) {
        let graph = Graph::parse(document).expect("graph should load");
        let instance = Instance::new(Arc::new(graph), SEED);
        let scratch = Scratch::new(&instance);
        (instance, scratch)
    }

    fn value_of(document: &str, x: i32, y: i32, z: i32) -> f64 {
        let (instance, mut scratch) = build(document);
        let root = instance.graph().root("out").expect("root named out");
        instance.eval(&mut scratch, None, root, x, y, z)
    }

    fn single(function: &str) -> String {
        format!(r#"{{"router":{{"out":{function}}}}}"#)
    }

    #[test]
    fn arithmetic_nodes() {
        let add = single(r#"{"type":"minecraft:add","argument1":1.5,"argument2":2.25}"#);
        assert_eq!(value_of(&add, 0, 0, 0), 3.75);
        let mul = single(r#"{"type":"minecraft:mul","argument1":-2.0,"argument2":4.5}"#);
        assert_eq!(value_of(&mul, 0, 0, 0), -9.0);
        let min = single(r#"{"type":"minecraft:min","argument1":3.0,"argument2":-1.0}"#);
        assert_eq!(value_of(&min, 0, 0, 0), -1.0);
        let max = single(r#"{"type":"minecraft:max","argument1":3.0,"argument2":-1.0}"#);
        assert_eq!(value_of(&max, 0, 0, 0), 3.0);
    }

    #[test]
    fn mapped_nodes() {
        let cases: [(&str, f64, f64); 6] = [
            ("abs", -3.0, 3.0),
            ("square", -3.0, 9.0),
            ("cube", -2.0, -8.0),
            ("half_negative", -4.0, -2.0),
            ("quarter_negative", -4.0, -1.0),
            ("squeeze", 2.0, 0.5 - 1.0 / 24.0),
        ];
        for (name, input, expected) in cases {
            let document = single(&format!(
                r#"{{"type":"minecraft:{name}","argument":{input}}}"#
            ));
            assert_eq!(value_of(&document, 0, 0, 0), expected, "{name}");
        }
    }

    #[test]
    fn clamp_and_range_choice() {
        let clamp = single(r#"{"type":"minecraft:clamp","input":9.0,"min":-1.0,"max":2.0}"#);
        assert_eq!(value_of(&clamp, 0, 0, 0), 2.0);
        let choice = |input: f64| {
            single(&format!(
                r#"{{"type":"minecraft:range_choice","input":{input},"min_inclusive":0.0,"max_exclusive":1.0,"when_in_range":10.0,"when_out_of_range":20.0}}"#
            ))
        };
        assert_eq!(value_of(&choice(0.0), 0, 0, 0), 10.0);
        assert_eq!(value_of(&choice(0.999), 0, 0, 0), 10.0);
        assert_eq!(value_of(&choice(1.0), 0, 0, 0), 20.0);
        assert_eq!(value_of(&choice(-0.001), 0, 0, 0), 20.0);
    }

    #[test]
    fn y_clamped_gradient() {
        let document = single(
            r#"{"type":"minecraft:y_clamped_gradient","from_y":-64,"to_y":320,"from_value":1.5,"to_value":-1.5}"#,
        );
        assert_eq!(value_of(&document, 0, -64, 0), 1.5);
        assert_eq!(value_of(&document, 0, 320, 0), -1.5);
        assert_eq!(value_of(&document, 0, 128, 0), 0.0);
        assert_eq!(value_of(&document, 0, -500, 0), 1.5);
        assert_eq!(value_of(&document, 0, 900, 0), -1.5);
    }

    #[test]
    fn spline_node_uses_coordinate_function() {
        let document = single(
            r#"{"type":"minecraft:spline","spline":{"coordinate":{"type":"minecraft:y_clamped_gradient","from_y":-1,"to_y":1,"from_value":-1.0,"to_value":1.0},"points":[{"location":-1.0,"value":0.0,"derivative":0.0},{"location":1.0,"value":1.0,"derivative":0.0}]}}"#,
        );
        assert_eq!(value_of(&document, 0, 0, 0), 0.5);
        assert_eq!(value_of(&document, 0, -1, 0), 0.0);
        assert_eq!(value_of(&document, 0, 1, 0), 1.0);
    }

    #[test]
    fn references_are_shared_and_resolved() {
        let document = r#"{
            "functions": {"test:base": {"type":"minecraft:add","argument1":1.0,"argument2":2.0}},
            "router": {"out": {"type":"minecraft:mul","argument1":"test:base","argument2":"test:base"}}
        }"#;
        let graph = Graph::parse(document).unwrap();
        let base_nodes = graph
            .nodes
            .iter()
            .filter(|node| matches!(node, Node::Add(_, _)))
            .count();
        assert_eq!(base_nodes, 1);
        assert_eq!(value_of(document, 0, 0, 0), 9.0);
    }

    #[test]
    fn loader_rejects_bad_documents() {
        let cycle = r#"{"functions":{"a":"b","b":"a"},"router":{"out":"a"}}"#;
        assert!(matches!(
            Graph::parse(cycle),
            Err(crate::graph::LoadError::Cycle(_))
        ));
        let unknown = single(r#"{"type":"minecraft:end_islands"}"#);
        assert!(matches!(
            Graph::parse(&unknown),
            Err(crate::graph::LoadError::Unsupported(_))
        ));
        let foreign = single(r#"{"type":"othermod:wobble"}"#);
        assert!(matches!(
            Graph::parse(&foreign),
            Err(crate::graph::LoadError::Unsupported(_))
        ));
        let missing = single(r#""nobody:home""#);
        assert!(matches!(
            Graph::parse(&missing),
            Err(crate::graph::LoadError::Missing(_))
        ));
        let unordered = single(
            r#"{"type":"minecraft:spline","spline":{"coordinate":0.0,"points":[{"location":1.0,"value":0.0,"derivative":0.0},{"location":0.0,"value":1.0,"derivative":0.0}]}}"#,
        );
        assert!(Graph::parse(&unordered).is_err());
        assert!(Graph::parse("{").is_err());
        assert!(Graph::parse("{}").is_err());
    }

    fn noise_document(function: &str) -> String {
        format!(
            r#"{{"noises":{{"minecraft:test":{{"firstOctave":-3,"amplitudes":[1.0,1.0,0.5]}}}},"router":{{"out":{function}}}}}"#
        )
    }

    fn reference_noise() -> NormalNoise {
        let mut random = Xoroshiro::from_seed(SEED)
            .fork_positional()
            .from_hash_of("minecraft:test");
        NormalNoise::new(&mut random, -3, &[1.0, 1.0, 0.5])
    }

    #[test]
    fn noise_node_matches_direct_normal_noise() {
        let document = noise_document(
            r#"{"type":"minecraft:noise","noise":"minecraft:test","xz_scale":0.5,"y_scale":2.0}"#,
        );
        let expected = reference_noise().value(10.0 * 0.5, 20.0 * 2.0, 30.0 * 0.5);
        assert_eq!(value_of(&document, 10, 20, 30), expected);
    }

    #[test]
    fn shift_nodes_match_definition() {
        let reference = reference_noise();
        let shift_a = noise_document(r#"{"type":"minecraft:shift_a","argument":"minecraft:test"}"#);
        assert_eq!(
            value_of(&shift_a, 12, 99, -40),
            reference.value(12.0 * 0.25, 0.0, -40.0 * 0.25) * 4.0
        );
        let shift_b = noise_document(r#"{"type":"minecraft:shift_b","argument":"minecraft:test"}"#);
        assert_eq!(
            value_of(&shift_b, 12, 99, -40),
            reference.value(-40.0 * 0.25, 12.0 * 0.25, 0.0) * 4.0
        );
    }

    #[test]
    fn shifted_noise_adds_shift_inputs() {
        let document = noise_document(
            r#"{"type":"minecraft:shifted_noise","shift_x":1.5,"shift_y":-2.0,"shift_z":0.25,"xz_scale":0.1,"y_scale":0.2,"noise":"minecraft:test"}"#,
        );
        let expected = reference_noise().value(7.0 * 0.1 + 1.5, 9.0 * 0.2 - 2.0, 11.0 * 0.1 + 0.25);
        assert_eq!(value_of(&document, 7, 9, 11), expected);
    }

    #[test]
    fn weird_scaled_sampler_uses_rarity_mapping() {
        let document = noise_document(
            r#"{"type":"minecraft:weird_scaled_sampler","input":0.7,"noise":"minecraft:test","rarity_value_mapper":"type_2"}"#,
        );
        let expected = 2.0
            * reference_noise()
                .value(8.0 / 2.0, 16.0 / 2.0, 24.0 / 2.0)
                .abs();
        assert_eq!(value_of(&document, 8, 16, 24), expected);
    }

    #[test]
    fn flat_cache_quantizes_to_quart_columns() {
        let inner =
            r#"{"type":"minecraft:noise","noise":"minecraft:test","xz_scale":1.0,"y_scale":1.0}"#;
        let cached = noise_document(&format!(
            r#"{{"type":"minecraft:flat_cache","argument":{inner}}}"#
        ));
        let plain = noise_document(inner);
        assert_eq!(value_of(&cached, 7, 55, 10), value_of(&plain, 4, 0, 8));
        assert_eq!(value_of(&cached, -1, 12, -5), value_of(&plain, -4, 0, -8));
    }

    #[test]
    fn exact_scratch_skips_flat_cache_quantization() {
        let inner =
            r#"{"type":"minecraft:noise","noise":"minecraft:test","xz_scale":1.0,"y_scale":1.0}"#;
        let cached = noise_document(&format!(
            r#"{{"type":"minecraft:flat_cache","argument":{inner}}}"#
        ));
        let (instance, _) = build(&cached);
        let root = instance.graph().root("out").unwrap();
        let mut exact = Scratch::exact(&instance);
        let plain = noise_document(inner);
        assert_eq!(
            instance.eval(&mut exact, None, root, 7, 55, 10),
            value_of(&plain, 7, 55, 10)
        );
    }

    #[test]
    fn marker_nodes_are_transparent() {
        let inner =
            r#"{"type":"minecraft:noise","noise":"minecraft:test","xz_scale":1.0,"y_scale":1.0}"#;
        let plain = noise_document(inner);
        let expected = value_of(&plain, 3, 4, 5);
        for marker in [
            "interpolated",
            "cache_2d",
            "cache_once",
            "cache_all_in_cell",
            "blend_density",
        ] {
            let document = noise_document(&format!(
                r#"{{"type":"minecraft:{marker}","argument":{inner}}}"#
            ));
            assert_eq!(value_of(&document, 3, 4, 5), expected, "{marker}");
        }
    }

    #[test]
    fn interpolated_source_overrides_inner_value() {
        struct Fixed;
        impl InterpolatedSource for Fixed {
            fn value(&self, _node: NodeId, _x: i32, _y: i32, _z: i32) -> f64 {
                42.0
            }
        }
        let document = single(
            r#"{"type":"minecraft:add","argument1":{"type":"minecraft:interpolated","argument":1.0},"argument2":0.5}"#,
        );
        let (instance, mut scratch) = build(&document);
        let root = instance.graph().root("out").unwrap();
        assert_eq!(
            instance.eval(&mut scratch, Some(&Fixed), root, 0, 0, 0),
            42.5
        );
        scratch.clear();
        assert_eq!(instance.eval(&mut scratch, None, root, 0, 0, 0), 1.5);
    }

    #[test]
    fn memoization_does_not_leak_between_points() {
        let document = noise_document(
            r#"{"type":"minecraft:noise","noise":"minecraft:test","xz_scale":1.0,"y_scale":1.0}"#,
        );
        let (instance, mut scratch) = build(&document);
        let root = instance.graph().root("out").unwrap();
        let first = instance.eval(&mut scratch, None, root, 1, 2, 3);
        let second = instance.eval(&mut scratch, None, root, 4, 5, 6);
        assert_ne!(first, second);
        assert_eq!(instance.eval(&mut scratch, None, root, 1, 2, 3), first);
        assert_eq!(instance.eval(&mut scratch, None, root, 4, 5, 6), second);
    }

    #[test]
    fn bounds_are_valid_for_noise_graphs() {
        let document = noise_document(
            r#"{"type":"minecraft:add","argument1":{"type":"minecraft:noise","noise":"minecraft:test","xz_scale":1.0,"y_scale":1.0},"argument2":{"type":"minecraft:y_clamped_gradient","from_y":0,"to_y":100,"from_value":1.0,"to_value":-1.0}}"#,
        );
        let (instance, mut scratch) = build(&document);
        let root = instance.graph().root("out").unwrap();
        let (low, high) = instance.bounds(root);
        for step in 0..3000 {
            let value = instance.eval(
                &mut scratch,
                None,
                root,
                step * 13 - 2000,
                step % 120 - 10,
                step * 7 - 900,
            );
            assert!(value >= low && value <= high);
        }
    }

    #[test]
    fn min_shortcut_does_not_change_value() {
        let heavy =
            r#"{"type":"minecraft:noise","noise":"minecraft:test","xz_scale":1.0,"y_scale":1.0}"#;
        let document = noise_document(&format!(
            r#"{{"type":"minecraft:min","argument1":-1000.0,"argument2":{heavy}}}"#
        ));
        assert_eq!(value_of(&document, 5, 5, 5), -1000.0);
        let zero_mul = noise_document(&format!(
            r#"{{"type":"minecraft:mul","argument1":0.0,"argument2":{heavy}}}"#
        ));
        assert_eq!(value_of(&zero_mul, 5, 5, 5), 0.0);
    }

    #[test]
    fn blended_noise_node_is_seeded_from_terrain_hash() {
        let document = single(
            r#"{"type":"minecraft:old_blended_noise","xz_scale":0.25,"y_scale":0.125,"xz_factor":80.0,"y_factor":160.0,"smear_scale_multiplier":8.0}"#,
        );
        let mut random = Xoroshiro::from_seed(SEED)
            .fork_positional()
            .from_hash_of("minecraft:terrain");
        let reference = BlendedNoise::new(&mut random, 0.25, 0.125, 80.0, 160.0, 8.0);
        assert_eq!(
            value_of(&document, 100, 70, -250),
            reference.compute(100, 70, -250)
        );
    }
}
