struct Params {
    adaptation: f32,
    darkening: f32,
    target_luminance: f32,
    min_exposure: f32,
    max_exposure: f32,
    ev_scale: f32,
    auto_enabled: u32,
}

struct State {
    adapted_luminance: f32,
    exposure: f32,
}

const GRID: u32 = 64u;
const THREADS: u32 = 256u;
const SAMPLES_PER_THREAD: u32 = GRID * GRID / THREADS;
const MIN_LUMINANCE: f32 = 1e-4;
const MAX_LUMINANCE: f32 = 1e6;
const EXPONENT_MASK: u32 = 0x7f800000u;

@group(0) @binding(0) var hdr_scene: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> state: State;

var<workgroup> partial_sums: array<f32, THREADS>;

fn luminance(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn is_finite(value: f32) -> bool {
    return (bitcast<u32>(value) & EXPONENT_MASK) != EXPONENT_MASK;
}

fn sample_luminance(rgb: vec3<f32>) -> f32 {
    let value = luminance(rgb);
    return select(MIN_LUMINANCE, clamp(value, MIN_LUMINANCE, MAX_LUMINANCE), is_finite(value));
}

// Towards a brighter picture `params.adaptation` is the blend factor; towards a darker one the
// adapted luminance keeps at least `params.darkening` of itself. Both are scaled by the frame time.
fn adapt(adapted: f32, measured: f32) -> f32 {
    if !is_finite(adapted) || adapted <= 0.0 {
        return measured;
    }
    if measured < adapted {
        return max(measured, adapted * params.darkening);
    }
    return adapted + (measured - adapted) * params.adaptation;
}

@compute @workgroup_size(256)
fn main(@builtin(local_invocation_index) thread: u32) {
    let size = vec2<f32>(textureDimensions(hdr_scene));
    var total = 0.0;
    for (var i = 0u; i < SAMPLES_PER_THREAD; i = i + 1u) {
        let sample_index = thread + i * THREADS;
        let uv = (vec2<f32>(f32(sample_index % GRID), f32(sample_index / GRID)) + 0.5) / f32(GRID);
        let rgb = textureLoad(hdr_scene, vec2<i32>(uv * size), 0).rgb;
        total += sample_luminance(rgb);
    }
    partial_sums[thread] = total;

    for (var stride = THREADS / 2u; stride > 0u; stride = stride / 2u) {
        workgroupBarrier();
        if thread < stride {
            partial_sums[thread] += partial_sums[thread + stride];
        }
    }

    if thread == 0u {
        let measured = partial_sums[0] / f32(GRID * GRID);
        let adapted = adapt(state.adapted_luminance, measured);
        state.adapted_luminance = adapted;
        let automatic = clamp(params.target_luminance / adapted, params.min_exposure, params.max_exposure);
        state.exposure = select(1.0, automatic, params.auto_enabled != 0u) * params.ev_scale;
    }
}
