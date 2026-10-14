use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use rayon::prelude::*;
use rayon::{ThreadPool, ThreadPoolBuilder};
use terrain_core::chunk::{column_offset, ChunkPlan, CornerGrid, Geometry, CHUNK_SIZE};
use terrain_core::density::{Instance, Scratch};
use terrain_core::graph::{Graph, NodeId};

const MAX_WORKERS: usize = 64;

static REQUESTED_THREADS: AtomicUsize = AtomicUsize::new(0);

static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static WORLDS: OnceLock<RwLock<HashMap<u64, Arc<World>>>> = OnceLock::new();

fn registry() -> &'static RwLock<HashMap<u64, Arc<World>>> {
    WORLDS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn request_threads(count: usize) -> bool {
    if POOL.get().is_some() {
        return false;
    }
    REQUESTED_THREADS.store(count, Ordering::Relaxed);
    true
}

fn default_threads() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(2);
    (cores / 2).max(1)
}

pub fn pool() -> Option<&'static ThreadPool> {
    POOL.get_or_init(|| {
        let requested = REQUESTED_THREADS.load(Ordering::Relaxed);
        let threads = if requested > 0 {
            requested
        } else {
            default_threads()
        }
        .clamp(1, MAX_WORKERS);
        ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("rustmod-worker-{index}"))
            .build()
            .ok()
    })
    .as_ref()
}

pub fn worker_threads() -> usize {
    pool().map_or(0, |pool| pool.current_num_threads())
}

pub struct World {
    plan: ChunkPlan,
    instance: Arc<Instance>,
}

pub fn create_world(
    seed: i64,
    graph_json: &str,
    geometry: Geometry,
    root_name: &str,
) -> Result<u64, String> {
    let graph = Graph::parse(graph_json).map_err(|error| error.to_string())?;
    let root = graph
        .root(root_name)
        .ok_or_else(|| format!("router entry {root_name} is missing"))?;
    let all_roots: Vec<NodeId> = graph.roots.iter().map(|(_, node)| *node).collect();
    let instance = Arc::new(Instance::new(Arc::new(graph), seed));
    let plan = ChunkPlan::with_roots(Arc::clone(&instance), root, &all_roots, geometry)?;
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    let world = Arc::new(World { plan, instance });
    match registry().write() {
        Ok(mut guard) => guard.insert(handle, world),
        Err(poisoned) => poisoned.into_inner().insert(handle, world),
    };
    Ok(handle)
}

pub fn destroy_world(handle: u64) -> bool {
    match registry().write() {
        Ok(mut guard) => guard.remove(&handle).is_some(),
        Err(poisoned) => poisoned.into_inner().remove(&handle).is_some(),
    }
}

pub fn find_world(handle: u64) -> Option<Arc<World>> {
    match registry().read() {
        Ok(guard) => guard.get(&handle).cloned(),
        Err(poisoned) => poisoned.into_inner().get(&handle).cloned(),
    }
}

impl World {
    pub fn column_count() -> usize {
        (CHUNK_SIZE * CHUNK_SIZE) as usize
    }

    pub fn output_len(&self) -> usize {
        Self::column_count() * self.plan.geometry().height as usize
    }

    pub fn geometry(&self) -> Geometry {
        self.plan.geometry()
    }

    pub fn root_index(&self, name: &str) -> Option<usize> {
        self.instance
            .graph()
            .roots
            .iter()
            .position(|(root_name, _)| root_name == name)
    }

    pub fn fill_chunk(&self, chunk_x: i32, chunk_z: i32, output: &mut [f64]) -> Result<(), String> {
        if output.len() != self.output_len() {
            return Err(format!(
                "output buffer holds {} values, {} required",
                output.len(),
                self.output_len()
            ));
        }
        match pool() {
            Some(pool) => {
                pool.install(|| self.fill_parallel(chunk_x, chunk_z, output));
            }
            None => {
                let sequential = self.plan.fill_chunk_sequential(chunk_x, chunk_z);
                output.copy_from_slice(&sequential);
            }
        }
        Ok(())
    }

    pub fn slots(&self) -> usize {
        self.plan.interpolated_count()
    }

    pub fn grid_len(&self) -> usize {
        self.plan.grid_len()
    }

    pub fn slot_uses_beardifier(&self, slot: usize) -> bool {
        self.plan.slot_uses_beardifier(slot)
    }

