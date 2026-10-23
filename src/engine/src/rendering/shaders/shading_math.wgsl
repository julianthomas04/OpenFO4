// Powers with a defined result at zero: a non-positive base gives 0 for every exponent.
fn pow_or_zero(base: f32, exponent: f32) -> f32 {
    if (base <= 0.0) {
        return 0.0;
    }
    return pow(base, exponent);
}

fn pow_or_zero3(base: vec3<f32>, exponent: f32) -> vec3<f32> {
    return vec3<f32>(
        pow_or_zero(base.x, exponent),
        pow_or_zero(base.y, exponent),
        pow_or_zero(base.z, exponent),
    );
}

// x^5 from multiplications only, for the per-light hot path.
fn pow5(x: f32) -> f32 {
    let x_squared = x * x;
    return x_squared * x_squared * x;
}
