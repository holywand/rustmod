use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::json::{self, Json};
use crate::spline::Spline;

pub type NodeId = u32;

const MAX_DEPTH: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    Abs,
    Square,
    Cube,
    HalfNegative,
    QuarterNegative,
    Squeeze,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rarity {
    Type1,
    Type2,
}

impl Rarity {
    pub fn scale(self, value: f64) -> f64 {
        match self {
            Rarity::Type1 => {
                if value < -0.5 {
                    0.75
                } else if value < 0.0 {
                    1.0
                } else if value < 0.5 {
                    1.5
                } else {
                    2.0
                }
            }
            Rarity::Type2 => {
                if value < -0.75 {
                    0.5
                } else if value < -0.5 {
                    0.75
                } else if value < 0.5 {
                    1.0
                } else if value < 0.75 {
                    2.0
                } else {
                    3.0
                }
            }
        }
    }

    pub fn max_rarity(self) -> f64 {
        match self {
            Rarity::Type1 => 2.0,
            Rarity::Type2 => 3.0,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Node {
    Constant(f64),
    Noise {
        noise: usize,
        xz_scale: f64,
        y_scale: f64,
    },
    ShiftedNoise {
        shift_x: NodeId,
        shift_y: NodeId,
        shift_z: NodeId,
        xz_scale: f64,
        y_scale: f64,
        noise: usize,
    },
    ShiftA {
        noise: usize,
    },
    ShiftB {
        noise: usize,
    },
    Shift {
        noise: usize,
    },
    Blended {
        index: usize,
    },
    Clamp {
        input: NodeId,
        min: f64,
        max: f64,
    },
    Mapped {
        kind: MapKind,
        input: NodeId,
    },
    Add(NodeId, NodeId),
    Mul(NodeId, NodeId),
    Min(NodeId, NodeId),
    Max(NodeId, NodeId),
    RangeChoice {
        input: NodeId,
        min_inclusive: f64,
        max_exclusive: f64,
        in_range: NodeId,
        out_of_range: NodeId,
    },
    YClampedGradient {
        from_y: f64,
        to_y: f64,
        from_value: f64,
        to_value: f64,
    },
    Spline(usize),
    WeirdScaledSampler {
        input: NodeId,
        noise: usize,
        rarity: Rarity,
    },
    Interpolated(NodeId),
    FlatCache(NodeId),
    Cache2D(NodeId),
    CacheOnce(NodeId),
    CacheAllInCell(NodeId),
    BlendAlpha,
    BlendOffset,
    BlendDensity(NodeId),
    Beardifier,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NoiseDef {
    pub name: String,
    pub first_octave: i32,
    pub amplitudes: Vec<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendedDef {
    pub xz_scale: f64,
    pub y_scale: f64,
    pub xz_factor: f64,
    pub y_factor: f64,
    pub smear_scale_multiplier: f64,
}

#[derive(Debug)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub splines: Vec<Spline>,
    pub noises: Vec<NoiseDef>,
    pub blended: Vec<BlendedDef>,
    pub roots: Vec<(String, NodeId)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadError {
    Json(String),
    Missing(String),
    Invalid(String),
    Unsupported(String),
    Cycle(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Json(message) => write!(formatter, "invalid JSON: {message}"),
            LoadError::Missing(message) => write!(formatter, "missing: {message}"),
            LoadError::Invalid(message) => write!(formatter, "invalid: {message}"),
            LoadError::Unsupported(message) => write!(formatter, "unsupported: {message}"),
            LoadError::Cycle(message) => write!(formatter, "cycle through {message}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl Graph {
    pub fn parse(text: &str) -> Result<Graph, LoadError> {
        let document = json::parse(text).map_err(|error| LoadError::Json(error.to_string()))?;
        Graph::from_json(&document)
    }

    pub fn from_json(document: &Json) -> Result<Graph, LoadError> {
        let mut loader = Loader {
            functions: HashMap::new(),
            noise_lookup: HashMap::new(),
            memo: HashMap::new(),
            active: HashSet::new(),
            graph: Graph {
                nodes: Vec::new(),
                splines: Vec::new(),
                noises: Vec::new(),
                blended: Vec::new(),
                roots: Vec::new(),
            },
        };

        if let Some(noises) = document.get("noises") {
            let entries = noises
                .as_object()
                .ok_or_else(|| LoadError::Invalid("\"noises\" must be an object".into()))?;
            for (name, definition) in entries {
                let first_octave = definition
                    .get("firstOctave")
                    .and_then(Json::as_f64)
                    .filter(|value| value.fract() == 0.0 && value.abs() < 1024.0)
                    .ok_or_else(|| LoadError::Invalid(format!("noise {name}: firstOctave")))?
                    as i32;
                let amplitudes = definition
                    .get("amplitudes")
                    .and_then(Json::as_array)
                    .ok_or_else(|| LoadError::Invalid(format!("noise {name}: amplitudes")))?
                    .iter()
                    .map(|value| {
                        value
                            .as_f64()
                            .ok_or_else(|| LoadError::Invalid(format!("noise {name}: amplitude")))
                    })
                    .collect::<Result<Vec<f64>, LoadError>>()?;
                if amplitudes.is_empty() {
                    return Err(LoadError::Invalid(format!("noise {name}: no amplitudes")));
                }
                loader
                    .noise_lookup
                    .insert(name.clone(), loader.graph.noises.len());
                loader.graph.noises.push(NoiseDef {
                    name: name.clone(),
                    first_octave,
                    amplitudes,
                });
            }
        }

        if let Some(functions) = document.get("functions") {
            let entries = functions
                .as_object()
                .ok_or_else(|| LoadError::Invalid("\"functions\" must be an object".into()))?;
            for (name, definition) in entries {
                loader.functions.insert(name.clone(), definition);
            }
        }

        let router = document
            .get("router")
            .and_then(Json::as_object)
            .ok_or_else(|| LoadError::Missing("\"router\" object".into()))?;
        for (name, definition) in router {
            let id = loader.node(definition, 0)?;
            loader.graph.roots.push((name.clone(), id));
        }
        Ok(loader.graph)
    }

    pub fn root(&self, name: &str) -> Option<NodeId> {
        self.roots
            .iter()
            .find(|(root_name, _)| root_name == name)
            .map(|(_, id)| *id)
    }

    pub fn interpolated_nodes(&self) -> Vec<NodeId> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node, Node::Interpolated(_)))
            .map(|(index, _)| index as NodeId)
            .collect()
    }

    pub fn reachable_from(&self, roots: &[NodeId]) -> Vec<bool> {
        let mut reached = vec![false; self.nodes.len()];
        let mut stack: Vec<NodeId> = roots.to_vec();
        while let Some(id) = stack.pop() {
            if std::mem::replace(&mut reached[id as usize], true) {
                continue;
            }
            self.for_each_child(id, &mut |child| stack.push(child));
        }
        reached
    }

    pub fn for_each_child(&self, id: NodeId, visit: &mut dyn FnMut(NodeId)) {
        match &self.nodes[id as usize] {
            Node::ShiftedNoise {
                shift_x,
                shift_y,
                shift_z,
                ..
            } => {
                visit(*shift_x);
                visit(*shift_y);
                visit(*shift_z);
            }
            Node::Clamp { input, .. }
            | Node::Mapped { input, .. }
            | Node::WeirdScaledSampler { input, .. } => visit(*input),
            Node::Add(a, b) | Node::Mul(a, b) | Node::Min(a, b) | Node::Max(a, b) => {
                visit(*a);
                visit(*b);
            }
            Node::RangeChoice {
                input,
                in_range,
                out_of_range,
                ..
            } => {
                visit(*input);
                visit(*in_range);
                visit(*out_of_range);
            }
            Node::Spline(index) => {
                self.splines[*index].for_each_coordinate(&mut |child| visit(child))
            }
            Node::Interpolated(inner)
            | Node::FlatCache(inner)
            | Node::Cache2D(inner)
            | Node::CacheOnce(inner)
            | Node::CacheAllInCell(inner)
            | Node::BlendDensity(inner) => visit(*inner),
            Node::Constant(_)
            | Node::Noise { .. }
            | Node::ShiftA { .. }
            | Node::ShiftB { .. }
            | Node::Shift { .. }
            | Node::Blended { .. }
            | Node::YClampedGradient { .. }
            | Node::BlendAlpha
            | Node::BlendOffset
            | Node::Beardifier => {}
        }
    }
}

struct Loader<'a> {
    functions: HashMap<String, &'a Json>,
    noise_lookup: HashMap<String, usize>,
    memo: HashMap<String, NodeId>,
    active: HashSet<String>,
    graph: Graph,
}

fn number(object: &Json, key: &str, context: &str) -> Result<f64, LoadError> {
    object
        .get(key)
        .and_then(Json::as_f64)
        .ok_or_else(|| LoadError::Invalid(format!("{context}: numeric field \"{key}\"")))
}

fn number_or(object: &Json, key: &str, default: f64, context: &str) -> Result<f64, LoadError> {
    match object.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_f64()
            .ok_or_else(|| LoadError::Invalid(format!("{context}: numeric field \"{key}\""))),
    }
}

impl<'a> Loader<'a> {
    fn push(&mut self, node: Node) -> NodeId {
        self.graph.nodes.push(node);
        (self.graph.nodes.len() - 1) as NodeId
    }

    fn child(
        &mut self,
        object: &Json,
        key: &str,
        context: &str,
        depth: usize,
    ) -> Result<NodeId, LoadError> {
        let value = object
            .get(key)
            .ok_or_else(|| LoadError::Missing(format!("{context}: field \"{key}\"")))?;
        self.node(value, depth + 1)
    }

    fn noise_ref(&self, object: &Json, key: &str, context: &str) -> Result<usize, LoadError> {
        let value = object
            .get(key)
            .ok_or_else(|| LoadError::Missing(format!("{context}: field \"{key}\"")))?;
        match value {
            Json::String(name) => self
                .noise_lookup
                .get(name)
                .copied()
                .ok_or_else(|| LoadError::Missing(format!("noise {name}"))),
            _ => Err(LoadError::Unsupported(format!(
                "{context}: inline noise definitions"
            ))),
        }
    }

    fn node(&mut self, value: &Json, depth: usize) -> Result<NodeId, LoadError> {
        if depth > MAX_DEPTH {
            return Err(LoadError::Invalid(
                "density function nesting too deep".into(),
            ));
        }
        match value {
            Json::Number(constant) => Ok(self.push(Node::Constant(*constant))),
            Json::String(name) => self.reference(name, depth),
            Json::Object(_) => self.typed(value, depth),
            _ => Err(LoadError::Invalid(
                "density function must be a number, a reference or an object".into(),
            )),
        }
    }

    fn reference(&mut self, name: &str, depth: usize) -> Result<NodeId, LoadError> {
        if let Some(id) = self.memo.get(name) {
            return Ok(*id);
        }
        if !self.active.insert(name.to_string()) {
            return Err(LoadError::Cycle(name.to_string()));
        }
        let definition = *self
            .functions
            .get(name)
            .ok_or_else(|| LoadError::Missing(format!("density function {name}")))?;
        let result = self.node(definition, depth + 1);
        self.active.remove(name);
        let id = result?;
        self.memo.insert(name.to_string(), id);
        Ok(id)
    }

    fn typed(&mut self, object: &Json, depth: usize) -> Result<NodeId, LoadError> {
        let full = object
            .get("type")
            .and_then(Json::as_str)
            .ok_or_else(|| LoadError::Missing("density function \"type\"".into()))?;
        let name = match full.strip_prefix("minecraft:") {
            Some(stripped) => stripped,
            None if full.contains(':') => {
                return Err(LoadError::Unsupported(format!(
                    "density function type {full}"
                )))
            }
            None => full,
        };
        let context = name;

        let node = match name {
            "constant" => Node::Constant(number(object, "argument", context)?),
            "abs" | "square" | "cube" | "half_negative" | "quarter_negative" | "squeeze" => {
                let kind = match name {
                    "abs" => MapKind::Abs,
                    "square" => MapKind::Square,
                    "cube" => MapKind::Cube,
                    "half_negative" => MapKind::HalfNegative,
                    "quarter_negative" => MapKind::QuarterNegative,
                    _ => MapKind::Squeeze,
                };
                let input = self.child(object, "argument", context, depth)?;
                Node::Mapped { kind, input }
            }
            "add" | "mul" | "min" | "max" => {
                let first = self.child(object, "argument1", context, depth)?;
                let second = self.child(object, "argument2", context, depth)?;
                match name {
                    "add" => Node::Add(first, second),
                    "mul" => Node::Mul(first, second),
                    "min" => Node::Min(first, second),
                    _ => Node::Max(first, second),
                }
            }
            "clamp" => {
                let input = self.child(object, "input", context, depth)?;
                let min = number(object, "min", context)?;
                let max = number(object, "max", context)?;
                if min > max {
                    return Err(LoadError::Invalid("clamp: min is greater than max".into()));
                }
                Node::Clamp { input, min, max }
            }
            "range_choice" => {
                let input = self.child(object, "input", context, depth)?;
                let min_inclusive = number(object, "min_inclusive", context)?;
                let max_exclusive = number(object, "max_exclusive", context)?;
                let in_range = self.child(object, "when_in_range", context, depth)?;
                let out_of_range = self.child(object, "when_out_of_range", context, depth)?;
                Node::RangeChoice {
                    input,
                    min_inclusive,
                    max_exclusive,
                    in_range,
                    out_of_range,
                }
            }
            "y_clamped_gradient" => Node::YClampedGradient {
                from_y: number(object, "from_y", context)?,
                to_y: number(object, "to_y", context)?,
                from_value: number(object, "from_value", context)?,
                to_value: number(object, "to_value", context)?,
            },
            "noise" => Node::Noise {
                noise: self.noise_ref(object, "noise", context)?,
                xz_scale: number(object, "xz_scale", context)?,
                y_scale: number(object, "y_scale", context)?,
            },
            "shifted_noise" => {
                let shift_x = self.child(object, "shift_x", context, depth)?;
                let shift_y = self.child(object, "shift_y", context, depth)?;
                let shift_z = self.child(object, "shift_z", context, depth)?;
                Node::ShiftedNoise {
                    shift_x,
                    shift_y,
                    shift_z,
                    xz_scale: number(object, "xz_scale", context)?,
                    y_scale: number(object, "y_scale", context)?,
                    noise: self.noise_ref(object, "noise", context)?,
                }
            }
            "shift_a" => Node::ShiftA {
                noise: self.noise_ref(object, "argument", context)?,
            },
            "shift_b" => Node::ShiftB {
                noise: self.noise_ref(object, "argument", context)?,
            },
            "shift" => Node::Shift {
                noise: self.noise_ref(object, "argument", context)?,
            },
            "old_blended_noise" => {
                let definition = BlendedDef {
                    xz_scale: number_or(object, "xz_scale", 1.0, context)?,
                    y_scale: number_or(object, "y_scale", 1.0, context)?,
                    xz_factor: number_or(object, "xz_factor", 80.0, context)?,
                    y_factor: number_or(object, "y_factor", 160.0, context)?,
                    smear_scale_multiplier: number_or(
                        object,
                        "smear_scale_multiplier",
                        8.0,
                        context,
                    )?,
                };
                if definition.xz_factor == 0.0 || definition.y_factor == 0.0 {
                    return Err(LoadError::Invalid("old_blended_noise: zero factor".into()));
                }
                self.graph.blended.push(definition);
                Node::Blended {
                    index: self.graph.blended.len() - 1,
                }
            }
            "weird_scaled_sampler" => {
                let input = self.child(object, "input", context, depth)?;
                let noise = self.noise_ref(object, "noise", context)?;
                let rarity = match object.get("rarity_value_mapper").and_then(Json::as_str) {
                    Some("type_1") => Rarity::Type1,
                    Some("type_2") => Rarity::Type2,
                    _ => {
                        return Err(LoadError::Invalid(
                            "weird_scaled_sampler: rarity_value_mapper".into(),
                        ))
                    }
                };
                Node::WeirdScaledSampler {
                    input,
                    noise,
                    rarity,
                }
            }
            "interpolated" | "flat_cache" | "cache_2d" | "cache_once" | "cache_all_in_cell"
            | "blend_density" => {
                let inner = self.child(object, "argument", context, depth)?;
                match name {
                    "interpolated" => Node::Interpolated(inner),
                    "flat_cache" => Node::FlatCache(inner),
                    "cache_2d" => Node::Cache2D(inner),
                    "cache_once" => Node::CacheOnce(inner),
                    "cache_all_in_cell" => Node::CacheAllInCell(inner),
                    _ => Node::BlendDensity(inner),
                }
            }
            "blend_alpha" => Node::BlendAlpha,
            "blend_offset" => Node::BlendOffset,
            "beardifier" => Node::Beardifier,
            "spline" => {
                let definition = object
                    .get("spline")
                    .ok_or_else(|| LoadError::Missing("spline: field \"spline\"".into()))?;
                let spline = self.spline(definition, depth + 1)?;
                self.graph.splines.push(spline);
                Node::Spline(self.graph.splines.len() - 1)
            }
            other => {
                return Err(LoadError::Unsupported(format!(
                    "density function type {other}"
                )))
            }
        };
        Ok(self.push(node))
    }

    fn spline(&mut self, value: &Json, depth: usize) -> Result<Spline, LoadError> {
        if depth > MAX_DEPTH {
            return Err(LoadError::Invalid("spline nesting too deep".into()));
        }
        match value {
            Json::Number(constant) => Ok(Spline::Constant(*constant as f32)),
            Json::Object(_) => {
                let coordinate_json = value
                    .get("coordinate")
                    .ok_or_else(|| LoadError::Missing("spline: field \"coordinate\"".into()))?;
                let coordinate = self.node(coordinate_json, depth + 1)?;
                let points = value
                    .get("points")
                    .and_then(Json::as_array)
                    .filter(|points| !points.is_empty())
                    .ok_or_else(|| {
                        LoadError::Invalid("spline: \"points\" must be a non-empty array".into())
                    })?;
                let mut locations: Vec<f32> = Vec::with_capacity(points.len());
                let mut values = Vec::with_capacity(points.len());
                let mut derivatives = Vec::with_capacity(points.len());
                for point in points {
                    let location = number(point, "location", "spline point")? as f32;
                    if let Some(previous) = locations.last() {
                        if location <= *previous {
                            return Err(LoadError::Invalid(
                                "spline points are not ascending".into(),
                            ));
                        }
                    }
                    let derivative = number(point, "derivative", "spline point")? as f32;
                    let child = point.get("value").ok_or_else(|| {
                        LoadError::Missing("spline point: field \"value\"".into())
                    })?;
                    locations.push(location);
                    derivatives.push(derivative);
                    values.push(self.spline(child, depth + 1)?);
                }
                Ok(Spline::Multipoint {
                    coordinate,
                    locations,
                    values,
                    derivatives,
                })
            }
            _ => Err(LoadError::Invalid(
                "spline must be a number or an object".into(),
            )),
        }
    }
}
