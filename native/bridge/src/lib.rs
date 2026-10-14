mod world;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};

use jni::objects::{JByteBuffer, JClass, JDoubleArray, JIntArray, JString};
use jni::sys::{jboolean, jint, jlong, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use terrain_core::chunk::Geometry;

const STATUS_OK: jint = 0;
const STATUS_UNKNOWN_WORLD: jint = 1;
const STATUS_BAD_BUFFER: jint = 2;
const STATUS_FAILED: jint = 3;
const STATUS_PANIC: jint = 4;

static PANICS: AtomicU64 = AtomicU64::new(0);

fn guard<T>(fallback: T, body: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(_) => {
            PANICS.fetch_add(1, Ordering::Relaxed);
            fallback
        }
    }
}

fn throw(env: &mut JNIEnv, class: &str, message: &str) {
    if !env.exception_check().unwrap_or(true) {
        let _ = env.throw_new(class, message);
    }
}

fn write_buffer(
    env: &JNIEnv,
    buffer: &JByteBuffer,
    needed: usize,
    fill: impl FnOnce(&mut [f64]) -> Result<(), String>,
) -> jint {
    let address = match env.get_direct_buffer_address(buffer) {
        Ok(pointer) if !pointer.is_null() => pointer,
        _ => return STATUS_BAD_BUFFER,
    };
    let capacity = env.get_direct_buffer_capacity(buffer).unwrap_or(0);
    let Some(bytes) = needed.checked_mul(std::mem::size_of::<f64>()) else {
        return STATUS_BAD_BUFFER;
    };
    if capacity < bytes {
        return STATUS_BAD_BUFFER;
    }
    if (address as usize) % std::mem::align_of::<f64>() == 0 {
        let target = unsafe { std::slice::from_raw_parts_mut(address as *mut f64, needed) };
        match fill(target) {
            Ok(()) => STATUS_OK,
            Err(_) => STATUS_FAILED,
        }
    } else {
        let mut staging = vec![0.0f64; needed];
        match fill(&mut staging) {
            Ok(()) => {
                unsafe {
                    std::ptr::copy_nonoverlapping(staging.as_ptr() as *const u8, address, bytes);
                }
                STATUS_OK
            }
            Err(_) => STATUS_FAILED,
        }
    }
}

fn read_ints(env: &mut JNIEnv, array: &JIntArray) -> Option<Vec<jint>> {
    let length = env.get_array_length(array).ok()?;
    if length < 0 {
        return None;
    }
    let mut values = vec![0 as jint; length as usize];
    env.get_int_array_region(array, 0, &mut values).ok()?;
    Some(values)
}

