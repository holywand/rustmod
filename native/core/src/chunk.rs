use std::sync::Arc;

use crate::density::{Instance, InterpolatedSource, Scratch};
use crate::graph::{Node, NodeId};

pub const CHUNK_SIZE: i32 = 16;
const NO_SLOT: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub min_y: i32,
    pub height: i32,
    pub cell_width: i32,
    pub cell_height: i32,
}

impl Geometry {
    pub fn validate(&self) -> Result<(), String> {
        if self.cell_width <= 0 || CHUNK_SIZE % self.cell_width != 0 {
            return Err(format!(
                "cell width {} does not divide the chunk",
                self.cell_width
            ));
        }
        if self.cell_height <= 0 || self.height <= 0 || self.height % self.cell_height != 0 {
            return Err(format!(
                "height {} is not a multiple of cell height {}",
                self.height, self.cell_height
            ));
        }
        if self.min_y % self.cell_height != 0 {
            return Err(format!(
                "min y {} is not aligned to cell height {}",
                self.min_y, self.cell_height
            ));
        }
        Ok(())
    }

    pub fn cells_xz(&self) -> usize {
        (CHUNK_SIZE / self.cell_width) as usize
    }

    pub fn cells_y(&self) -> usize {
        (self.height / self.cell_height) as usize
    }
}

#[inline]
fn lerp(delta: f64, start: f64, end: f64) -> f64 {
    start + delta * (end - start)
}

pub struct ChunkPlan {
    instance: Arc<Instance>,
    root: NodeId,
    geometry: Geometry,
    interpolated: Vec<NodeId>,
    slot_of: Vec<u32>,
    beardifier_slots: Vec<bool>,
}

pub struct CornerGrid {
    geometry: Geometry,
    origin_x: i32,
    origin_z: i32,
    slot_of: Vec<u32>,
    corners_xz: usize,
    corners_y: usize,
    values: Vec<f64>,
}

impl ChunkPlan {
    pub fn new(instance: Arc<Instance>, root: NodeId, geometry: Geometry) -> Result<Self, String> {
        Self::with_roots(instance, root, &[root], geometry)
    }

    pub fn with_roots(
        instance: Arc<Instance>,
        root: NodeId,
        roots: &[NodeId],
        geometry: Geometry,
    ) -> Result<Self, String> {
        geometry.validate()?;
        let graph = instance.graph();
        if root as usize >= graph.nodes.len()
            || roots.iter().any(|node| *node as usize >= graph.nodes.len())
        {
            return Err("root node does not exist".into());
        }
        let mut seeds = roots.to_vec();
        seeds.push(root);
        let reachable = graph.reachable_from(&seeds);
        let mut interpolated = Vec::new();
        for (index, node) in graph.nodes.iter().enumerate() {
            if !reachable[index] {
                continue;
            }
            if let Node::Interpolated(inner) = node {
                let nested = graph.reachable_from(&[*inner]);
                let has_nested = graph.nodes.iter().enumerate().any(|(other, candidate)| {
                    nested[other] && matches!(candidate, Node::Interpolated(_))
                });
                if has_nested {
                    return Err("nested interpolated nodes are not supported".into());
                }
                interpolated.push(index as NodeId);
            }
        }
        let mut slot_of = vec![NO_SLOT; graph.nodes.len()];
        for (slot, node) in interpolated.iter().enumerate() {
            slot_of[*node as usize] = slot as u32;
        }
        let beardifier_slots = interpolated
            .iter()
            .map(|node| {
                let inner = match &graph.nodes[*node as usize] {
                    Node::Interpolated(inner) => *inner,
                    _ => return false,
                };
                let reached = graph.reachable_from(&[inner]);
                graph.nodes.iter().enumerate().any(|(index, candidate)| {
                    reached[index] && matches!(candidate, Node::Beardifier)
                })
            })
            .collect();
        Ok(Self {
            instance,
            root,
            geometry,
            interpolated,
            slot_of,
            beardifier_slots,
        })
    }

    pub fn slot_uses_beardifier(&self, slot: usize) -> bool {
        self.beardifier_slots.get(slot).copied().unwrap_or(true)
    }

    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    pub fn new_scratch(&self) -> Scratch {
        Scratch::new(&self.instance)
    }

