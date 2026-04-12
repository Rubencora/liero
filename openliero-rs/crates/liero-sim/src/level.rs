//! Level — terrain pixel map and material lookup.
//!
//! Each pixel stores a palette index (u8).  Material flags for a pixel come
//! from `tc_materials[palette_index]`, where `tc_materials` is the slice from
//! `TcData::constants.materials`.
//!
//! The level wraps on X (toroidal) to match C++ `checkedMatWrap`.
//! Out-of-bounds Y is treated as background (no collision).

use crate::material::Material;

/// Standard level dimensions (pixels), matching C++ `Level::WIDTH/HEIGHT`.
pub const WIDTH:  u32 = 504;
pub const HEIGHT: u32 = 350;

/// Terrain pixel map: palette index per pixel.
pub struct Level {
    pub width:  u32,
    pub height: u32,
    pixels: Vec<u8>,
    /// Bounding box of pixels written since last `take_dirty()` call.
    /// (x0, y0, x1_exclusive, y1_exclusive).
    pub dirty: Option<(u32, u32, u32, u32)>,
}

impl Level {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0u8; (width * height) as usize],
            dirty: None,
        }
    }

    /// Raw read-only slice of all palette indices (row-major, top-to-bottom).
    #[inline(always)]
    pub fn pixels(&self) -> &[u8] { &self.pixels }

    /// Overwrite all pixels from a snapshot buffer.  Used by rollback restore.
    pub fn restore_pixels(&mut self, snapshot: &[u8]) {
        debug_assert_eq!(snapshot.len(), self.pixels.len());
        self.pixels.copy_from_slice(snapshot);
        self.dirty = Some((0, 0, self.width, self.height)); // mark all dirty
    }

    /// Consume and return the accumulated dirty bounding box, resetting it to None.
    pub fn take_dirty(&mut self) -> Option<(u32, u32, u32, u32)> {
        self.dirty.take()
    }

    /// True if `(x, y)` is within level bounds.
    #[inline(always)]
    pub fn inside(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height
    }

    /// Raw palette index at `(x, y)`.  Returns 0 (background) if out-of-bounds.
    #[inline(always)]
    pub fn pixel(&self, x: i32, y: i32) -> u8 {
        if self.inside(x, y) {
            self.pixels[(y as u32 * self.width + x as u32) as usize]
        } else {
            0
        }
    }

    /// Raw palette index with X-wrapping (toroidal), Y-clamped.
    ///
    /// Mirrors C++ `Level::checkedMatWrap`: X wraps modulo `width`, Y < 0 or
    /// ≥ height returns 0 (background).
    #[inline(always)]
    pub fn pixel_wrap(&self, x: i32, y: i32) -> u8 {
        if y < 0 || (y as u32) >= self.height {
            return 0;
        }
        let wx = x.rem_euclid(self.width as i32) as u32;
        self.pixels[(y as u32 * self.width + wx) as usize]
    }

    /// Write a palette index, ignoring out-of-bounds writes.
    /// Expands the dirty bounding box to cover the modified pixel.
    #[inline(always)]
    pub fn set_pixel(&mut self, x: i32, y: i32, colour: u8) {
        if self.inside(x, y) {
            self.pixels[(y as u32 * self.width + x as u32) as usize] = colour;
            let (xu, yu) = (x as u32, y as u32);
            self.dirty = Some(match self.dirty {
                None => (xu, yu, xu + 1, yu + 1),
                Some((x0, y0, x1, y1)) => (x0.min(xu), y0.min(yu), x1.max(xu + 1), y1.max(yu + 1)),
            });
        }
    }

    /// Material at `(x, y)` looked up from `tc_materials`.
    /// Returns `Material::BACKGROUND` for out-of-bounds pixels.
    #[inline(always)]
    pub fn material(&self, x: i32, y: i32, tc_materials: &[Material]) -> Material {
        let idx = self.pixel(x, y) as usize;
        tc_materials.get(idx).copied().unwrap_or(Material::BACKGROUND)
    }

    /// Material at `(x, y)` with X-wrapping.
    #[inline(always)]
    pub fn material_wrap(&self, x: i32, y: i32, tc_materials: &[Material]) -> Material {
        let idx = self.pixel_wrap(x, y) as usize;
        tc_materials.get(idx).copied().unwrap_or(Material::BACKGROUND)
    }

    /// Convenience: is this pixel traversable (background)?
    #[inline(always)]
    pub fn is_background(&self, x: i32, y: i32, tc_materials: &[Material]) -> bool {
        self.material(x, y, tc_materials).background()
    }
}