    fn compute_grid(&self, chunk_x: i32, chunk_z: i32) -> CornerGrid {
        let plan = &self.plan;
        let per_axis = plan.corner_columns_per_axis();
        let columns: Vec<Vec<f64>> = (0..per_axis * per_axis)
            .into_par_iter()
            .map_init(
                || plan.new_scratch(),
                |scratch, index| {
                    plan.compute_corner_column(
                        scratch,
                        chunk_x,
                        chunk_z,
                        index / per_axis,
                        index % per_axis,
                    )
                },
            )
            .collect();
        plan.build_grid(chunk_x, chunk_z, columns)
    }

    pub fn corner_grid(
        &self,
        chunk_x: i32,
        chunk_z: i32,
        output: &mut [f64],
    ) -> Result<(), String> {
        if output.len() != self.grid_len() {
            return Err(format!(
                "grid buffer holds {} values, {} required",
                output.len(),
                self.grid_len()
            ));
        }
        let grid = match pool() {
            Some(pool) => pool.install(|| self.compute_grid(chunk_x, chunk_z)),
            None => {
                let plan = &self.plan;
                let per_axis = plan.corner_columns_per_axis();
                let mut scratch = plan.new_scratch();
                let mut columns = Vec::with_capacity(per_axis * per_axis);
                for corner_x in 0..per_axis {
                    for corner_z in 0..per_axis {
                        columns.push(plan.compute_corner_column(
                            &mut scratch,
                            chunk_x,
                            chunk_z,
                            corner_x,
                            corner_z,
                        ));
                    }
                }
                plan.build_grid(chunk_x, chunk_z, columns)
            }
        };
        output.copy_from_slice(grid.values());
        Ok(())
    }

    pub fn probe(&self, points: &[i32], output: &mut [f64]) -> Result<(), String> {
        let point_count = points.len() / 3;
        if points.len() % 3 != 0 || output.len() != self.slots() * point_count {
            return Err("probe output must hold one value per slot and point".into());
        }
        let mut scratch = Scratch::exact(&self.instance);
        for slot in 0..self.slots() {
            let inner = self
                .plan
                .interpolated_inner(slot)
                .ok_or_else(|| format!("slot {slot} has no inner function"))?;
            for point in 0..point_count {
                let base = point * 3;
                output[slot * point_count + point] = self.instance.eval(
                    &mut scratch,
                    None,
                    inner,
                    points[base],
                    points[base + 1],
                    points[base + 2],
                );
            }
        }
        Ok(())
    }

    fn fill_parallel(&self, chunk_x: i32, chunk_z: i32, output: &mut [f64]) {
        let plan = &self.plan;
        let grid = self.compute_grid(chunk_x, chunk_z);
        let height = plan.geometry().height as usize;
        output.par_chunks_mut(height).enumerate().for_each_init(
            || plan.new_scratch(),
            |scratch, (index, column)| {
                let local_x = (index / CHUNK_SIZE as usize) as i32;
                let local_z = (index % CHUNK_SIZE as usize) as i32;
                debug_assert_eq!(column_offset(local_x, local_z, height), index * height);
                plan.fill_column(scratch, &grid, local_x, local_z, column);
            },
        );
    }