fn write_doubles(env: &mut JNIEnv, array: &JDoubleArray, values: &[f64]) -> bool {
    env.set_double_array_region(array, 0, values).is_ok()
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_createWorld<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    seed: jlong,
    graph_json: JString<'local>,
    min_y: jint,
    height: jint,
    cell_width: jint,
    cell_height: jint,
    root_name: JString<'local>,
) -> jlong {
    let json: String = match env.get_string(&graph_json) {
        Ok(value) => value.into(),
        Err(_) => return 0,
    };
    let root: String = match env.get_string(&root_name) {
        Ok(value) => value.into(),
        Err(_) => return 0,
    };
    let geometry = Geometry {
        min_y,
        height,
        cell_width,
        cell_height,
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        world::create_world(seed, &json, geometry, &root)
    }));
    match outcome {
        Ok(Ok(handle)) => handle as jlong,
        Ok(Err(message)) => {
            throw(&mut env, "java/lang/IllegalArgumentException", &message);
            0
        }
        Err(_) => {
            PANICS.fetch_add(1, Ordering::Relaxed);
            throw(&mut env, "java/lang/IllegalStateException", "native panic");
            0
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_destroyWorld<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jboolean {
    guard(JNI_FALSE, || {
        if handle > 0 && world::destroy_world(handle as u64) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_configureThreads<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    count: jint,
) -> jboolean {
    guard(JNI_FALSE, || {
        if count > 0 && world::request_threads(count as usize) {
            JNI_TRUE
        } else {
            JNI_FALSE
        }
    })
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_workerThreads<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jint {
    guard(0, || world::worker_threads() as jint)
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_panicCount<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jlong {
    PANICS.load(Ordering::Relaxed) as jlong
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_findRoot<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    name: JString<'local>,
) -> jint {
    if handle <= 0 {
        return -1;
    }
    let name: String = match env.get_string(&name) {
        Ok(value) => value.into(),
        Err(_) => return -1,
    };
    guard(-1, || {
        let Some(found) = world::find_world(handle as u64) else {
            return -1;
        };
        found.root_index(&name).map_or(-1, |index| index as jint)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_fillChunk<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    chunk_x: jint,
    chunk_z: jint,
    buffer: JByteBuffer<'local>,
) -> jint {
    if handle <= 0 {
        return STATUS_UNKNOWN_WORLD;
    }
    guard(STATUS_PANIC, || {
        let Some(found) = world::find_world(handle as u64) else {
            return STATUS_UNKNOWN_WORLD;
        };
        write_buffer(&env, &buffer, found.output_len(), |target| {
            found.fill_chunk(chunk_x, chunk_z, target)
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_evaluate<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    root: jint,
    points: JIntArray<'local>,
    output: JDoubleArray<'local>,
) -> jint {
    if handle <= 0 || root < 0 {
        return STATUS_UNKNOWN_WORLD;
    }
    let Some(coordinates) = read_ints(&mut env, &points) else {
        return STATUS_FAILED;
    };
    let output_count = match env.get_array_length(&output) {
        Ok(length) if length >= 0 => length as usize,
        _ => return STATUS_FAILED,
    };
    let computed = guard(Err(STATUS_PANIC), || {
        let Some(found) = world::find_world(handle as u64) else {
            return Err(STATUS_UNKNOWN_WORLD);
        };
        let mut values = vec![0.0f64; output_count];
        match found.evaluate(root as usize, &coordinates, &mut values) {
            Ok(()) => Ok(values),
            Err(_) => Err(STATUS_FAILED),
        }
    });
    match computed {
        Ok(values) if write_doubles(&mut env, &output, &values) => STATUS_OK,
        Ok(_) => STATUS_FAILED,
        Err(status) => status,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_slotCount<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jint {
    if handle <= 0 {
        return -1;
    }
    guard(-1, || {
        world::find_world(handle as u64).map_or(-1, |found| found.slots() as jint)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_gridLength<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jint {
    if handle <= 0 {
        return -1;
    }
    guard(-1, || {
        world::find_world(handle as u64).map_or(-1, |found| found.grid_len() as jint)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_cornerGrid<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    chunk_x: jint,
    chunk_z: jint,
    buffer: JByteBuffer<'local>,
) -> jint {
    if handle <= 0 {
        return STATUS_UNKNOWN_WORLD;
    }
    guard(STATUS_PANIC, || {
        let Some(found) = world::find_world(handle as u64) else {
            return STATUS_UNKNOWN_WORLD;
        };
        write_buffer(&env, &buffer, found.grid_len(), |target| {
            found.corner_grid(chunk_x, chunk_z, target)
        })
    })
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_probeInterpolated<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    points: JIntArray<'local>,
    output: JDoubleArray<'local>,
) -> jint {
    if handle <= 0 {
        return STATUS_UNKNOWN_WORLD;
    }
    let Some(coordinates) = read_ints(&mut env, &points) else {
        return STATUS_FAILED;
    };
    let output_count = match env.get_array_length(&output) {
        Ok(length) if length >= 0 => length as usize,
        _ => return STATUS_FAILED,
    };
    let computed = guard(Err(STATUS_PANIC), || {
        let Some(found) = world::find_world(handle as u64) else {
            return Err(STATUS_UNKNOWN_WORLD);
        };
        let mut values = vec![0.0f64; output_count];
        match found.probe(&coordinates, &mut values) {
            Ok(()) => Ok(values),
            Err(_) => Err(STATUS_FAILED),
        }
    });
    match computed {
        Ok(values) if write_doubles(&mut env, &output, &values) => STATUS_OK,
        Ok(_) => STATUS_FAILED,
        Err(status) => status,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_example_rustmod_RustBridge_slotUsesBeardifier<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    slot: jint,
) -> jint {
    if handle <= 0 || slot < 0 {
        return -1;
    }
    guard(-1, || {
        world::find_world(handle as u64).map_or(-1, |found| {
            if found.slot_uses_beardifier(slot as usize) {
                1
            } else {
                0
            }
        })
    })
}
