use std::collections::HashMap;

use crate::assets::db::landscapes::Landscape;
use glam::Vec3;

use crate::rendering::texture_streaming::uv_density;
use crate::rendering::vertex::Vertex;

pub const CELL_SIZE: f32 = 4096.0;
const SCALE: f32 = 0.1;
pub const TERRAIN_VERTS: usize = 33;
pub const TERRAIN_QUADS: usize = TERRAIN_VERTS - 1; // 32
pub const TERRAIN_QUAD_SIZE: f32 = CELL_SIZE / TERRAIN_QUADS as f32; // 128 beth units
const HEIGHT_SCALE: f32 = 8.0;
const SPLATMAP_RES: usize = 128;
pub const ALPHA_GRID: usize = 17;
const MAX_LAYERS: usize = 4;
/// The texture set vanilla shows wherever a quadrant has no base layer, and under overlay layers
/// that name no texture: `LandscapeCommonWealthDefault01`. Measured on vanilla's own views, whose
/// ground in such quadrants matches it far better than the quadrant's largest overlay does.
pub const DEFAULT_LAND_TEXTURE_SET: &str = "0AB07C:Fallout4.esm";
/// Cells per baked terrain-LOD tile edge (level-4 tiles: `<ws>.4.<x>.<y>.dds`).
pub const BAKED_TILE_CELLS: i32 = 4;
/// Times a splat layer texture repeats across a cell; mirrors `terrain.wgsl`.
pub const LAYER_TILING: f32 = 32.0;
/// Baked LOD tile DDS row 0 = north edge, but grid +y = north, so the cell's
/// v coordinate flips. Toggle if runtime verification shows mirrored tiles.
const BAKED_TILE_FLIP_V: bool = true;

pub struct TerrainCellCpu {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub splatmap_data: [u8; SPLATMAP_RES * SPLATMAP_RES * 4],
    pub texture_paths: [Option<String>; MAX_LAYERS],
    /// Asset-relative path of the baked level-4 LOD tile when this cell
    /// renders in baked mode; vertex UVs are already remapped to the tile
    /// sub-rect by `apply_baked_tile_uvs`.
    pub baked_tile: Option<String>,
    pub grid_x: i32,
    pub grid_y: i32,
    /// World units one UV unit spans with UVs running 0..1 over the cell
    /// (infinite when the mesh has no usable UVs).
    cell_world_per_uv: f32,
}

impl TerrainCellCpu {
    /// World units one UV unit of the cell's texture spans: a splat layer
    /// repeats `LAYER_TILING` times per cell, a baked tile spans
    /// `BAKED_TILE_CELLS` cells.
    pub fn world_per_uv(&self) -> f32 {
        if self.baked_tile.is_some() {
            self.cell_world_per_uv * BAKED_TILE_CELLS as f32
        } else {
            self.cell_world_per_uv / LAYER_TILING
        }
    }

    /// World-space bounds of the cell's mesh, heights included.
    pub fn world_bounds(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for vertex in &self.vertices {
            let position = Vec3::from(vertex.position);
            min = min.min(position);
            max = max.max(position);
        }
        (min, max)
    }
}

fn cell_world_per_uv(vertices: &[Vertex], indices: &[u32]) -> f32 {
    uv_density::world_per_uv(vertices, indices).unwrap_or(f32::INFINITY)
}

/// Grid coordinate of the level-4 baked tile containing this cell (tiles are
/// named by their south-west cell, a multiple of 4; works for negatives).
pub fn baked_tile_coord(grid: i32) -> i32 {
    grid.div_euclid(BAKED_TILE_CELLS) * BAKED_TILE_CELLS
}

/// Switch a cell to baked-tile texturing: remap vertex UVs from cell-space
/// (0..1) to the cell's sub-rect of the containing 4x4-cell tile.
pub fn apply_baked_tile_uvs(cpu: &mut TerrainCellCpu, tile_rel_path: String) {
    let lx = (cpu.grid_x - baked_tile_coord(cpu.grid_x)) as f32; // 0..3
    let ly = (cpu.grid_y - baked_tile_coord(cpu.grid_y)) as f32; // 0..3
    for v in &mut cpu.vertices {
        let u = (lx + v.tex_coords[0]) / BAKED_TILE_CELLS as f32;
        let w = (ly + v.tex_coords[1]) / BAKED_TILE_CELLS as f32;
        v.tex_coords = [u, if BAKED_TILE_FLIP_V { 1.0 - w } else { w }];
    }
    cpu.baked_tile = Some(tile_rel_path);
}

#[allow(
    clippy::implicit_hasher,
    reason = "public signature takes the engine's default-hasher map; generalising it would change the API"
)]
pub fn build_cell_cpu(
    record: &Landscape,
    ltex_to_diffuse: &HashMap<String, String>,
) -> anyhow::Result<TerrainCellCpu> {
    let heights = HeightGrid::decode(&record.vertex_height_map)?;
    let (vertices, indices) = generate_mesh(&heights, record.grid_x, record.grid_y, 1);
    let (splatmap_data, texture_paths) = build_splatmap(record, ltex_to_diffuse);

    Ok(TerrainCellCpu {
        cell_world_per_uv: cell_world_per_uv(&vertices, &indices),
        vertices,
        indices,
        splatmap_data,
        texture_paths,
        baked_tile: None,
        grid_x: record.grid_x,
        grid_y: record.grid_y,
    })
}

#[allow(
    clippy::implicit_hasher,
    reason = "public signature takes the engine's default-hasher map; generalising it would change the API"
)]
pub fn build_cell_cpu_lod(
    record: &Landscape,
    ltex_to_diffuse: &HashMap<String, String>,
) -> anyhow::Result<TerrainCellCpu> {
    let heights = HeightGrid::decode(&record.vertex_height_map)?;
    let (vertices, indices) = generate_mesh(&heights, record.grid_x, record.grid_y, 4);
    let (splatmap_data, texture_paths) = build_splatmap(record, ltex_to_diffuse);

    Ok(TerrainCellCpu {
        cell_world_per_uv: cell_world_per_uv(&vertices, &indices),
        vertices,
        indices,
        splatmap_data,
        texture_paths,
        baked_tile: None,
        grid_x: record.grid_x,
        grid_y: record.grid_y,
    })
}