    pub fn corner_columns_per_axis(&self) -> usize {
        self.geometry.cells_xz() + 1
    }

    pub fn interpolated_count(&self) -> usize {
        self.interpolated.len()
    }

    pub fn interpolated_inner(&self, slot: usize) -> Option<NodeId> {
        let node = *self.interpolated.get(slot)?;
        match &self.instance.graph().nodes[node as usize] {
            Node::Interpolated(inner) => Some(*inner),
            _ => None,
        }
    }

    pub fn corners_y(&self) -> usize {
        self.geometry.cells_y() + 1
    }

    pub fn grid_len(&self) -> usize {
        self.interpolated.len()
            * self.corner_columns_per_axis()
            * self.corner_columns_per_axis()
            * self.corners_y()
    }

    pub fn compute_corner_column(
        &self,
        scratch: &mut Scratch,
        chunk_x: i32,
        chunk_z: i32,
        corner_x: usize,
        corner_z: usize,
    ) -> Vec<f64> {
        let corners_y = self.geometry.cells_y() + 1;
        let x = chunk_x * CHUNK_SIZE + corner_x as i32 * self.geometry.cell_width;
        let z = chunk_z * CHUNK_SIZE + corner_z as i32 * self.geometry.cell_width;
        let graph = self.instance.graph();
        let mut column = Vec::with_capacity(self.interpolated.len() * corners_y);
        for node in &self.interpolated {
            let inner = match &graph.nodes[*node as usize] {
                Node::Interpolated(inner) => *inner,
                _ => unreachable!("slot list only holds interpolated nodes"),
            };
            for corner_y in 0..corners_y {
                let y = self.geometry.min_y + corner_y as i32 * self.geometry.cell_height;
                column.push(self.instance.eval(scratch, None, inner, x, y, z));
            }
        }
        column
    }

    pub fn build_grid(&self, chunk_x: i32, chunk_z: i32, columns: Vec<Vec<f64>>) -> CornerGrid {
        let corners_xz = self.corner_columns_per_axis();
        let corners_y = self.geometry.cells_y() + 1;
        assert_eq!(
            columns.len(),
            corners_xz * corners_xz,
            "corner column count"
        );
        let mut values = vec![0.0; self.interpolated.len() * corners_xz * corners_xz * corners_y];
        for (column_index, column) in columns.iter().enumerate() {
            assert_eq!(
                column.len(),
                self.interpolated.len() * corners_y,
                "corner column length"
            );
            let corner_x = column_index / corners_xz;
            let corner_z = column_index % corners_xz;
            for slot in 0..self.interpolated.len() {
                let target = ((slot * corners_xz + corner_x) * corners_xz + corner_z) * corners_y;
                values[target..target + corners_y]
                    .copy_from_slice(&column[slot * corners_y..(slot + 1) * corners_y]);
            }
        }
        CornerGrid {
            geometry: self.geometry,
            origin_x: chunk_x * CHUNK_SIZE,
            origin_z: chunk_z * CHUNK_SIZE,
            slot_of: self.slot_of.clone(),
            corners_xz,
            corners_y,
            values,
        }
    }

    pub fn fill_column(
        &self,
        scratch: &mut Scratch,
        grid: &CornerGrid,
        local_x: i32,
        local_z: i32,
        output: &mut [f64],
    ) {
        assert_eq!(
            output.len(),
            self.geometry.height as usize,
            "column buffer length"
        );
        let x = grid.origin_x + local_x;
        let z = grid.origin_z + local_z;
        for (offset, slot) in output.iter_mut().enumerate() {
            let y = self.geometry.min_y + offset as i32;
            *slot = self.instance.eval(scratch, Some(grid), self.root, x, y, z);
        }
    }

    pub fn fill_chunk_sequential(&self, chunk_x: i32, chunk_z: i32) -> Vec<f64> {
        let mut scratch = self.new_scratch();
        let per_axis = self.corner_columns_per_axis();
        let mut columns = Vec::with_capacity(per_axis * per_axis);
        for corner_x in 0..per_axis {
            for corner_z in 0..per_axis {
                columns.push(self.compute_corner_column(
                    &mut scratch,
                    chunk_x,
                    chunk_z,
                    corner_x,
                    corner_z,
                ));
            }
        }
        let grid = self.build_grid(chunk_x, chunk_z, columns);
        scratch.clear();
        let height = self.geometry.height as usize;
        let mut output = vec![0.0; CHUNK_SIZE as usize * CHUNK_SIZE as usize * height];
        for local_x in 0..CHUNK_SIZE {
            for local_z in 0..CHUNK_SIZE {
                let start = column_offset(local_x, local_z, height);
                self.fill_column(
                    &mut scratch,
                    &grid,
                    local_x,
                    local_z,
                    &mut output[start..start + height],
                );
            }
        }
        output
    }
}

