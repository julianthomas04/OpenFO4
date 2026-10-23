//! Synthetic cube-map DDS files for the loader and cube-array tests.

use std::path::{Path, PathBuf};

pub const FACE_COLORS: [[u8; 3]; 6] = [
    [255, 0, 0],
    [0, 255, 255],
    [0, 255, 0],
    [255, 0, 255],
    [0, 0, 255],
    [255, 255, 0],
];

const DXGI_R8G8B8A8_UNORM: u32 = 28;
const DXGI_B8G8R8A8_UNORM: u32 = 87;
const DXGI_BC1_UNORM: u32 = 71;
const RESOURCE_DIMENSION_TEXTURE_2D: u32 = 3;
const MISC_TEXTURE_CUBE: u32 = 0x4;
const CAPS2_CUBEMAP: u32 = 0x200;
const CAPS2_ALL_FACES: u32 = 0xFC00;

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Rgba8,
    Bgra8,
    Bc1,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Header {
    Legacy,
    Dx10 { array_size: u32 },
}

#[derive(Clone, Copy)]
pub struct DdsSpec {
    pub width: u32,
    pub height: u32,
    pub mips: u32,
    pub format: Format,
    pub header: Header,
    pub cube: bool,
    pub face_flags: bool,
    pub drop_tail_bytes: usize,
}

impl DdsSpec {
    pub fn cube(size: u32, mips: u32, format: Format, header: Header) -> Self {
        Self {
            width: size,
            height: size,
            mips,
            format,
            header,
            cube: true,
            face_flags: true,
            drop_tail_bytes: 0,
        }
    }

    pub fn mip_bytes(&self, level: u32) -> usize {
        let w = (self.width >> level).max(1) as usize;
        let h = (self.height >> level).max(1) as usize;
        match self.format {
            Format::Rgba8 | Format::Bgra8 => w * h * 4,
            Format::Bc1 => w.div_ceil(4) * h.div_ceil(4) * 8,
        }
    }

    pub fn mip_pixels(&self, face: usize, level: u32) -> Vec<u8> {
        let [r, g, b] = FACE_COLORS[face];
        let bytes = self.mip_bytes(level);
        match self.format {
            Format::Rgba8 => [r, g, b, 255].repeat(bytes / 4),
            Format::Bgra8 => [b, g, r, 255].repeat(bytes / 4),
            Format::Bc1 => {
                let rgb565 =
                    (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3);
                let mut block = rgb565.to_le_bytes().to_vec();
                block.extend_from_slice(&[0; 6]);
                block.repeat(bytes / 8)
            }
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        let mut out = b"DDS ".to_vec();
        let dx10 = self.header != Header::Legacy;
        let mut words: Vec<u32> = vec![
            124,
            0x1 | 0x2 | 0x4 | 0x1000 | 0x20000,
            self.height,
            self.width,
            0,
            0,
            self.mips,
        ];
        words.extend([0; 11]);
        words.push(32);
        match (self.format, dx10) {
            (_, true) => words.extend([0x4, u32::from_le_bytes(*b"DX10"), 0, 0, 0, 0, 0]),
            (Format::Bc1, false) => {
                words.extend([0x4, u32::from_le_bytes(*b"DXT1"), 0, 0, 0, 0, 0]);
            }
            (Format::Rgba8, false) => words.extend([
                0x41,
                0,
                32,
                0x0000_00FF,
                0x0000_FF00,
                0x00FF_0000,
                0xFF00_0000,
            ]),
            (Format::Bgra8, false) => words.extend([
                0x41,
                0,
                32,
                0x00FF_0000,
                0x0000_FF00,
                0x0000_00FF,
                0xFF00_0000,
            ]),
        }
        words.push(0x1000 | 0x8 | 0x40_0000);
        words.push(match (self.cube, dx10, self.face_flags) {
            (true, false, true) => CAPS2_CUBEMAP | CAPS2_ALL_FACES,
            (true, false, false) => CAPS2_CUBEMAP,
            _ => 0,
        });
        words.extend([0, 0, 0]);
        if let Header::Dx10 { array_size } = self.header {
            words.extend([
                match self.format {
                    Format::Rgba8 => DXGI_R8G8B8A8_UNORM,
                    Format::Bgra8 => DXGI_B8G8R8A8_UNORM,
                    Format::Bc1 => DXGI_BC1_UNORM,
                },
                RESOURCE_DIMENSION_TEXTURE_2D,
                if self.cube { MISC_TEXTURE_CUBE } else { 0 },
                array_size,
                0,
            ]);
        }
        for word in words {
            out.extend_from_slice(&word.to_le_bytes());
        }
        let faces = if self.cube { 6 } else { 1 };
        for face in 0..faces {
            for level in 0..self.mips {
                out.extend(self.mip_pixels(face, level));
            }
        }
        out.truncate(out.len() - self.drop_tail_bytes);
        out
    }

    pub fn write(&self, directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, self.into_bytes()).expect("test DDS is writable");
        path
    }
}
