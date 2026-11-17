//! GLB containers for test fixtures, laid out per the glTF 2.0 spec: a 12-byte header, a JSON
//! chunk and an optional BIN chunk, each padded to 4 bytes.

/// A clip of a test pack that moves one node linearly between two keys.
pub(crate) struct TranslationClip<'a> {
    pub name: &'a str,
    /// The clip's `extras` object, as JSON.
    pub extras: &'a str,
    pub node: usize,
    pub times: [f32; 2],
    pub keys: [[f32; 3]; 2],
}

/// A pack of `clips` over `nodes`, the JSON objects of its node list.
pub(crate) fn translation_pack_glb(nodes: &str, clips: &[TranslationClip<'_>]) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut animations = Vec::new();
    for (index, clip) in clips.iter().enumerate() {
        let (times, keys) = (2 * index, 2 * index + 1);
        let offset = bin.len();
        for value in clip
            .times
            .into_iter()
            .chain(clip.keys.into_iter().flatten())
        {
            bin.extend(value.to_le_bytes());
        }
        views.push(format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":8}},{{"buffer":0,"byteOffset":{},"byteLength":24}}"#,
            offset + 8
        ));
        accessors.push(format!(
            r#"{{"bufferView":{times},"componentType":5126,"count":2,"type":"SCALAR"}},{{"bufferView":{keys},"componentType":5126,"count":2,"type":"VEC3"}}"#
        ));
        animations.push(format!(
            r#"{{"name":"{}","extras":{},"channels":[{{"sampler":0,"target":{{"node":{},"path":"translation"}}}}],"samplers":[{{"input":{times},"output":{keys},"interpolation":"LINEAR"}}]}}"#,
            clip.name, clip.extras, clip.node
        ));
    }
    let json = format!(
        r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":{}}}],"bufferViews":[{}],"accessors":[{}],"nodes":[{nodes}],"animations":[{}]}}"#,
        bin.len(),
        views.join(","),
        accessors.join(","),
        animations.join(",")
    );
    glb_container(&json, &bin)
}

pub(crate) fn glb_container(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut json = json.as_bytes().to_vec();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut bin = bin.to_vec();
    bin.resize(bin.len().next_multiple_of(4), 0);
    let bin_chunk = if bin.is_empty() { 0 } else { 8 + bin.len() };
    let total = 12 + 8 + json.len() + bin_chunk;
    let mut out = Vec::with_capacity(total);
    out.extend(b"glTF");
    out.extend(2_u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(&json);
    if !bin.is_empty() {
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(&bin);
    }
    out
}
