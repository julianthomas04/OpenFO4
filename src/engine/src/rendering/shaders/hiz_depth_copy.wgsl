@group(0) @binding(0) var depth_tex: texture_depth_2d;
@group(0) @binding(1) var output_tex: texture_storage_2d<r32float, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = textureDimensions(depth_tex);
    if gid.x >= size.x || gid.y >= size.y {
        return;
    }
    let d = textureLoad(depth_tex, gid.xy, 0);
    textureStore(output_tex, gid.xy, vec4(d, 0.0, 0.0, 0.0));
}