/// The decoded `VHGT` heights of one cell, in Bethesda units.
pub struct HeightGrid {
    /// Indexed `[x][y]`.
    heights: [[f32; TERRAIN_VERTS]; TERRAIN_VERTS],
}

impl HeightGrid {
    /// `blob` is the `landscapes.vertex_height_map` column: an `f32` offset,
    /// then 33 x 33 `i8` deltas in rows of constant y.
    pub fn decode(blob: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            blob.len() >= 4 + TERRAIN_VERTS * TERRAIN_VERTS,
            "heightmap blob too short"
        );
        let offset = f32::from_le_bytes(blob[0..4].try_into()?);
        let deltas = &blob[4..];
        let mut heights = [[0.0f32; TERRAIN_VERTS]; TERRAIN_VERTS];
        let mut row_start_height = offset * HEIGHT_SCALE;

        for y in 0..TERRAIN_VERTS {
            let mut current_height = row_start_height;
            for x in 0..TERRAIN_VERTS {
                let raw = deltas[y * TERRAIN_VERTS + x] as i8;
                current_height += f32::from(raw) * HEIGHT_SCALE;
                heights[x][y] = current_height;
                if x == 0 {
                    row_start_height = current_height;
                }
            }
        }
        Ok(Self { heights })
    }

    pub fn height(&self, x: usize, y: usize) -> f32 {
        self.heights[x][y]
    }

    /// Height gradient `(dh/dx, dh/dy)` at a vertex from the neighbours
    /// `stride` vertices away (one-sided at the cell edge).
    fn slopes(&self, x: usize, y: usize, stride: usize) -> (f32, f32) {
        let h = self.heights[x][y];
        let step = TERRAIN_QUAD_SIZE * stride as f32 * SCALE;
        let has_x0 = x >= stride;
        let has_x1 = x + stride < TERRAIN_VERTS;
        let has_y0 = y >= stride;
        let has_y1 = y + stride < TERRAIN_VERTS;
        let hx0 = if has_x0 {
            self.heights[x - stride][y]
        } else {
            h
        };
        let hx1 = if has_x1 {
            self.heights[x + stride][y]
        } else {
            h
        };
        let hy0 = if has_y0 {
            self.heights[x][y - stride]
        } else {
            h
        };
        let hy1 = if has_y1 {
            self.heights[x][y + stride]
        } else {
            h
        };
        let dx = if has_x0 && has_x1 { 2.0 * step } else { step };
        let dy = if has_y0 && has_y1 { 2.0 * step } else { step };
        ((hx1 - hx0) * SCALE / dx, (hy1 - hy0) * SCALE / dy)
    }

    /// The smoothed vertex normal in engine space (Y up), as the rendered
    /// mesh carries it.
    pub fn normal(&self, x: usize, y: usize, stride: usize) -> [f32; 3] {
        let (dh_dx, dh_dy) = self.slopes(x, y, stride);
        let (nx, ny, nz) = (-dh_dx, 1.0f32, dh_dy);
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        [nx / len, ny / len, nz / len]
    }
}

/// Terrain mesh sampled every `stride` vertices (1 = full detail).
fn generate_mesh(
    heights: &HeightGrid,
    grid_x: i32,
    grid_y: i32,
    stride: usize,
) -> (Vec<Vertex>, Vec<u32>) {
    let cell_origin_x = grid_x as f32 * CELL_SIZE;
    let cell_origin_y = grid_y as f32 * CELL_SIZE;

    let sampled: Vec<usize> = (0..TERRAIN_VERTS).step_by(stride).collect();
    let lod_verts = sampled.len();

    let mut vertices = Vec::with_capacity(lod_verts * lod_verts);
    for &sx in &sampled {
        for &sy in &sampled {
            let bx = cell_origin_x + sx as f32 * TERRAIN_QUAD_SIZE;
            let by = cell_origin_y + sy as f32 * TERRAIN_QUAD_SIZE;
            let h = heights.height(sx, sy);
            let (dh_dx, _) = heights.slopes(sx, sy, stride);

            let tx = 1.0f32;
            let ty = dh_dx;
            let tlen = (tx * tx + ty * ty).sqrt();
            let tangent = [tx / tlen, ty / tlen, 0.0, 1.0];

            vertices.push(Vertex {
                position: [bx * SCALE, h * SCALE, -by * SCALE],
                tex_coords: [
                    sx as f32 / TERRAIN_QUADS as f32,
                    sy as f32 / TERRAIN_QUADS as f32,
                ],
                normal: heights.normal(sx, sy, stride),
                tangent,
            });
        }
    }

    let lod_quads = lod_verts - 1;
    let mut indices = Vec::with_capacity(lod_quads * lod_quads * 6);
    for x in 0..lod_quads {
        for y in 0..lod_quads {
            let i00 = (x * lod_verts + y) as u32;
            let i10 = ((x + 1) * lod_verts + y) as u32;
            let i01 = (x * lod_verts + y + 1) as u32;
            let i11 = ((x + 1) * lod_verts + y + 1) as u32;

            indices.push(i00);
            indices.push(i10);
            indices.push(i01);

            indices.push(i10);
            indices.push(i11);
            indices.push(i01);
        }
    }

    (vertices, indices)
}

/// Per-quadrant opacity grid: 17 x 17 vertices, indexed `[row][col]`.
pub type AlphaGrid = [[f32; ALPHA_GRID]; ALPHA_GRID];

/// One layer record of the `landscapes.layers` blob.
#[derive(Clone, Debug)]
pub struct LandscapeLayer {
    /// LTEX form key; `None` when the record carries none.
    pub texture_key: Option<String>,
    /// 0 = SW, 1 = SE, 2 = NW, 3 = NE.
    pub quadrant: u8,
    pub is_base: bool,
    pub alpha_grid: Option<AlphaGrid>,
    /// Summed opacity over the quadrant, 0 without an alpha grid.
    pub coverage: f32,
}

