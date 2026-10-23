// The ambient environment is a per-channel affine function of direction in
// gamma space, expanded to linear radiance with a power.
const AMBIENT_GAMMA = 2.2;
const AMBIENT_FACE_COUNT = 6.0;

struct AmbientGradient {
    // Columns: gamma-space change of (r, g, b) along +X, +Y, +Z.
    slope: mat3x3<f32>,
    // Gamma-space colour in the direction where every axis term vanishes.
    offset: vec3<f32>,
}

fn encode_ambient_gamma(linear_color: vec3<f32>) -> vec3<f32> {
    return pow_or_zero3(linear_color, 1.0 / AMBIENT_GAMMA);
}

// Gamma-space change per unit of travel along an axis: half the difference of its two faces.
fn axis_slope(positive_face: vec3<f32>, negative_face: vec3<f32>) -> vec3<f32> {
    return 0.5 * (positive_face - negative_face);
}

// Fits the six cube faces (+X, -X, +Y, -Y, +Z, -Z): each axis' two faces give
// its slope, the mean of all faces the offset.
fn ambient_gradient() -> AmbientGradient {
    let pos_x = encode_ambient_gamma(frame.ambient_cube[0].rgb);
    let neg_x = encode_ambient_gamma(frame.ambient_cube[1].rgb);
    let pos_y = encode_ambient_gamma(frame.ambient_cube[2].rgb);
    let neg_y = encode_ambient_gamma(frame.ambient_cube[3].rgb);
    let pos_z = encode_ambient_gamma(frame.ambient_cube[4].rgb);
    let neg_z = encode_ambient_gamma(frame.ambient_cube[5].rgb);
    return AmbientGradient(
        mat3x3<f32>(axis_slope(pos_x, neg_x), axis_slope(pos_y, neg_y), axis_slope(pos_z, neg_z)),
        (pos_x + neg_x + pos_y + neg_y + pos_z + neg_z) / AMBIENT_FACE_COUNT,
    );
}

// Ambient radiance arriving from `direction` (world space, unit length).
fn ambient_toward(gradient: AmbientGradient, direction: vec3<f32>) -> vec3<f32> {
    return pow_or_zero3(gradient.slope * direction + gradient.offset, AMBIENT_GAMMA);
}
