use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::sync::Arc;

use ddsfile::{Caps2, D3D10ResourceDimension, DxgiFormat, FourCC, Header, Header10, MiscFlag};

mod cube;
#[cfg(test)]
pub(crate) mod cube_test_support;
#[cfg(test)]
mod cube_tests;

pub use cube::{CubeImage, create_cube_texture, load_cube, read_cube_layout};

fn header_format_info(
    header: &Header,
    header10: Option<&Header10>,
) -> Option<(wgpu::TextureFormat, u32, u32)> {
    if let Some(h10) = header10 {
        return match h10.dxgi_format {
            DxgiFormat::BC1_UNorm => Some((wgpu::TextureFormat::Bc1RgbaUnorm, 8, 4)),
            DxgiFormat::BC1_UNorm_sRGB => Some((wgpu::TextureFormat::Bc1RgbaUnormSrgb, 8, 4)),
            DxgiFormat::BC2_UNorm => Some((wgpu::TextureFormat::Bc2RgbaUnorm, 16, 4)),
            DxgiFormat::BC2_UNorm_sRGB => Some((wgpu::TextureFormat::Bc2RgbaUnormSrgb, 16, 4)),
            DxgiFormat::BC3_UNorm => Some((wgpu::TextureFormat::Bc3RgbaUnorm, 16, 4)),
            DxgiFormat::BC3_UNorm_sRGB => Some((wgpu::TextureFormat::Bc3RgbaUnormSrgb, 16, 4)),
            DxgiFormat::BC4_UNorm => Some((wgpu::TextureFormat::Bc4RUnorm, 8, 4)),
            DxgiFormat::BC4_SNorm => Some((wgpu::TextureFormat::Bc4RSnorm, 8, 4)),
            DxgiFormat::BC5_UNorm => Some((wgpu::TextureFormat::Bc5RgUnorm, 16, 4)),
            DxgiFormat::BC5_SNorm => Some((wgpu::TextureFormat::Bc5RgSnorm, 16, 4)),
            DxgiFormat::BC7_UNorm => Some((wgpu::TextureFormat::Bc7RgbaUnorm, 16, 4)),
            DxgiFormat::BC7_UNorm_sRGB => Some((wgpu::TextureFormat::Bc7RgbaUnormSrgb, 16, 4)),
            DxgiFormat::R8G8B8A8_UNorm => Some((wgpu::TextureFormat::Rgba8Unorm, 4, 1)),
            DxgiFormat::R8G8B8A8_UNorm_sRGB => Some((wgpu::TextureFormat::Rgba8UnormSrgb, 4, 1)),
            DxgiFormat::B8G8R8A8_UNorm => Some((wgpu::TextureFormat::Bgra8Unorm, 4, 1)),
            DxgiFormat::B8G8R8A8_UNorm_sRGB => Some((wgpu::TextureFormat::Bgra8UnormSrgb, 4, 1)),
            _ => None,
        };
    }

    if let Some(fourcc) = &header.spf.fourcc {
        return match fourcc.0 {
            0x3154_5844 => Some((wgpu::TextureFormat::Bc1RgbaUnorm, 8, 4)), // DXT1
            0x3354_5844 => Some((wgpu::TextureFormat::Bc2RgbaUnorm, 16, 4)), // DXT3
            0x3554_5844 => Some((wgpu::TextureFormat::Bc3RgbaUnorm, 16, 4)), // DXT5
            0x3149_5441 => Some((wgpu::TextureFormat::Bc4RUnorm, 8, 4)),    // ATI1
            0x3249_5441 => Some((wgpu::TextureFormat::Bc5RgUnorm, 16, 4)), // ATI2 (legacy normal maps)
            _ => None,
        };
    }

    // Uncompressed 32-bit legacy DDS, identified by channel masks. The
    // installer's greyscale-to-palette bake (palette_baker.cs) writes BGRA;
    // vanilla LUT/gradient textures use the same layout.
    let spf = &header.spf;
    if spf.rgb_bit_count == Some(32) {
        let (r, g, b) = (spf.r_bit_mask, spf.g_bit_mask, spf.b_bit_mask);
        if r == Some(0x00FF_0000) && g == Some(0x0000_FF00) && b == Some(0x0000_00FF) {
            return Some((wgpu::TextureFormat::Bgra8Unorm, 4, 1));
        }
        if r == Some(0x0000_00FF) && g == Some(0x0000_FF00) && b == Some(0x00FF_0000) {
            return Some((wgpu::TextureFormat::Rgba8Unorm, 4, 1));
        }
    }

    None
}