/// Decodes the whole `landscapes.layers` blob, in record order. `None` when
/// the blob is absent or too short to hold the layer count; a truncated tail
/// ends the list early.
pub fn decode_layers(layers_blob: &[u8]) -> Option<Vec<LandscapeLayer>> {
    if layers_blob.len() < 2 {
        return None;
    }
    let mut decoded: Vec<LandscapeLayer> = Vec::new();
    let layer_count = u16::from_le_bytes([layers_blob[0], layers_blob[1]]) as usize;
    let mut pos = 2usize;

    for _ in 0..layer_count {
        if pos >= layers_blob.len() {
            break;
        }

        // NUL-terminated texture form_key
        let nul = layers_blob[pos..].iter().position(|&b| b == 0).unwrap_or(0);
        let texture_key = if nul > 0 {
            std::str::from_utf8(&layers_blob[pos..pos + nul])
                .ok()
                .map(std::string::ToString::to_string)
        } else {
            None
        };
        pos += nul + 1;

        if pos + 2 > layers_blob.len() {
            break;
        }
        let quadrant = layers_blob[pos];
        pos += 1;
        let flags = layers_blob[pos];
        pos += 1;
        let is_base = flags & 1 != 0;

        if pos + 2 > layers_blob.len() {
            break;
        }
        let alpha_len = u16::from_le_bytes([layers_blob[pos], layers_blob[pos + 1]]) as usize;
        pos += 2;

        let alpha_grid = if alpha_len > 0 && pos + alpha_len <= layers_blob.len() {
            let bytes = &layers_blob[pos..pos + alpha_len];
            let mut grid = [[0.0f32; ALPHA_GRID]; ALPHA_GRID];
            let entry_size = 8;
            let num_entries = bytes.len() / entry_size;
            for i in 0..num_entries {
                let off = i * entry_size;
                if off + entry_size > bytes.len() {
                    break;
                }
                let p = u16::from_le_bytes([bytes[off], bytes[off + 1]]) as usize;
                let opacity = f32::from_le_bytes([
                    bytes[off + 4],
                    bytes[off + 5],
                    bytes[off + 6],
                    bytes[off + 7],
                ]);
                let row = p / ALPHA_GRID;
                let col = p % ALPHA_GRID;
                if row < ALPHA_GRID && col < ALPHA_GRID {
                    grid[row][col] = opacity.clamp(0.0, 1.0);
                }
            }
            pos += alpha_len;
            Some(grid)
        } else {
            pos += alpha_len;
            None
        };

        let coverage = alpha_grid.as_ref().map_or(0.0, |g| {
            g.iter().flat_map(|row| row.iter()).copied().sum::<f32>()
        });

        decoded.push(LandscapeLayer {
            texture_key,
            quadrant,
            is_base,
            alpha_grid,
            coverage,
        });
    }
    Some(decoded)
}

/// The base layer of a quadrant: the last base record wins.
pub(crate) fn base_layer(layers: &[LandscapeLayer], quadrant: u8) -> Option<&LandscapeLayer> {
    layers
        .iter()
        .filter(|layer| layer.quadrant == quadrant)
        .rfind(|layer| layer.is_base)
}

/// The quadrant of a cell vertex (`x` east, `y` north, both `0..TERRAIN_VERTS`) with the vertex's
/// `(column, row)` inside that quadrant's alpha grid. The shared middle vertices belong to the
/// east and north quadrants.
fn quadrant_vertex(x: usize, y: usize) -> (u8, usize, usize) {
    const SPAN: usize = ALPHA_GRID - 1;
    let (east, north) = (x >= SPAN, y >= SPAN);
    let quadrant = u8::from(east) + 2 * u8::from(north);
    (
        quadrant,
        if east { x - SPAN } else { x },
        if north { y - SPAN } else { y },
    )
}

/// The texture an overlay layer paints: its own, or the default land texture when it names none.
fn overlay_texture(layer: &LandscapeLayer) -> &str {
    layer
        .texture_key
        .as_deref()
        .unwrap_or(DEFAULT_LAND_TEXTURE_SET)
}

/// The layer with the highest blend weight at a cell vertex. An overlay weighs its opacity, the
/// base layer what the overlays leave (`1 - sum of overlay opacities`). Only layers of the
/// vertex's quadrant with a texture count. Ties go to the base layer, then to the earlier overlay
/// record. Without a base layer the highest overlay wins. `None` when the quadrant has no
/// textured layer.
pub(crate) fn dominant_layer_at(
    layers: &[LandscapeLayer],
    x: usize,
    y: usize,
) -> Option<&LandscapeLayer> {
    let (quadrant, col, row) = quadrant_vertex(x, y);
    let overlays: Vec<(&LandscapeLayer, f32)> = layers
        .iter()
        .filter(|layer| layer.quadrant == quadrant && !layer.is_base && layer.texture_key.is_some())
        .map(|layer| {
            let opacity = layer.alpha_grid.as_ref().map_or(0.0, |grid| grid[row][col]);
            (layer, opacity)
        })
        .collect();
    let base = base_layer(layers, quadrant).filter(|layer| layer.texture_key.is_some());
    let base_weight = (1.0 - overlays.iter().map(|(_, opacity)| opacity).sum::<f32>()).max(0.0);

    let mut candidates = base
        .map(|layer| (layer, base_weight))
        .into_iter()
        .chain(overlays);
    let first = candidates.next()?;
    let (winner, _) = candidates.fold(
        first,
        |best, next| if next.1 > best.1 { next } else { best },
    );
    Some(winner)
}