    pub fn evaluate(&self, root: usize, points: &[i32], output: &mut [f64]) -> Result<(), String> {
        if points.len() % 3 != 0 || points.len() / 3 != output.len() {
            return Err("points must hold x, y, z triples matching the output length".into());
        }
        let node = self
            .instance
            .graph()
            .roots
            .get(root)
            .map(|(_, node)| *node)
            .ok_or_else(|| format!("root index {root} is out of range"))?;
        let instance = &self.instance;
        let run = |output: &mut [f64]| {
            output.par_iter_mut().enumerate().for_each_init(
                || Scratch::exact(instance),
                |scratch, (index, slot)| {
                    let base = index * 3;
                    *slot = instance.eval(
                        scratch,
                        None,
                        node,
                        points[base],
                        points[base + 1],
                        points[base + 2],
                    );
                },
            );
        };
        match pool() {
            Some(pool) => pool.install(|| run(output)),
            None => {
                let mut scratch = Scratch::exact(instance);
                for (index, slot) in output.iter_mut().enumerate() {
                    let base = index * 3;
                    *slot = instance.eval(
                        &mut scratch,
                        None,
                        node,
                        points[base],
                        points[base + 1],
                        points[base + 2],
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GEOMETRY: Geometry = Geometry {
        min_y: -64,
        height: 384,
        cell_width: 4,
        cell_height: 8,
    };

    const DOCUMENT: &str = r#"{
        "noises": {"minecraft:test": {"firstOctave": -3, "amplitudes": [1.0, 1.0, 0.5]}},
        "functions": {
            "test:base": {"type": "minecraft:noise", "noise": "minecraft:test", "xz_scale": 0.05, "y_scale": 0.05}
        },
        "router": {
            "continents": "test:base",
            "final_density": {
                "type": "minecraft:add",
                "argument1": {"type": "minecraft:interpolated", "argument": "test:base"},
                "argument2": {"type": "minecraft:y_clamped_gradient", "from_y": -64, "to_y": 320, "from_value": 1.0, "to_value": -1.0}
            }
        }
    }"#;

    fn world(seed: i64) -> (u64, Arc<World>) {
        let handle = create_world(seed, DOCUMENT, GEOMETRY, "final_density").expect("world");
        (handle, find_world(handle).expect("registered"))
    }

    #[test]
    fn parallel_fill_matches_sequential_reference() {
        let (handle, world) = world(77);
        let mut output = vec![0.0; world.output_len()];
        world.fill_chunk(12, -5, &mut output).unwrap();
        let reference = world.plan.fill_chunk_sequential(12, -5);
        assert_eq!(output, reference);
        assert!(destroy_world(handle));
    }

    #[test]
    fn corner_grid_covers_every_router_entry() {
        let (handle, world) = world(31);
        assert_eq!(world.slots(), 1);
        assert_eq!(world.grid_len(), 1 * 5 * 5 * 49);
        let mut grid = vec![0.0; world.grid_len()];
        world.corner_grid(2, -3, &mut grid).unwrap();
        let mut wrong = vec![0.0; 7];
        assert!(world.corner_grid(2, -3, &mut wrong).is_err());

        let mut probe = vec![0.0; world.slots() * 2];
        let points = [32, -64, -48, 36, -56, -48];
        world.probe(&points, &mut probe).unwrap();
        assert_eq!(probe[0], grid[0]);
        let corners_y = 49;
        assert_eq!(probe[1], grid[(1 * 5 + 0) * corners_y + 1]);
        assert!(world.probe(&points[..5], &mut probe).is_err());
        destroy_world(handle);
    }

    #[test]
    fn fill_rejects_wrong_buffer_size() {
        let (handle, world) = world(1);
        let mut small = vec![0.0; 10];
        assert!(world.fill_chunk(0, 0, &mut small).is_err());
        destroy_world(handle);
    }

    #[test]
    fn destroyed_worlds_stay_alive_for_inflight_callers() {
        let (handle, world) = world(5);
        assert!(destroy_world(handle));
        assert!(find_world(handle).is_none());
        assert!(!destroy_world(handle));
        let mut output = vec![0.0; world.output_len()];
        assert!(world.fill_chunk(0, 0, &mut output).is_ok());
    }

    #[test]
    fn concurrent_callers_get_identical_results() {
        let (handle, world) = world(2026);
        let reference = world.plan.fill_chunk_sequential(3, 3);
        let results: Vec<Vec<f64>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..6)
                .map(|_| {
                    scope.spawn(|| {
                        let mut output = vec![0.0; world.output_len()];
                        world.fill_chunk(3, 3, &mut output).unwrap();
                        output
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect()
        });
        assert!(results.iter().all(|result| *result == reference));
        destroy_world(handle);
    }

    #[test]
    fn evaluate_matches_graph_evaluation() {
        let (handle, world) = world(9);
        let root = world.root_index("continents").expect("root");
        assert_eq!(world.root_index("missing"), None);
        let points = [10, 20, 30, -5, 64, 1000, 0, 0, 0];
        let mut output = vec![0.0; 3];
        world.evaluate(root, &points, &mut output).unwrap();
        let mut scratch = Scratch::exact(&world.instance);
        let node = world.instance.graph().roots[root].1;
        for (index, value) in output.iter().enumerate() {
            let base = index * 3;
            let expected = world.instance.eval(
                &mut scratch,
                None,
                node,
                points[base],
                points[base + 1],
                points[base + 2],
            );
            assert_eq!(*value, expected);
        }
        assert!(world.evaluate(99, &points, &mut output).is_err());
        assert!(world.evaluate(root, &points[..8], &mut output).is_err());
        destroy_world(handle);
    }

    #[test]
    fn invalid_documents_are_reported() {
        assert!(create_world(1, "{", GEOMETRY, "final_density").is_err());
        assert!(create_world(1, DOCUMENT, GEOMETRY, "nonexistent").is_err());
        let bad_geometry = Geometry {
            cell_width: 3,
            ..GEOMETRY
        };
        assert!(create_world(1, DOCUMENT, bad_geometry, "final_density").is_err());
    }
}