/// FO4 diffuse DDS files are sRGB-encoded but tagged plain UNORM (the game
/// applies sRGB at the view level). Sampling them as linear on an sRGB
/// swapchain double-encodes gamma — promote color textures to the sRGB
/// variant. BC4/BC5 (data textures) have no sRGB variants and pass through.
fn promote_srgb(format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    match format {
        wgpu::TextureFormat::Bc1RgbaUnorm => wgpu::TextureFormat::Bc1RgbaUnormSrgb,
        wgpu::TextureFormat::Bc2RgbaUnorm => wgpu::TextureFormat::Bc2RgbaUnormSrgb,
        wgpu::TextureFormat::Bc3RgbaUnorm => wgpu::TextureFormat::Bc3RgbaUnormSrgb,
        wgpu::TextureFormat::Bc7RgbaUnorm => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8UnormSrgb,
        other => other,
    }
}

const DDS_MAGIC: &[u8; 4] = b"DDS ";
const DDS_PREAMBLE_BYTES: u64 = 4 + 124;
const DDS_DX10_HEADER_BYTES: u64 = 20;

#[derive(Debug, thiserror::Error)]
pub enum TextureError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid DDS {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: ddsfile::Error,
    },
    #[error("{path} is not a DDS file")]
    BadMagic { path: String },
    #[error("unsupported DDS format in {path}")]
    UnsupportedFormat { path: String },
    #[error(
        "{path} has {layers} array layers/faces and {depth} depth slices; only single 2D surfaces are supported"
    )]
    NotSingleSurface {
        path: String,
        layers: u32,
        depth: u32,
    },
    #[error("{path} is not a cube map; load it through the 2D texture path")]
    NotCube { path: String },
    #[error("cube map {path} lacks some of its six faces")]
    IncompleteCube { path: String },
    #[error("cube map {path} has non-square faces ({width}x{height})")]
    NonSquareCube {
        path: String,
        width: u32,
        height: u32,
    },
    #[error("{path} is an array of {array_size} cube maps; only a single cube map is supported")]
    CubeArray { path: String, array_size: u32 },
    #[error(
        "compressed texture {path} has a {width}x{height} base level that is not block-aligned"
    )]
    BlockMisaligned {
        path: String,
        width: u32,
        height: u32,
    },
    #[error("{path} is truncated: mips need {expected} bytes of data, file holds {actual}")]
    Truncated {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("mip range {first}..{end} is invalid for a texture with {mip_count} mips")]
    MipRange {
        first: u32,
        end: u32,
        mip_count: u32,
    },
    #[error(
        "payload mips {payload_first}..{payload_end} do not fit a texture holding mips from {texture_first} ({texture_mips} levels)"
    )]
    PayloadOutsideTexture {
        payload_first: u32,
        payload_end: u32,
        texture_first: u32,
        texture_mips: u32,
    },
}

impl TextureError {
    fn io(path: &str, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_owned(),
            source,
        }
    }

    fn parse(path: &str, source: ddsfile::Error) -> Self {
        Self::Parse {
            path: path.to_owned(),
            source,
        }
    }
}

/// Physical size of one mip level. Extents round up to the block dimension, so
/// sub-4x4 BC mips occupy a whole block.
struct MipGeometry {
    blocks_wide: u32,
    blocks_high: u32,
    block_dim: u32,
    bytes_per_row: u32,
    bytes: u64,
}

impl MipGeometry {
    fn new(width: u32, height: u32, block_dim: u32, block_bytes: u32, level: u32) -> Self {
        let blocks_wide = (width >> level).max(1).div_ceil(block_dim);
        let blocks_high = (height >> level).max(1).div_ceil(block_dim);
        let bytes_per_row = blocks_wide * block_bytes;
        Self {
            blocks_wide,
            blocks_high,
            block_dim,
            bytes_per_row,
            bytes: u64::from(bytes_per_row) * u64::from(blocks_high),
        }
    }

    fn copy_extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.blocks_wide * self.block_dim,
            height: self.blocks_high * self.block_dim,
            depth_or_array_layers: 1,
        }
    }
}