#[allow(
    clippy::large_stack_arrays,
    reason = "the splatmap pixel buffer is returned by value; boxing it would change the signature"
)]
fn build_splatmap(
    record: &Landscape,
    ltex_to_diffuse: &HashMap<String, String>,
) -> (
    [u8; SPLATMAP_RES * SPLATMAP_RES * 4],
    [Option<String>; MAX_LAYERS],
) {
    let mut pixel_data = [0u8; SPLATMAP_RES * SPLATMAP_RES * 4];
    let mut texture_paths: [Option<String>; MAX_LAYERS] = Default::default();

    let Some(decoded) = record.layers.as_deref().and_then(decode_layers) else {
        for i in 0..(SPLATMAP_RES * SPLATMAP_RES) {
            pixel_data[i * 4] = 255;
        }
        return (pixel_data, texture_paths);
    };

    let quadrant_base: [&str; 4] = std::array::from_fn(|q| {
        base_layer(&decoded, q as u8)
            .and_then(|dl| dl.texture_key.as_deref())
            .unwrap_or(DEFAULT_LAND_TEXTURE_SET)
    });

    let mut slot_map: HashMap<String, usize> = HashMap::new();
    let mut next_slot = 0usize;

    for key in quadrant_base {
        if !slot_map.contains_key(key) {
            slot_map.insert(key.to_string(), next_slot);
            next_slot += 1;
        }
    }

    let mut overlays: Vec<(&str, f32)> = Vec::new();
    for dl in decoded.iter().filter(|dl| !dl.is_base) {
        let key = overlay_texture(dl);
        if !slot_map.contains_key(key) {
            overlays.push((key, dl.coverage));
        }
    }
    overlays.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (key, _) in overlays {
        if next_slot >= MAX_LAYERS {
            break;
        }
        if !slot_map.contains_key(key) {
            slot_map.insert(key.to_string(), next_slot);
            next_slot += 1;
        }
    }

    for (key, &slot) in &slot_map {
        texture_paths[slot] = ltex_to_diffuse.get(key).cloned();
        if texture_paths[slot].is_none() {
            crate::utils::warn_once(key, || {
                format!("terrain splat layer LTEX {key} has no diffuse path; renders fallback")
            });
        }
    }

    let half = SPLATMAP_RES / 2;
    for py in 0..SPLATMAP_RES {
        for px in 0..SPLATMAP_RES {
            let q = match (px >= half, py >= half) {
                (false, false) => 0u8,
                (true, false) => 1,
                (false, true) => 2,
                (true, true) => 3,
            };

            let qx = if px >= half { px - half } else { px };
            let qy = if py >= half { py - half } else { py };

            let base_slot = slot_map[quadrant_base[q as usize]];

            let mut weights = [0.0f32; MAX_LAYERS];
            let mut total_alpha = 0.0f32;

            for dl in &decoded {
                if dl.quadrant != q || dl.is_base {
                    continue;
                }
                let Some(&slot) = slot_map.get(overlay_texture(dl)) else {
                    continue;
                };
                let Some(grid) = &dl.alpha_grid else {
                    continue;
                };

                let opacity = sample_alpha_bilinear(grid, qx, qy, half);
                weights[slot] += opacity;
                total_alpha += opacity;
            }

            weights[base_slot] += (1.0 - total_alpha).max(0.0);

            let idx = (py * SPLATMAP_RES + px) * 4;
            for ch in 0..MAX_LAYERS {
                pixel_data[idx + ch] = (weights[ch].clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }

    (pixel_data, texture_paths)
}

fn sample_alpha_bilinear(
    grid: &[[f32; ALPHA_GRID]; ALPHA_GRID],
    px: usize,
    py: usize,
    quadrant_size: usize,
) -> f32 {
    let fx = px as f32 / (quadrant_size - 1) as f32 * (ALPHA_GRID - 1) as f32;
    let fy = py as f32 / (quadrant_size - 1) as f32 * (ALPHA_GRID - 1) as f32;

    let x0 = (fx.floor() as usize).min(ALPHA_GRID - 1);
    let y0 = (fy.floor() as usize).min(ALPHA_GRID - 1);
    let x1 = (x0 + 1).min(ALPHA_GRID - 1);
    let y1 = (y0 + 1).min(ALPHA_GRID - 1);

    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;

    let c00 = grid[y0][x0];
    let c10 = grid[y0][x1];
    let c01 = grid[y1][x0];
    let c11 = grid[y1][x1];

    let top = c00 * (1.0 - tx) + c10 * tx;
    let bot = c01 * (1.0 - tx) + c11 * tx;
    top * (1.0 - ty) + bot * ty
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const HEIGHT_BLOB_LEN: usize = 4 + TERRAIN_VERTS * TERRAIN_VERTS;

    fn landscape(
        grid_x: i32,
        grid_y: i32,
        offset: f32,
        deltas: &[i8],
        layers: Option<Vec<u8>>,
    ) -> Landscape {
        let mut blob = offset.to_le_bytes().to_vec();
        blob.extend(deltas.iter().map(|d| *d as u8));
        blob.resize(HEIGHT_BLOB_LEN, 0);
        Landscape {
            form_key: None,
            editor_id: None,
            cell: "cell".into(),
            grid_x,
            grid_y,
            vertex_height_map: blob,
            vertex_colors: None,
            layers,
            flags: None,
        }
    }

    fn flat(grid_x: i32, grid_y: i32) -> TerrainCellCpu {
        build_cell_cpu(&landscape(grid_x, grid_y, 0.0, &[], None), &HashMap::new()).unwrap()
    }

    /// Vertices are stored x-major: index = x * 33 + y.
    fn vertex(cell: &TerrainCellCpu, x: usize, y: usize) -> &Vertex {
        &cell.vertices[x * TERRAIN_VERTS + y]
    }

    /// One layer record of the `landscapes.layers` blob: NUL-terminated key,
    /// quadrant, flags (bit 0 = base), u16 alpha byte length, then 8-byte
    /// entries (u16 grid position, 2 unused bytes, f32 opacity).
    fn layer_record(key: &str, quadrant: u8, is_base: bool, alpha: &[(u16, f32)]) -> Vec<u8> {
        let mut out = key.as_bytes().to_vec();
        out.push(0);
        out.push(quadrant);
        out.push(u8::from(is_base));
        out.extend(((alpha.len() * 8) as u16).to_le_bytes());
        for (position, opacity) in alpha {
            out.extend(position.to_le_bytes());
            out.extend([0, 0]);
            out.extend(opacity.to_le_bytes());
        }
        out
    }

    fn layers_blob(records: &[Vec<u8>]) -> Vec<u8> {
        let mut out = (records.len() as u16).to_le_bytes().to_vec();
        for record in records {
            out.extend(record);
        }
        out
    }

    fn full_alpha(opacity: f32) -> Vec<(u16, f32)> {
        (0..(ALPHA_GRID * ALPHA_GRID) as u16)
            .map(|p| (p, opacity))
            .collect()
    }

    fn textures() -> HashMap<String, String> {
        [
            ("grass", "textures/grass_d.dds"),
            ("rock", "textures/rock_d.dds"),
            (DEFAULT_LAND_TEXTURE_SET, "textures/default_d.dds"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    /// `key` as the base layer of all four quadrants.
    fn base_everywhere(key: &str) -> Vec<Vec<u8>> {
        (0..4).map(|q| layer_record(key, q, true, &[])).collect()
    }

    fn splat_pixel(cell: &TerrainCellCpu, px: usize, py: usize) -> [u8; 4] {
        let i = (py * SPLATMAP_RES + px) * 4;
        cell.splatmap_data[i..i + 4].try_into().unwrap()
    }

    fn layer(key: &str, quadrant: u8, is_base: bool, opacity: f32) -> LandscapeLayer {
        LandscapeLayer {
            texture_key: Some(key.to_string()),
            quadrant,
            is_base,
            alpha_grid: (!is_base).then_some([[opacity; ALPHA_GRID]; ALPHA_GRID]),
            coverage: 0.0,
        }
    }

    fn dominant_key(layers: &[LandscapeLayer], x: usize, y: usize) -> Option<&str> {
        dominant_layer_at(layers, x, y).and_then(|layer| layer.texture_key.as_deref())
    }

    #[test]
    fn dominant_layer_of_a_base_only_quadrant_is_the_base() {
        let layers = [layer("grass", 0, true, 0.0)];
        assert_eq!(dominant_key(&layers, 3, 3), Some("grass"));
    }

    #[test]
    fn dominant_layer_is_an_overlay_above_half_and_the_base_below() {
        let strong = [layer("grass", 0, true, 0.0), layer("rock", 0, false, 0.6)];
        assert_eq!(dominant_key(&strong, 3, 3), Some("rock"));
        let weak = [layer("grass", 0, true, 0.0), layer("rock", 0, false, 0.4)];
        assert_eq!(dominant_key(&weak, 3, 3), Some("grass"));
    }

    #[test]
    fn dominant_layer_base_weight_is_what_all_overlays_leave() {
        let layers = [
            layer("grass", 0, true, 0.0),
            layer("rock", 0, false, 0.3),
            layer("sand", 0, false, 0.3),
        ];
        assert_eq!(dominant_key(&layers, 3, 3), Some("grass"));
        let layers = [
            layer("grass", 0, true, 0.0),
            layer("rock", 0, false, 0.35),
            layer("sand", 0, false, 0.4),
        ];
        assert_eq!(dominant_key(&layers, 3, 3), Some("sand"));
    }

    #[test]
    fn dominant_layer_tie_goes_to_the_base_then_to_the_earlier_overlay() {
        let base_tie = [layer("grass", 0, true, 0.0), layer("rock", 0, false, 0.5)];
        assert_eq!(dominant_key(&base_tie, 3, 3), Some("grass"));
        let overlay_tie = [
            layer("grass", 0, true, 0.0),
            layer("rock", 0, false, 0.4),
            layer("sand", 0, false, 0.4),
        ];
        assert_eq!(dominant_key(&overlay_tie, 3, 3), Some("rock"));
    }

    #[test]
    fn dominant_layer_without_a_base_is_the_highest_overlay() {
        let layers = [layer("rock", 0, false, 0.1), layer("sand", 0, false, 0.2)];
        assert_eq!(dominant_key(&layers, 3, 3), Some("sand"));
    }

    #[test]
    fn dominant_layer_reads_the_quadrant_of_the_vertex() {
        let layers = [
            layer("sw", 0, true, 0.0),
            layer("se", 1, true, 0.0),
            layer("nw", 2, true, 0.0),
            layer("ne", 3, true, 0.0),
        ];
        assert_eq!(dominant_key(&layers, 15, 15), Some("sw"));
        assert_eq!(dominant_key(&layers, 16, 15), Some("se"));
        assert_eq!(dominant_key(&layers, 15, 16), Some("nw"));
        assert_eq!(dominant_key(&layers, 16, 16), Some("ne"));
        assert_eq!(dominant_key(&layers, 32, 32), Some("ne"));
    }

    #[test]
    fn dominant_layer_reads_the_opacity_at_the_vertex_inside_its_quadrant() {
        let mut rock = layer("rock", 3, false, 0.0);
        rock.alpha_grid.as_mut().unwrap()[2][5] = 1.0;
        let layers = [layer("grass", 3, true, 0.0), rock];
        assert_eq!(dominant_key(&layers, 16 + 5, 16 + 2), Some("rock"));
        assert_eq!(dominant_key(&layers, 16 + 2, 16 + 5), Some("grass"));
    }

    #[test]
    fn dominant_layer_of_a_quadrant_without_textured_layers_is_none() {
        let mut untextured = layer("x", 0, true, 0.0);
        untextured.texture_key = None;
        assert!(dominant_layer_at(&[untextured], 3, 3).is_none());
        assert!(dominant_layer_at(&[layer("grass", 1, true, 0.0)], 3, 3).is_none());
    }

    #[test]
    fn baked_tile_coord_rounds_down_to_a_multiple_of_four() {
        for (grid, tile) in [(0, 0), (3, 0), (4, 4), (7, 4), (-1, -4), (-4, -4), (-5, -8)] {
            assert_eq!(baked_tile_coord(grid), tile, "grid {grid}");
        }
    }

    #[test]
    fn short_height_blob_is_rejected() {
        let mut record = landscape(0, 0, 0.0, &[], None);
        record.vertex_height_map.truncate(HEIGHT_BLOB_LEN - 1);
        assert!(build_cell_cpu(&record, &HashMap::new()).is_err());
        assert!(build_cell_cpu_lod(&record, &HashMap::new()).is_err());
    }

    #[test]
    fn empty_height_blob_is_rejected_without_panicking() {
        let mut record = landscape(0, 0, 0.0, &[], None);
        record.vertex_height_map.clear();
        assert!(build_cell_cpu(&record, &HashMap::new()).is_err());
    }

    #[test]
    fn full_resolution_cell_has_33_by_33_vertices_and_two_triangles_per_quad() {
        let cell = flat(0, 0);
        assert_eq!(cell.vertices.len(), 1089);
        assert_eq!(cell.indices.len(), 32 * 32 * 6);
        assert!(
            cell.indices
                .iter()
                .all(|&i| (i as usize) < cell.vertices.len())
        );
    }

    #[test]
    fn lod_cell_samples_every_fourth_vertex() {
        let cell = build_cell_cpu_lod(&landscape(0, 0, 0.0, &[], None), &HashMap::new()).unwrap();
        assert_eq!(cell.vertices.len(), 9 * 9);
        assert_eq!(cell.indices.len(), 8 * 8 * 6);
        assert!(
            cell.indices
                .iter()
                .all(|&i| (i as usize) < cell.vertices.len())
        );
    }

    #[test]
    fn flat_cell_height_is_offset_times_eight_game_units_in_engine_scale() {
        // offset 2.0 -> 16 game units -> 1.6 engine units on every vertex.
        let cell = build_cell_cpu(&landscape(0, 0, 2.0, &[], None), &HashMap::new()).unwrap();
        assert!(
            cell.vertices
                .iter()
                .all(|v| (v.position[1] - 1.6).abs() < 1e-5)
        );
    }

    #[test]
    fn cell_vertices_span_one_cell_from_the_grid_origin() {
        // Cell (2, -1): origin (8192, -4096) game units -> engine (819.2, ., 409.6).
        let cell = flat(2, -1);
        let first = vertex(&cell, 0, 0);
        let last = vertex(&cell, 32, 32);
        assert!((first.position[0] - 819.2).abs() < 1e-3);
        assert!((first.position[2] - 409.6).abs() < 1e-3);
        assert!((last.position[0] - (819.2 + 409.6)).abs() < 1e-3);
        assert!((last.position[2] - (409.6 - 409.6)).abs() < 1e-3);
    }

    #[test]
    fn world_bounds_of_a_flat_cell_cover_one_cell_at_constant_height() {
        let (min, max) = flat(0, 0).world_bounds();
        assert!((max.x - min.x - 409.6).abs() < 1e-3);
        assert!((max.z - min.z - 409.6).abs() < 1e-3);
        assert_eq!(min.y, max.y);
    }

    #[test]
    fn deltas_accumulate_along_a_row_and_negative_bytes_descend() {
        // Row 0 (y = 0), x = 0..: +1, +1, -1 -> 8, 16, 8 game units above offset 0.
        let mut deltas = vec![0i8; 33 * 33];
        deltas[0] = 1;
        deltas[1] = 1;
        deltas[2] = -1;
        let record = landscape(0, 0, 0.0, &deltas, None);
        let cell = build_cell_cpu(&record, &HashMap::new()).unwrap();
        let heights: Vec<f32> = (0..3).map(|x| vertex(&cell, x, 0).position[1]).collect();
        assert!((heights[0] - 0.8).abs() < 1e-5);
        assert!((heights[1] - 1.6).abs() < 1e-5);
        assert!((heights[2] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn first_column_delta_carries_into_the_next_row() {
        // Delta layout is row-major (y * 33 + x). The first value of each row
        // is relative to the previous row's first value; the rest of the row
        // is relative to the vertex to its left.
        let mut deltas = vec![0i8; 33 * 33];
        deltas[0] = 1; // row 0 starts at 8
        deltas[33] = 2; // row 1 starts at 8 + 16 = 24
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &deltas, None), &HashMap::new()).unwrap();
        assert!((vertex(&cell, 0, 0).position[1] - 0.8).abs() < 1e-5);
        assert!((vertex(&cell, 32, 0).position[1] - 0.8).abs() < 1e-5);
        assert!((vertex(&cell, 0, 1).position[1] - 2.4).abs() < 1e-5);
        assert!((vertex(&cell, 32, 1).position[1] - 2.4).abs() < 1e-5);
    }

    #[test]
    fn flat_terrain_normals_point_up_and_are_unit_length() {
        let cell = flat(0, 0);
        for v in &cell.vertices {
            assert!((v.normal[1] - 1.0).abs() < 1e-6);
            assert_eq!(v.normal[0], 0.0);
            assert_eq!(v.normal[2], 0.0);
        }
    }

    #[test]
    fn slope_rising_towards_plus_x_tilts_the_normal_towards_minus_x() {
        // +1 delta on every vertex along x: 8 game units rise per 128-unit step.
        let mut deltas = vec![0i8; 33 * 33];
        for row in 0..33 {
            for x in 1..33 {
                deltas[row * 33 + x] = 1;
            }
        }
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &deltas, None), &HashMap::new()).unwrap();
        // Slope 8 / 128 = 0.0625 -> normal = (-0.0625, 1, 0) / sqrt(1.00390625).
        let normal = vertex(&cell, 16, 16).normal;
        assert!((normal[0] + 0.062_378).abs() < 1e-5, "{normal:?}");
        assert!((normal[1] - 0.998_053).abs() < 1e-5, "{normal:?}");
        assert!(normal[2].abs() < 1e-6, "{normal:?}");
    }

    #[test]
    fn texture_coordinates_run_zero_to_one_across_the_cell() {
        let cell = flat(0, 0);
        assert_eq!(vertex(&cell, 0, 0).tex_coords, [0.0, 0.0]);
        assert_eq!(vertex(&cell, 32, 32).tex_coords, [1.0, 1.0]);
        assert_eq!(vertex(&cell, 16, 0).tex_coords, [0.5, 0.0]);
    }

    #[test]
    fn baked_tile_uvs_map_a_cell_to_its_subrect_of_the_tile() {
        // Cell (5, -1) is local (1, 3) inside tile (4, -4). u = (1 + u) / 4,
        // v = 1 - (3 + v) / 4 after the north-up flip.
        let mut cell = flat(5, -1);
        apply_baked_tile_uvs(&mut cell, "tile.dds".into());
        assert_eq!(cell.baked_tile.as_deref(), Some("tile.dds"));
        let origin = vertex(&cell, 0, 0).tex_coords;
        let far = vertex(&cell, 32, 32).tex_coords;
        assert!((origin[0] - 0.25).abs() < 1e-6 && (origin[1] - 0.25).abs() < 1e-6);
        assert!((far[0] - 0.5).abs() < 1e-6 && (far[1] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn world_per_uv_spans_the_tile_when_baked_and_one_layer_repeat_otherwise() {
        let mut cell = flat(0, 0);
        let splat = cell.world_per_uv();
        apply_baked_tile_uvs(&mut cell, "tile.dds".into());
        let baked = cell.world_per_uv();
        assert!((baked / splat - 128.0).abs() < 1e-2, "{baked} / {splat}");
    }

    #[test]
    fn no_layers_blob_renders_the_first_slot_everywhere() {
        let cell = flat(0, 0);
        assert_eq!(splat_pixel(&cell, 0, 0), [255, 0, 0, 0]);
        assert_eq!(splat_pixel(&cell, 127, 127), [255, 0, 0, 0]);
        assert!(cell.texture_paths.iter().all(Option::is_none));
    }

    #[test]
    fn truncated_layers_blob_does_not_panic() {
        let full = layers_blob(&[
            layer_record("grass", 0, true, &[]),
            layer_record("rock", 0, false, &full_alpha(1.0)),
        ]);
        for len in 0..full.len() {
            let record = landscape(0, 0, 0.0, &[], Some(full[..len].to_vec()));
            assert!(build_cell_cpu(&record, &textures()).is_ok(), "len {len}");
        }
    }

    #[test]
    fn base_layer_fills_its_quadrant_and_maps_the_texture_path() {
        let layers = layers_blob(&[layer_record("grass", 0, true, &[])]);
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        assert_eq!(
            cell.texture_paths[0].as_deref(),
            Some("textures/grass_d.dds")
        );
        assert_eq!(splat_pixel(&cell, 10, 10), [255, 0, 0, 0]);
    }

    #[test]
    fn opaque_overlay_replaces_the_base_in_its_quadrant_only() {
        let layers = layers_blob(
            &[
                base_everywhere("grass"),
                vec![layer_record("rock", 0, false, &full_alpha(1.0))],
            ]
            .concat(),
        );
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        assert_eq!(
            cell.texture_paths[1].as_deref(),
            Some("textures/rock_d.dds")
        );
        assert_eq!(splat_pixel(&cell, 10, 10), [0, 255, 0, 0]);
        assert_eq!(splat_pixel(&cell, 100, 10), [255, 0, 0, 0]);
    }

    #[test]
    fn half_opaque_overlay_splits_the_weight_evenly() {
        let layers = layers_blob(
            &[
                base_everywhere("grass"),
                vec![layer_record("rock", 0, false, &full_alpha(0.5))],
            ]
            .concat(),
        );
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        let [grass, rock, _, _] = splat_pixel(&cell, 10, 10);
        assert_eq!((grass, rock), (127, 127));
    }

    #[test]
    fn overlay_opacity_is_clamped_to_unit_range() {
        let layers = layers_blob(
            &[
                base_everywhere("grass"),
                vec![layer_record("rock", 0, false, &full_alpha(7.0))],
            ]
            .concat(),
        );
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        assert_eq!(splat_pixel(&cell, 10, 10), [0, 255, 0, 0]);
    }

    #[test]
    fn quadrant_without_a_base_layer_shows_the_default_land_texture_under_its_overlays() {
        // 20% rock leaves 80% default: 0.8 * 255 = 204, 0.2 * 255 = 51.
        let layers = layers_blob(&[
            layer_record("grass", 0, false, &[(0, 0.1)]),
            layer_record("rock", 0, false, &full_alpha(0.2)),
        ]);
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        assert_eq!(
            cell.texture_paths[0].as_deref(),
            Some("textures/default_d.dds")
        );
        let [default, rock, grass, _] = splat_pixel(&cell, 20, 20);
        assert!(
            default.abs_diff(204) <= 1 && rock.abs_diff(51) <= 1 && grass == 0,
            "{default} {rock} {grass}"
        );
    }

    #[test]
    fn an_overlay_without_a_texture_paints_the_default_land_texture() {
        let layers = layers_blob(
            &[
                base_everywhere("grass"),
                vec![layer_record("", 0, false, &full_alpha(1.0))],
            ]
            .concat(),
        );
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        assert_eq!(
            cell.texture_paths[1].as_deref(),
            Some("textures/default_d.dds")
        );
        assert_eq!(splat_pixel(&cell, 10, 10), [0, 255, 0, 0]);
    }

    #[test]
    fn unmapped_layer_texture_leaves_its_slot_empty() {
        let layers = layers_blob(&[layer_record("mystery", 0, true, &[])]);
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        assert_eq!(cell.texture_paths[0], None);
    }

    #[test]
    fn at_most_four_distinct_textures_get_slots() {
        let keys = ["t0", "t1", "t2", "t3", "t4"];
        let mut records = base_everywhere("t0");
        for key in &keys[1..] {
            records.push(layer_record(key, 0, false, &full_alpha(0.1)));
        }
        let map: HashMap<String, String> = keys
            .iter()
            .map(|k| ((*k).to_string(), format!("{k}.dds")))
            .collect();
        let cell = build_cell_cpu(
            &landscape(0, 0, 0.0, &[], Some(layers_blob(&records))),
            &map,
        )
        .unwrap();
        assert_eq!(cell.texture_paths.iter().flatten().count(), 4);
    }

    #[test]
    fn base_receives_whatever_weight_the_overlay_leaves() {
        // 30% rock leaves 70% grass: 0.7 * 255 = 178.5, 0.3 * 255 = 76.5.
        let layers = layers_blob(
            &[
                base_everywhere("grass"),
                vec![layer_record("rock", 0, false, &full_alpha(0.3))],
            ]
            .concat(),
        );
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        let [grass, rock, unused_a, unused_b] = splat_pixel(&cell, 20, 20);
        assert!(
            grass.abs_diff(178) <= 1 && rock.abs_diff(76) <= 1,
            "{grass} {rock}"
        );
        assert_eq!((unused_a, unused_b), (0, 0));
    }

    #[test]
    fn each_quadrant_is_driven_by_its_own_layers() {
        // Quadrant 3 is the south-east (+x, +y) block: pixels with x, y >= 64.
        let layers = layers_blob(&[
            layer_record("grass", 3, true, &[]),
            layer_record("rock", 3, false, &full_alpha(1.0)),
        ]);
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        // The other quadrants have no layers: the default land texture, slot 0.
        assert_eq!(splat_pixel(&cell, 100, 100), [0, 0, 255, 0]);
        assert_eq!(splat_pixel(&cell, 10, 100), [255, 0, 0, 0]);
        assert_eq!(splat_pixel(&cell, 100, 10), [255, 0, 0, 0]);
    }

    #[test]
    fn alpha_entries_outside_the_grid_are_ignored() {
        let layers = layers_blob(&[
            layer_record("grass", 0, true, &[]),
            layer_record(
                "rock",
                0,
                false,
                &[(ALPHA_GRID as u16 * ALPHA_GRID as u16 + 5, 1.0)],
            ),
        ]);
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers)), &textures()).unwrap();
        assert_eq!(splat_pixel(&cell, 10, 10), [255, 0, 0, 0]);
    }

    #[test]
    fn lod_vertices_coincide_with_every_fourth_full_resolution_vertex() {
        let deltas: Vec<i8> = (0..33 * 33).map(|i| ((i * 7) % 11) as i8 - 5).collect();
        let record = landscape(1, -2, 3.0, &deltas, None);
        let full = build_cell_cpu(&record, &HashMap::new()).unwrap();
        let lod = build_cell_cpu_lod(&record, &HashMap::new()).unwrap();
        for i in 0..9 {
            for j in 0..9 {
                assert_eq!(
                    lod.vertices[i * 9 + j].position,
                    vertex(&full, i * 4, j * 4).position,
                    "({i}, {j})"
                );
            }
        }
    }

    #[test]
    fn world_bounds_follow_the_heights_of_a_sloped_cell() {
        let mut deltas = vec![0i8; 33 * 33];
        deltas[0] = 5; // whole cell starts 40 game units up
        deltas[1] = 10; // then rises 80 more along the first row
        let cell = build_cell_cpu(&landscape(0, 0, 0.0, &deltas, None), &HashMap::new()).unwrap();
        let (min, max) = cell.world_bounds();
        assert!(
            (min.y - 4.0).abs() < 1e-4 && (max.y - 12.0).abs() < 1e-4,
            "{min} {max}"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn arbitrary_layer_bytes_never_panic_and_never_break_the_mesh(
            layers in proptest::collection::vec(any::<u8>(), 0..400),
        ) {
            let record = landscape(0, 0, 0.0, &[], Some(layers));
            let cell = build_cell_cpu(&record, &textures()).unwrap();
            prop_assert_eq!(cell.vertices.len(), 1089);
            prop_assert_eq!(cell.indices.len(), 32 * 32 * 6);
        }

        #[test]
        fn arbitrary_height_blob_either_decodes_to_a_full_grid_or_errors(
            blob in proptest::collection::vec(any::<u8>(), 0..1200),
        ) {
            let mut record = landscape(0, 0, 0.0, &[], None);
            record.vertex_height_map = blob;
            let long_enough = record.vertex_height_map.len() >= HEIGHT_BLOB_LEN;
            let full = build_cell_cpu(&record, &HashMap::new());
            let lod = build_cell_cpu_lod(&record, &HashMap::new());
            prop_assert_eq!(full.is_ok(), long_enough);
            prop_assert_eq!(lod.is_ok(), long_enough);
        }

        #[test]
        fn splat_weights_of_a_valid_layer_stack_stay_within_one_unit_of_full(
            opacities in proptest::collection::vec(0.0f32..=1.0, 1..4),
        ) {
            // Weights past full opacity are not renormalised, so only stacks
            // that stay within one unit are constrained.
            prop_assume!(opacities.iter().sum::<f32>() <= 1.0);
            let mut records = vec![layer_record("grass", 0, true, &[])];
            for (n, opacity) in opacities.iter().enumerate() {
                records.push(layer_record(&format!("t{n}"), 0, false, &full_alpha(*opacity)));
            }
            let map: HashMap<String, String> = (0..4).map(|n| (format!("t{n}"), format!("t{n}.dds")))
                .chain([("grass".to_string(), "grass.dds".to_string())])
                .collect();
            let cell = build_cell_cpu(&landscape(0, 0, 0.0, &[], Some(layers_blob(&records))), &map).unwrap();
            let [a, b, c, d] = splat_pixel(&cell, 20, 20);
            let sum = u32::from(a) + u32::from(b) + u32::from(c) + u32::from(d);
            prop_assert!(sum.abs_diff(255) <= 4, "sum {}", sum);
        }
    }

    #[test]
    fn alpha_sample_at_grid_corners_returns_the_corner_value() {
        let mut grid = [[0.0f32; ALPHA_GRID]; ALPHA_GRID];
        grid[0][0] = 0.25;
        grid[ALPHA_GRID - 1][ALPHA_GRID - 1] = 0.75;
        assert_eq!(sample_alpha_bilinear(&grid, 0, 0, 64), 0.25);
        assert!((sample_alpha_bilinear(&grid, 63, 63, 64) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn alpha_sample_midway_between_two_grid_points_is_their_average() {
        let mut grid = [[0.0f32; ALPHA_GRID]; ALPHA_GRID];
        grid[0][0] = 1.0;
        // quadrant_size 33 -> 32 pixel steps over 16 grid steps: pixel 1 is halfway to grid x = 1.
        let sample = sample_alpha_bilinear(&grid, 1, 0, 33);
        assert!((sample - 0.5).abs() < 1e-6);
    }
}
