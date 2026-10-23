const MIN_FRAME_LENGTH_SQ = 1e-20;
const AUTHORED_TANGENT_MIN_W = 0.5;

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    return v * inverseSqrt(max(dot(v, v), MIN_FRAME_LENGTH_SQ));
}

// BC5 normal maps carry xy only; DirectX convention: green points along +dP/dv.
fn decode_normal_map(encoded: vec4<f32>) -> vec3<f32> {
    let xy = encoded.rg * 2.0 - 1.0;
    return vec3<f32>(xy.x, xy.y, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

// Schüler's cotangent frame: T along +dP/du, B along +dP/dv, scale-invariant.
// Uses derivatives, so it must be evaluated in uniform control flow.
fn cotangent_frame(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>) -> mat3x3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let inv_max = inverseSqrt(max(max(dot(t, t), dot(b, b)), MIN_FRAME_LENGTH_SQ));
    return mat3x3<f32>(t * inv_max, b * inv_max, n);
}

fn authored_tangent_frame(n: vec3<f32>, tangent: vec3<f32>, tangent_w: f32) -> mat3x3<f32> {
    let t = safe_normalize(tangent - n * dot(n, tangent));
    return mat3x3<f32>(t, cross(n, t) * tangent_w, n);
}

// tangent_w == 0 marks geometry without authored tangents. Both frames are
// always built and picked with select so the derivatives stay in uniform control flow.
fn tangent_frame(n: vec3<f32>, tangent: vec3<f32>, tangent_w: f32, p: vec3<f32>, uv: vec2<f32>) -> mat3x3<f32> {
    let derived = cotangent_frame(n, p, uv);
    let authored = authored_tangent_frame(n, tangent, tangent_w);
    let has_authored = abs(tangent_w) > AUTHORED_TANGENT_MIN_W;
    return mat3x3<f32>(
        select(derived[0], authored[0], has_authored),
        select(derived[1], authored[1], has_authored),
        n,
    );
}