fn write_mip(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    geometry: &MipGeometry,
    bytes: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d::ZERO,
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

fn demote_srgb(format: wgpu::TextureFormat) -> wgpu::TextureFormat {
    match format {
        wgpu::TextureFormat::Bc1RgbaUnormSrgb => wgpu::TextureFormat::Bc1RgbaUnorm,
        wgpu::TextureFormat::Bc2RgbaUnormSrgb => wgpu::TextureFormat::Bc2RgbaUnorm,
        wgpu::TextureFormat::Bc3RgbaUnormSrgb => wgpu::TextureFormat::Bc3RgbaUnorm,
        wgpu::TextureFormat::Bc7RgbaUnormSrgb => wgpu::TextureFormat::Bc7RgbaUnorm,
        wgpu::TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8Unorm,
        other => other,
    }
}

/// Header-derived description of a single-surface 2D DDS: enough to locate any
/// mip range in the file without reading pixel data.
#[derive(Debug, Clone)]
pub struct TextureLayout {
    /// Non-sRGB variant; `create_texture` promotes on request.
    pub format: wgpu::TextureFormat,
    pub block_bytes: u32,
    pub block_dim: u32,
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    /// Absolute file offset of the first byte of mip 0.
    pub data_offset: u64,
}

impl TextureLayout {
    /// Logical (unrounded) extent of `level`.
    pub fn mip_extent(&self, level: u32) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }

    /// Stored bytes of `level`, with block-rounded extents.
    pub fn mip_bytes(&self, level: u32) -> u64 {
        self.geometry(level).bytes
    }

    /// Absolute file offset of `level`; `mip_offset(mip_count)` is the end of the mip data.
    pub fn mip_offset(&self, level: u32) -> u64 {
        self.data_offset + (0..level).map(|l| self.mip_bytes(l)).sum::<u64>()
    }

    fn geometry(&self, level: u32) -> MipGeometry {
        MipGeometry::new(
            self.width,
            self.height,
            self.block_dim,
            self.block_bytes,
            level,
        )
    }

    /// Extent for copying `level` between textures: the block-rounded
    /// physical size, which is what wgpu requires for compressed formats.
    pub fn mip_copy_extent(&self, level: u32) -> wgpu::Extent3d {
        self.geometry(level).copy_extent()
    }

    fn check_range(&self, range: &Range<u32>) -> Result<(), TextureError> {
        if range.start >= range.end || range.end > self.mip_count {
            return Err(TextureError::MipRange {
                first: range.start,
                end: range.end,
                mip_count: self.mip_count,
            });
        }
        Ok(())
    }
}

/// A contiguous run of mips `first_mip..mip_end` as stored in the file.
pub struct TexturePayload {
    pub layout: Arc<TextureLayout>,
    pub first_mip: u32,
    pub mip_end: u32,
    pub data: Vec<u8>,
}

struct ParsedHeader {
    header: Header,
    header10: Option<Header10>,
    data_offset: u64,
    file_len: u64,
}

fn read_header(path: &str) -> Result<ParsedHeader, TextureError> {
    let mut file = File::open(path).map_err(|e| TextureError::io(path, e))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|e| TextureError::io(path, e))?;
    if &magic != DDS_MAGIC {
        return Err(TextureError::BadMagic {
            path: path.to_owned(),
        });
    }
    let header = Header::read(&mut file).map_err(|e| TextureError::parse(path, e))?;

    let mut data_offset = DDS_PREAMBLE_BYTES;
    let header10 = if header.spf.fourcc == Some(FourCC(FourCC::DX10)) {
        data_offset += DDS_DX10_HEADER_BYTES;
        Some(Header10::read(&mut file).map_err(|e| TextureError::parse(path, e))?)
    } else {
        None
    };
    let file_len = file
        .metadata()
        .map_err(|e| TextureError::io(path, e))?
        .len();
    Ok(ParsedHeader {
        header,
        header10,
        data_offset,
        file_len,
    })
}

