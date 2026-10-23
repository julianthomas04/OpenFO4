use glam::Vec2;

const MAX_CELLS_PER_AXIS: f32 = 512.0;
const MIN_CELL_SIZE: f32 = 1e-3;

/// Uniform XZ grid over triangle bounding boxes, stored as compressed rows of triangle indices.
#[derive(Debug, Clone)]
pub struct TriangleGrid {
    origin: Vec2,
    cell_size: f32,
    cols: usize,
    rows: usize,
    starts: Vec<u32>,
    entries: Vec<u16>,
}

impl TriangleGrid {
    pub fn build(triangle_bounds: &[(Vec2, Vec2)], min: Vec2, max: Vec2) -> Self {
        let extent = (max - min).max(Vec2::ZERO);
        let count = triangle_bounds.len().max(1) as f32;
        let area = (extent.x * extent.y).max(f32::MIN_POSITIVE);
        let cell_size = (area / count)
            .sqrt()
            .max(extent.max_element() / MAX_CELLS_PER_AXIS)
            .max(MIN_CELL_SIZE);
        let cols = (extent.x / cell_size) as usize + 1;
        let rows = (extent.y / cell_size) as usize + 1;
        let mut grid = Self {
            origin: min,
            cell_size,
            cols,
            rows,
            starts: vec![0; cols * rows + 1],
            entries: Vec::new(),
        };
        grid.fill(triangle_bounds);
        grid
    }

    fn fill(&mut self, triangle_bounds: &[(Vec2, Vec2)]) {
        for &(lo, hi) in triangle_bounds {
            for cell in self.cells_in(lo, hi).collect::<Vec<_>>() {
                self.starts[cell + 1] += 1;
            }
        }
        for i in 1..self.starts.len() {
            self.starts[i] += self.starts[i - 1];
        }
        let mut cursor = self.starts.clone();
        self.entries = vec![0; self.starts[self.starts.len() - 1] as usize];
        for (tri, &(lo, hi)) in triangle_bounds.iter().enumerate() {
            for cell in self.cells_in(lo, hi).collect::<Vec<_>>() {
                self.entries[cursor[cell] as usize] = tri as u16;
                cursor[cell] += 1;
            }
        }
    }

    fn cell_coord(&self, p: Vec2) -> (usize, usize) {
        let local = (p - self.origin) / self.cell_size;
        let col = (local.x.max(0.0) as usize).min(self.cols - 1);
        let row = (local.y.max(0.0) as usize).min(self.rows - 1);
        (col, row)
    }

    fn cells_in(&self, lo: Vec2, hi: Vec2) -> impl Iterator<Item = usize> + '_ {
        let (col_lo, row_lo) = self.cell_coord(lo);
        let (col_hi, row_hi) = self.cell_coord(hi);
        (row_lo..=row_hi)
            .flat_map(move |row| (col_lo..=col_hi).map(move |col| row * self.cols + col))
    }

    /// Triangles whose bounds overlap the box; a triangle spanning several cells repeats.
    pub fn candidates(&self, lo: Vec2, hi: Vec2) -> impl Iterator<Item = u16> + '_ {
        self.cells_in(lo, hi).flat_map(move |cell| {
            let range = self.starts[cell] as usize..self.starts[cell + 1] as usize;
            self.entries[range].iter().copied()
        })
    }
}