#[inline]
pub fn column_offset(local_x: i32, local_z: i32, height: usize) -> usize {
    (local_x as usize * CHUNK_SIZE as usize + local_z as usize) * height
}

impl CornerGrid {
    pub fn values(&self) -> &[f64] {
        &self.values
    }

    #[inline]
    fn corner(&self, slot: usize, corner_x: usize, corner_y: usize, corner_z: usize) -> f64 {
        self.values[((slot * self.corners_xz + corner_x) * self.corners_xz + corner_z)
            * self.corners_y
            + corner_y]
    }
}

impl InterpolatedSource for CornerGrid {
    fn value(&self, node: NodeId, x: i32, y: i32, z: i32) -> f64 {
        let slot = self.slot_of[node as usize];
        debug_assert!(slot != NO_SLOT, "node is not an interpolated slot");
        let slot = slot as usize;
        let geometry = &self.geometry;
        let relative_x = x - self.origin_x;
        let relative_z = z - self.origin_z;
        let relative_y = y - geometry.min_y;
        debug_assert!(relative_x >= 0 && relative_z >= 0 && relative_y >= 0);

        let cell_x = (relative_x / geometry.cell_width) as usize;
        let cell_z = (relative_z / geometry.cell_width) as usize;
        let cell_y = (relative_y / geometry.cell_height) as usize;
        let tx = f64::from(relative_x % geometry.cell_width) / f64::from(geometry.cell_width);
        let tz = f64::from(relative_z % geometry.cell_width) / f64::from(geometry.cell_width);
        let ty = f64::from(relative_y % geometry.cell_height) / f64::from(geometry.cell_height);

        let v000 = self.corner(slot, cell_x, cell_y, cell_z);
        let v100 = self.corner(slot, cell_x + 1, cell_y, cell_z);
        let v010 = self.corner(slot, cell_x, cell_y + 1, cell_z);
        let v110 = self.corner(slot, cell_x + 1, cell_y + 1, cell_z);
        let v001 = self.corner(slot, cell_x, cell_y, cell_z + 1);
        let v101 = self.corner(slot, cell_x + 1, cell_y, cell_z + 1);
        let v011 = self.corner(slot, cell_x, cell_y + 1, cell_z + 1);
        let v111 = self.corner(slot, cell_x + 1, cell_y + 1, cell_z + 1);

        let along_y_x0_z0 = lerp(ty, v000, v010);
        let along_y_x1_z0 = lerp(ty, v100, v110);
        let along_y_x0_z1 = lerp(ty, v001, v011);
        let along_y_x1_z1 = lerp(ty, v101, v111);
        let along_x_z0 = lerp(tx, along_y_x0_z0, along_y_x1_z0);
        let along_x_z1 = lerp(tx, along_y_x0_z1, along_y_x1_z1);
        lerp(tz, along_x_z0, along_x_z1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;

    const SEED: i64 = 20_261_013;
    const GEOMETRY: Geometry = Geometry {
        min_y: -64,
        height: 384,
        cell_width: 4,
        cell_height: 8,
    };

    fn plan(router: &str) -> ChunkPlan {
        let document = format!(
            r#"{{"noises":{{"minecraft:test":{{"firstOctave":-3,"amplitudes":[1.0,1.0,0.5]}}}},"router":{{"out":{router}}}}}"#
        );
        let graph = Graph::parse(&document).expect("graph");
        let root = graph.root("out").expect("root");
        let instance = Arc::new(Instance::new(Arc::new(graph), SEED));
        ChunkPlan::new(instance, root, GEOMETRY).expect("plan")
    }

    const NOISY: &str = r#"{"type":"minecraft:add","argument1":{"type":"minecraft:interpolated","argument":{"type":"minecraft:noise","noise":"minecraft:test","xz_scale":0.05,"y_scale":0.05}},"argument2":{"type":"minecraft:y_clamped_gradient","from_y":-64,"to_y":320,"from_value":1.0,"to_value":-1.0}}"#;

    #[test]
    fn geometry_validation() {
        assert!(GEOMETRY.validate().is_ok());
        let bad = [
            Geometry {
                cell_width: 3,
                ..GEOMETRY
            },
            Geometry {
                cell_height: 0,
                ..GEOMETRY
            },
            Geometry {
                height: 380,
                ..GEOMETRY
            },
            Geometry {
                min_y: -60,
                ..GEOMETRY
            },
        ];
        for geometry in bad {
            assert!(geometry.validate().is_err(), "{geometry:?}");
        }
        assert_eq!(GEOMETRY.cells_xz(), 4);
        assert_eq!(GEOMETRY.cells_y(), 48);
    }

    #[test]
    fn corner_positions_reproduce_inner_values_exactly() {
        let plan = plan(NOISY);
        let chunk = plan.fill_chunk_sequential(3, -2);
        let mut scratch = plan.new_scratch();
        let graph = plan.instance().graph();
        let height = GEOMETRY.height as usize;
        for local_x in [0, 4, 8, 12] {
            for local_z in [0, 4, 12] {
                for y in [-64, -8, 0, 64, 200, 312] {
                    let x = 3 * 16 + local_x;
                    let z = -2 * 16 + local_z;
                    let inner = match &graph.nodes[graph.interpolated_nodes()[0] as usize] {
                        Node::Interpolated(inner) => *inner,
                        _ => unreachable!(),
                    };
                    let expected =
                        plan.instance().eval(&mut scratch, None, inner, x, y, z) + gradient(y);
                    let offset =
                        column_offset(local_x, local_z, height) + (y - GEOMETRY.min_y) as usize;
                    assert_eq!(chunk[offset], expected, "x={x} y={y} z={z}");
                }
            }
        }
    }

    fn gradient(y: i32) -> f64 {
        let delta = (f64::from(y) + 64.0) / 384.0;
        1.0 + delta * (-1.0 - 1.0)
    }

    #[test]
    fn linear_inner_function_interpolates_exactly() {
        let plan = plan(
            r#"{"type":"minecraft:interpolated","argument":{"type":"minecraft:y_clamped_gradient","from_y":-64,"to_y":320,"from_value":-3.0,"to_value":5.0}}"#,
        );
        let chunk = plan.fill_chunk_sequential(0, 0);
        let height = GEOMETRY.height as usize;
        for local_x in 0..16 {
            for local_z in 0..16 {
                for y_index in 0..height {
                    let y = GEOMETRY.min_y + y_index as i32;
                    let expected = -3.0 + (f64::from(y) + 64.0) / 384.0 * 8.0;
                    let actual = chunk[column_offset(local_x, local_z, height) + y_index];
                    assert!(
                        (actual - expected).abs() < 1.0e-9,
                        "y={y}: {actual} vs {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn fill_is_deterministic_and_chunk_dependent() {
        let plan = plan(NOISY);
        let first = plan.fill_chunk_sequential(5, 7);
        let again = plan.fill_chunk_sequential(5, 7);
        let other = plan.fill_chunk_sequential(6, 7);
        assert_eq!(first, again);
        assert_ne!(first, other);
        assert!(first.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn non_interpolated_graph_needs_no_corners() {
        let plan = plan(
            r#"{"type":"minecraft:y_clamped_gradient","from_y":0,"to_y":64,"from_value":0.0,"to_value":1.0}"#,
        );
        assert_eq!(plan.interpolated_count(), 0);
        let chunk = plan.fill_chunk_sequential(0, 0);
        let height = GEOMETRY.height as usize;
        assert_eq!(
            chunk[column_offset(3, 9, height) + (32 - GEOMETRY.min_y) as usize],
            0.5
        );
    }

    #[test]
    fn multi_root_plan_collects_every_interpolated_node() {
        let document = r#"{"router":{
            "a":{"type":"minecraft:interpolated","argument":1.5},
            "b":{"type":"minecraft:add","argument1":{"type":"minecraft:interpolated","argument":2.5},"argument2":1.0}
        }}"#;
        let graph = Graph::parse(document).unwrap();
        let a = graph.root("a").unwrap();
        let b = graph.root("b").unwrap();
        let instance = Arc::new(Instance::new(Arc::new(graph), SEED));
        let single = ChunkPlan::new(Arc::clone(&instance), a, GEOMETRY).unwrap();
        assert_eq!(single.interpolated_count(), 1);
        let both = ChunkPlan::with_roots(instance, a, &[a, b], GEOMETRY).unwrap();
        assert_eq!(both.interpolated_count(), 2);
        let inner_values: Vec<f64> = (0..2)
            .map(|slot| {
                let mut scratch = both.new_scratch();
                let inner = both.interpolated_inner(slot).unwrap();
                both.instance().eval(&mut scratch, None, inner, 0, 0, 0)
            })
            .collect();
        assert_eq!(inner_values, vec![1.5, 2.5]);
        assert!(both.interpolated_inner(2).is_none());
        assert_eq!(both.grid_len(), 2 * 5 * 5 * 49);
    }

    #[test]
    fn corner_grid_layout_is_slot_major() {
        let plan = plan(NOISY);
        let per_axis = plan.corner_columns_per_axis();
        let mut scratch = plan.new_scratch();
        let columns: Vec<Vec<f64>> = (0..per_axis * per_axis)
            .map(|index| {
                plan.compute_corner_column(&mut scratch, 1, 1, index / per_axis, index % per_axis)
            })
            .collect();
        let grid = plan.build_grid(1, 1, columns.clone());
        assert_eq!(grid.values().len(), plan.grid_len());
        let corners_y = plan.corners_y();
        for corner_x in 0..per_axis {
            for corner_z in 0..per_axis {
                let start = (corner_x * per_axis + corner_z) * corners_y;
                assert_eq!(
                    &grid.values()[start..start + corners_y],
                    &columns[corner_x * per_axis + corner_z][..corners_y]
                );
            }
        }
    }

    #[test]
    fn beardifier_dependency_is_flagged_per_slot() {
        let document = r#"{"router":{
            "a":{"type":"minecraft:interpolated","argument":{"type":"minecraft:add","argument1":1.0,"argument2":{"type":"minecraft:beardifier"}}},
            "b":{"type":"minecraft:interpolated","argument":2.0}
        }}"#;
        let graph = Graph::parse(document).unwrap();
        let a = graph.root("a").unwrap();
        let b = graph.root("b").unwrap();
        let instance = Arc::new(Instance::new(Arc::new(graph), SEED));
        let plan = ChunkPlan::with_roots(instance, a, &[a, b], GEOMETRY).unwrap();
        assert!(plan.slot_uses_beardifier(0));
        assert!(!plan.slot_uses_beardifier(1));
        assert!(plan.slot_uses_beardifier(9));
    }

    #[test]
    fn nested_interpolation_is_rejected() {
        let document = r#"{"router":{"out":{"type":"minecraft:interpolated","argument":{"type":"minecraft:interpolated","argument":1.0}}}}"#;
        let graph = Graph::parse(document).unwrap();
        let root = graph.root("out").unwrap();
        let instance = Arc::new(Instance::new(Arc::new(graph), SEED));
        assert!(ChunkPlan::new(instance, root, GEOMETRY).is_err());
    }

    #[test]
    fn split_corner_computation_matches_sequential_fill() {
        let plan = plan(NOISY);
        let reference = plan.fill_chunk_sequential(-4, 9);
        let per_axis = plan.corner_columns_per_axis();
        let mut columns = Vec::new();
        for corner_x in 0..per_axis {
            for corner_z in 0..per_axis {
                let mut scratch = plan.new_scratch();
                columns.push(plan.compute_corner_column(&mut scratch, -4, 9, corner_x, corner_z));
            }
        }
        let grid = plan.build_grid(-4, 9, columns);
        let height = GEOMETRY.height as usize;
        let mut rebuilt = vec![0.0; 16 * 16 * height];
        for local_x in (0..16).rev() {
            for local_z in (0..16).rev() {
                let mut scratch = plan.new_scratch();
                let start = column_offset(local_x, local_z, height);
                plan.fill_column(
                    &mut scratch,
                    &grid,
                    local_x,
                    local_z,
                    &mut rebuilt[start..start + height],
                );
            }
        }
        assert_eq!(reference, rebuilt);
    }
}