/// Parse only the DDS (and DX10) header. Never reads pixel data.
pub fn read_layout(path: &str) -> Result<TextureLayout, TextureError> {
    let ParsedHeader {
        header,
        header10,
        data_offset,
        file_len,
    } = read_header(path)?;

    let (layers, depth) = if let Some(h10) = &header10 {
        let faces = if h10.misc_flag.contains(MiscFlag::TEXTURECUBE) {
            6
        } else {
            1
        };
        let depth = if h10.resource_dimension == D3D10ResourceDimension::Texture3D {
            header.depth.unwrap_or(1).max(1)
        } else {
            1
        };
        (h10.array_size.max(1) * faces, depth)
    } else {
        let faces = if header.caps2.contains(Caps2::CUBEMAP) {
            6
        } else {
            1
        };
        let depth = if header.caps2.contains(Caps2::VOLUME) {
            header.depth.unwrap_or(1).max(1)
        } else {
            1
        };
        (faces, depth)
    };
    if layers != 1 || depth != 1 {
        return Err(TextureError::NotSingleSurface {
            path: path.to_owned(),
            layers,
            depth,
        });
    }

    let (format, block_bytes, block_dim) = header_format_info(&header, header10.as_ref())
        .ok_or_else(|| TextureError::UnsupportedFormat {
            path: path.to_owned(),
        })?;
    let mut layout = TextureLayout {
        format: demote_srgb(format),
        block_bytes,
        block_dim,
        width: header.width,
        height: header.height,
        mip_count: header.mip_map_count.unwrap_or(1).max(1),
        data_offset,
    };

    let held_mips = (1..=layout.mip_count)
        .take_while(|&count| layout.mip_offset(count) <= file_len)
        .last()
        .ok_or_else(|| TextureError::Truncated {
            path: path.to_owned(),
            expected: layout.mip_offset(1) - data_offset,
            actual: file_len.saturating_sub(data_offset),
        })?;
    if held_mips < layout.mip_count {
        crate::utils::warn_once(&format!("texture-truncated:{path}"), || {
            format!(
                "texture '{path}' header declares {} mips but the file holds {held_mips}; using those",
                layout.mip_count
            )
        });
        layout.mip_count = held_mips;
    }
    Ok(layout)
}

/// Read mips `range` with one seek and one contiguous read (DDS stores mips
/// largest-first, so any mip range is contiguous).
pub fn read_mips(
    path: &str,
    layout: &Arc<TextureLayout>,
    range: Range<u32>,
) -> Result<TexturePayload, TextureError> {
    layout.check_range(&range)?;
    let start = layout.mip_offset(range.start);
    let len = layout.mip_offset(range.end) - start;

    let mut file = File::open(path).map_err(|e| TextureError::io(path, e))?;
    file.seek(SeekFrom::Start(start))
        .map_err(|e| TextureError::io(path, e))?;
    let mut data = vec![0u8; len as usize];
    file.read_exact(&mut data)
        .map_err(|e| TextureError::io(path, e))?;

    Ok(TexturePayload {
        layout: Arc::clone(layout),
        first_mip: range.start,
        mip_end: range.end,
        data,
    })
}

/// Allocate a texture holding mips `first_mip..mip_count`; texture level 0 is
/// file mip `first_mip`. `first_mip` must leave a base edge that is a multiple
/// of the block size (the streaming policy keeps it >= 64).
pub fn create_texture(
    device: &wgpu::Device,
    layout: &TextureLayout,
    first_mip: u32,
    srgb: bool,
    label: &str,
) -> wgpu::Texture {
    let (width, height) = layout.mip_extent(first_mip);
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: layout.mip_count - first_mip,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if srgb {
            promote_srgb(layout.format)
        } else {
            layout.format
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Write the payload's mips into `texture`, whose level 0 is file mip
/// `texture_first_mip`.
pub fn write_payload(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    texture_first_mip: u32,
    payload: &TexturePayload,
) -> Result<(), TextureError> {
    let texture_mips = texture.mip_level_count();
    if payload.first_mip < texture_first_mip || payload.mip_end > texture_first_mip + texture_mips {
        return Err(TextureError::PayloadOutsideTexture {
            payload_first: payload.first_mip,
            payload_end: payload.mip_end,
            texture_first: texture_first_mip,
            texture_mips,
        });
    }

    let mut offset = 0usize;
    for mip in payload.first_mip..payload.mip_end {
        let geometry = payload.layout.geometry(mip);
        let end = offset + geometry.bytes as usize;
        write_mip(
            queue,
            texture,
            mip - texture_first_mip,
            &geometry,
            &payload.data[offset..end],
        );
        offset = end;
    }
    Ok(())
}
