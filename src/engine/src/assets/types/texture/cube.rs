use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use super::{
    Caps2, D3D10ResourceDimension, MipGeometry, MiscFlag, ParsedHeader, TextureError,
    TextureLayout, demote_srgb, header_format_info, promote_srgb, read_header,
};

pub const CUBE_FACE_COUNT: usize = 6;

/// A fully decoded cube map: `faces[face][mip]` holds the stored bytes of that
/// mip, faces in +X, -X, +Y, -Y, +Z, -Z order.
#[derive(Debug)]
pub struct CubeImage {
    /// Non-sRGB variant; `create_cube_texture` promotes color formats.
    pub format: wgpu::TextureFormat,
    pub size: u32,
    pub mip_count: u32,
    pub block_bytes: u32,
    pub block_dim: u32,
    pub faces: [Vec<Vec<u8>>; CUBE_FACE_COUNT],
}

impl CubeImage {
    fn geometry(&self, level: u32) -> MipGeometry {
        MipGeometry::new(
            self.size,
            self.size,
            self.block_dim,
            self.block_bytes,
            level,
        )
    }
}

fn check_cube_shape(
    path: &str,
    header: &ddsfile::Header,
    header10: Option<&ddsfile::Header10>,
) -> Result<(), TextureError> {
    let not_cube = || TextureError::NotCube {
        path: path.to_owned(),
    };
    if let Some(h10) = header10 {
        if !h10.misc_flag.contains(MiscFlag::TEXTURECUBE)
            || h10.resource_dimension != D3D10ResourceDimension::Texture2D
        {
            return Err(not_cube());
        }
        if h10.array_size != 1 {
            return Err(TextureError::CubeArray {
                path: path.to_owned(),
                array_size: h10.array_size,
            });
        }
    } else {
        if !header.caps2.contains(Caps2::CUBEMAP) || header.caps2.contains(Caps2::VOLUME) {
            return Err(not_cube());
        }
        if !header.caps2.contains(Caps2::CUBEMAP_ALLFACES) {
            return Err(TextureError::IncompleteCube {
                path: path.to_owned(),
            });
        }
    }
    if header.width != header.height {
        return Err(TextureError::NonSquareCube {
            path: path.to_owned(),
            width: header.width,
            height: header.height,
        });
    }
    Ok(())
}

/// Parse only the header of a single cube map. Faces share `width == height`;
/// the face-major data starts at `data_offset`, each face holding `mip_count`
/// mips.
pub fn read_cube_layout(path: &str) -> Result<TextureLayout, TextureError> {
    let ParsedHeader {
        header,
        header10,
        data_offset,
        file_len,
    } = read_header(path)?;
    check_cube_shape(path, &header, header10.as_ref())?;

    let (format, block_bytes, block_dim) = header_format_info(&header, header10.as_ref())
        .ok_or_else(|| TextureError::UnsupportedFormat {
            path: path.to_owned(),
        })?;
    if header.width == 0 || !header.width.is_multiple_of(block_dim) {
        return Err(TextureError::BlockMisaligned {
            path: path.to_owned(),
            width: header.width,
            height: header.height,
        });
    }
    let layout = TextureLayout {
        format: demote_srgb(format),
        block_bytes,
        block_dim,
        width: header.width,
        height: header.height,
        mip_count: header.mip_map_count.unwrap_or(1).max(1),
        data_offset,
    };

    let face_bytes = layout.mip_offset(layout.mip_count) - layout.data_offset;
    let expected = face_bytes * CUBE_FACE_COUNT as u64;
    let actual = file_len.saturating_sub(data_offset);
    if actual < expected {
        return Err(TextureError::Truncated {
            path: path.to_owned(),
            expected,
            actual,
        });
    }
    Ok(layout)
}

/// Read every face and mip of a single cube map.
pub fn load_cube(path: &str) -> Result<CubeImage, TextureError> {
    let layout = read_cube_layout(path)?;
    let mut file = File::open(path).map_err(|e| TextureError::io(path, e))?;
    file.seek(SeekFrom::Start(layout.data_offset))
        .map_err(|e| TextureError::io(path, e))?;

    let mut faces: [Vec<Vec<u8>>; CUBE_FACE_COUNT] = Default::default();
    for face in &mut faces {
        for level in 0..layout.mip_count {
            let mut bytes = vec![0u8; layout.mip_bytes(level) as usize];
            file.read_exact(&mut bytes)
                .map_err(|e| TextureError::io(path, e))?;
            face.push(bytes);
        }
    }
    Ok(CubeImage {
        format: layout.format,
        size: layout.width,
        mip_count: layout.mip_count,
        block_bytes: layout.block_bytes,
        block_dim: layout.block_dim,
        faces,
    })
}

/// Upload `image` as a 6-layer 2D texture holding the whole mip chain. Color
/// formats are promoted to their sRGB variant, matching diffuse textures; view
/// it with `TextureViewDimension::Cube`.
pub fn create_cube_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &CubeImage,
    label: &str,
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: image.size,
            height: image.size,
            depth_or_array_layers: CUBE_FACE_COUNT as u32,
        },
        mip_level_count: image.mip_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: promote_srgb(image.format),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (face, mips) in image.faces.iter().enumerate() {
        for (level, bytes) in mips.iter().enumerate() {
            let level = level as u32;
            let geometry = image.geometry(level);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: face as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(geometry.bytes_per_row),
                    rows_per_image: None,
                },
                geometry.copy_extent(),
            );
        }
    }
    texture
}
