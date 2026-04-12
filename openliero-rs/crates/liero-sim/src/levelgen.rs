//! Procedural level generator — port of `Level::generateRandom()` from C++.
//!
//! Entry point: [`generate`].  Generates a 504×350 palette-indexed terrain
//! identical in appearance and determinism to the original Liero/OpenLiero
//! level generator for the same RNG seed.
//!
//! # Algorithm (mirrors `level.cpp`)
//! 1. `generate_dirt_pattern()` — averaged noise + stamp decorative sprites
//! 2. Wandering dirt paths via `draw_dirt_effect()` (50-54 paths)
//! 3. Large 32×32 rocks (2×2 grid of 16×16 stone sprites, 5-19 rocks)
//! 4. Small rocks (single 16×16 stone sprite, 5-29 rocks)
//! 5. `make_shadow()` — darken pixels under/near rocks

use crate::level::Level;
use crate::material::Material;
use crate::rand::Mwc;
use liero_data::{Tc, SPRITE_W, SPRITE_SIZE};

/// Stone sprite indices: `STONE_TAB[rock_type][corner]`.
/// Mirrors `stoneTab[3][4]` in `common.cpp`.
const STONE_TAB: [[usize; 4]; 3] = [
    [98, 60, 61, 62],
    [63, 75, 85, 86],
    [89, 90, 97, 96],
];

/// Generate a random level, overwriting `level` with palette-indexed terrain.
///
/// `tc` provides large sprite data, materials, and texture definitions.
/// `rand` must already be seeded (caller's responsibility).
/// `shadow` mirrors `Settings::shadow` — applies the shadow darkening pass.
pub fn generate(level: &mut Level, tc: &Tc, rand: &mut Mwc, shadow: bool) {
    generate_dirt_pattern(level, tc, rand);
    generate_dirt_paths(level, tc, rand);
    place_large_rocks(level, tc, rand);
    place_small_rocks(level, tc, rand);
    if shadow {
        make_shadow(level, tc);
    }
}

/// Select a valid worm spawn point of size `w × h`.
///
/// Uses reservoir sampling (uniform random over all valid squares).
/// Returns `Some((x, y))` or `None` if no valid spot found.
/// Mirrors `Level::selectSpawn()` in `level.cpp`.
pub fn select_spawn(level: &Level, tc_materials: &[Material], rand: &mut Mwc, w: i32, h: i32) -> Option<(i32, i32)> {
    let width  = level.width as i32;
    let height = level.height as i32;

    let mut vruns  = vec![0i32; (width - w + 1) as usize];
    let mut vdists = vec![0i32; (width - w + 1) as usize];

    let mut count: u32 = 0;
    let mut selected: Option<(i32, i32)> = None;

    for y in 0..height {
        let mut hrun  = 0i32;
        let mut filled = 0i32;

        for x in 0..width {
            let mat = mat_at(level, tc_materials, x, y);
            if is_free(mat) {
                hrun += 1;
            } else {
                hrun = 0;
                filled += 1;
            }

            let cx = x - (w - 1);
            if cx < 0 { continue; }

            let vrun  = &mut vruns[cx as usize];
            let vdist = &mut vdists[cx as usize];

            if hrun >= w {
                if *vdist > 0 {
                    *vrun  = 0;
                    *vdist = 0;
                }
                *vrun += 1;
            } else {
                if *vrun >= h && *vdist <= 8 && filled > w / 4 {
                    count += 1;
                    if rand.rand(count) < 1 {
                        selected = Some((cx, y - h));
                    }
                }
                *vdist += 1;
            }

            let back_mat = mat_at(level, tc_materials, x - w + 1 - 1, y); // x - w
            filled -= !is_free(back_mat) as i32;
        }
    }

    selected
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Fill the level with averaged random noise (palette indices 12-19) and stamp
/// decorative sprites.
/// Mirrors `Level::generateDirtPattern()`.
fn generate_dirt_pattern(level: &mut Level, tc: &Tc, rand: &mut Mwc) {
    let w = level.width as i32;
    let h = level.height as i32;

    // Smoothed noise initialisation.
    level.set_pixel(0, 0, rand.rand(7) as u8 + 12);

    for y in 1..h {
        let prev = level.pixel(0, y - 1);
        level.set_pixel(0, y, ((rand.rand(7) as u8 + 12) as u16 + prev as u16 >> 1) as u8);
    }

    for x in 1..w {
        let prev = level.pixel(x - 1, 0);
        level.set_pixel(x, 0, ((rand.rand(7) as u8 + 12) as u16 + prev as u16 >> 1) as u8);
    }

    for y in 1..h {
        for x in 1..w {
            let left = level.pixel(x - 1, y) as u32;
            let up   = level.pixel(x, y - 1) as u32;
            let r    = rand.rand(8) as u32 + 12;
            level.set_pixel(x, y, ((left + up + r) / 3) as u8);
        }
    }

    // Stamp texture sprites (69-72), with special blend for palette 176-179.
    let count = rand.rand(100) as i32;
    for _ in 0..count {
        let x    = rand.rand(level.width) as i32 - 8;
        let y    = rand.rand(level.height) as i32 - 8;
        let temp = rand.rand(4) as usize + 69;
        let frame = &tc.large_sprites[temp * SPRITE_SIZE .. (temp + 1) * SPRITE_SIZE];

        for cy in 0..16i32 {
            let my = cy + y;
            if my >= h { break; }
            if my < 0  { continue; }
            for cx in 0..16i32 {
                let mx = cx + x;
                if mx >= w { break; }
                if mx < 0  { continue; }
                let src_px = frame[(cy as usize) * SPRITE_W + cx as usize];
                if src_px > 0 {
                    let cur = level.pixel(mx, my);
                    let new_px = if cur > 176 && cur < 180 {
                        ((src_px as u16 + cur as u16) / 2) as u8
                    } else {
                        src_px
                    };
                    level.set_pixel(mx, my, new_px);
                }
            }
        }
    }

    // Stamp decorative stone sprites (56-59).
    let count2 = rand.rand(15) as i32;
    for _ in 0..count2 {
        let x     = rand.rand(level.width) as i32 - 8;
        let y     = rand.rand(level.height) as i32 - 8;
        let which = rand.rand(4) as usize + 56;
        blit_stone(level, &tc.large_sprites, which, x, y);
    }
}

/// Generate wandering dirt-effect paths.
/// Mirrors the first `rand(50)+5` loop in `generateRandom()`.
fn generate_dirt_paths(level: &mut Level, tc: &Tc, rand: &mut Mwc) {
    let count = rand.rand(50) as i32 + 5;
    for _ in 0..count {
        let mut cx = rand.rand(level.width) as i32 - 8;
        let mut cy = rand.rand(level.height) as i32 - 8;
        let dx     = rand.rand(11) as i32 - 5;
        let dy     = rand.rand(5) as i32 - 2;
        let count2 = rand.rand(12) as i32;

        for _ in 0..count2 {
            let count3 = rand.rand(5) as i32;
            for _ in 0..count3 {
                cx += dx;
                cy += dy;
                draw_dirt_effect(level, tc, rand, 1, cx, cy);
            }
            cx -= (count3 + 1) * dx;
            cy -= (count3 + 1) * dy;
            cx += rand.rand(7) as i32 - 3;
            cy += rand.rand(15) as i32 - 7;
        }
    }
}

/// Place large 32×32 rocks (2×2 grid of 16×16 stone sprites).
/// Mirrors the `rand(15)+5` loop in `generateRandom()`.
fn place_large_rocks(level: &mut Level, tc: &Tc, rand: &mut Mwc) {
    let count = rand.rand(15) as i32 + 5;
    for _ in 0..count {
        let (cx, cy) = loop {
            let cx = rand.rand(level.width) as i32 - 16;
            let cy = if rand.rand(4) == 0 {
                level.height as i32 - 1 - rand.rand(20) as i32
            } else {
                rand.rand(level.height) as i32 - 16
            };
            if is_no_rock(level, tc, 32, cx, cy) {
                break (cx, cy);
            }
        };

        let rock = rand.rand(3) as usize;
        blit_stone(level, &tc.large_sprites, STONE_TAB[rock][0], cx,      cy);
        blit_stone(level, &tc.large_sprites, STONE_TAB[rock][1], cx + 16, cy);
        blit_stone(level, &tc.large_sprites, STONE_TAB[rock][2], cx,      cy + 16);
        blit_stone(level, &tc.large_sprites, STONE_TAB[rock][3], cx + 16, cy + 16);
    }
}

/// Place small single-sprite rocks.
/// Mirrors the `rand(25)+5` loop in `generateRandom()`.
fn place_small_rocks(level: &mut Level, tc: &Tc, rand: &mut Mwc) {
    let count = rand.rand(25) as i32 + 5;
    for _ in 0..count {
        let (cx, cy) = loop {
            let cx = rand.rand(level.width) as i32 - 8;
            let cy = if rand.rand(5) == 0 {
                level.height as i32 - 1 - rand.rand(13) as i32
            } else {
                rand.rand(level.height) as i32 - 8
            };
            if is_no_rock(level, tc, 15, cx, cy) {
                break (cx, cy);
            }
        };

        let which = rand.rand(6) as usize + 3;
        blit_stone(level, &tc.large_sprites, which, cx, cy);
    }
}

/// Apply shadow darkening to pixels near rock edges.
/// Mirrors `Level::makeShadow()`.
fn make_shadow(level: &mut Level, tc: &Tc) {
    let w = level.width as i32;
    let h = level.height as i32;

    for x in 0..w - 3 {
        for y in 3..h {
            let m  = mat_from_tc(tc, level.pixel(x, y));
            let m2 = mat_from_tc(tc, level.pixel(x + 3, y - 3));
            let p  = level.pixel(x, y);

            if m.see_shadow() && m2.dirt_rock() {
                level.set_pixel(x, y, p.wrapping_add(4));
            }

            if p >= 12 && p <= 18 && m2.rock() {
                let new_p = p.saturating_sub(2).max(12);
                level.set_pixel(x, y, new_p);
            }
        }
    }

    // Bottom row: unfilled background pixels become palette 13.
    for x in 0..w {
        if mat_from_tc(tc, level.pixel(x, h - 1)).background() {
            level.set_pixel(x, h - 1, 13);
        }
    }
}

/// `drawDirtEffect` — stamp the mask-driven texture at `(x, y)`.
/// `effect_idx` indexes into `tc.data.constants.textures`.
/// Mirrors `drawDirtEffect()` in `blit.cpp`.
fn draw_dirt_effect(level: &mut Level, tc: &Tc, rand: &mut Mwc, effect_idx: usize, x: i32, y: i32) {
    let Some(tex) = tc.data.constants.textures.get(effect_idx) else { return };

    let t_frame_idx = tex.sframe as usize + rand.rand(tex.rframe as u32) as usize;
    let m_frame_idx = tex.mframe as usize;

    // Bounds check to avoid panics on unusual TCs.
    let sprite_count = tc.large_sprites.len() / SPRITE_SIZE;
    if t_frame_idx >= sprite_count || m_frame_idx >= sprite_count { return; }

    let t_frame = &tc.large_sprites[t_frame_idx * SPRITE_SIZE .. (t_frame_idx + 1) * SPRITE_SIZE];
    let m_frame = &tc.large_sprites[m_frame_idx * SPRITE_SIZE .. (m_frame_idx + 1) * SPRITE_SIZE];

    let w = level.width as i32;
    let h = level.height as i32 - 1; // C++ clips to height-1

    let ndrawback = tex.ndrawback;

    for y_ in 0..16i32 {
        let my = y + y_;
        if my < 0 || my >= h { continue; }
        for x_ in 0..16i32 {
            let mx = x + x_;
            if mx < 0 || mx >= w { continue; }

            let c   = m_frame[(y_ as usize) * SPRITE_W + x_ as usize];
            let cur = level.pixel(mx, my);
            let mat = mat_from_tc(tc, cur);

            let new_px: Option<u8> = if ndrawback {
                // ndrawback = true: only paint over dirt
                match c {
                    6 => if mat.any_dirt() {
                        Some(t_frame[((my as usize & 15) << 4) + (mx as usize & 15)])
                    } else { None },
                    1 => if (cur & 2) != 0 { // dirt2
                        Some(2)
                    } else if mat.dirt() {
                        Some(1)
                    } else { None },
                    _ => None,
                }
            } else {
                // ndrawback = false: only paint over background
                match c {
                    6 | 10 => if mat.background() {
                        Some(t_frame[((my as usize & 15) << 4) + (mx as usize & 15)])
                    } else { None },
                    2 => if mat.background() { Some(2) } else { None },
                    1 => if mat.background() { Some(1) } else { None },
                    _ => None,
                }
            };

            if let Some(px) = new_px {
                level.set_pixel(mx, my, px);
            }
        }
    }
}

/// Blit a 16×16 stone sprite (p1=false variant): non-zero pixels overwrite.
/// Clips to level bounds. Mirrors `blitStone(..., p1=false, ...)`.
fn blit_stone(level: &mut Level, large_sprites: &[u8], frame: usize, x: i32, y: i32) {
    let w = level.width as i32;
    let h = level.height as i32;

    if frame * SPRITE_SIZE + SPRITE_SIZE > large_sprites.len() { return; }
    let src = &large_sprites[frame * SPRITE_SIZE .. (frame + 1) * SPRITE_SIZE];

    for cy in 0..16i32 {
        let my = y + cy;
        if my < 0 || my >= h { continue; }
        for cx in 0..16i32 {
            let mx = x + cx;
            if mx < 0 || mx >= w { continue; }
            let c = src[(cy as usize) * SPRITE_W + cx as usize];
            if c != 0 {
                level.set_pixel(mx, my, c);
            }
        }
    }
}

/// True if no rock pixels exist in the `size × size` area starting at `(x, y)`.
/// Mirrors `isNoRock()`.
fn is_no_rock(level: &Level, tc: &Tc, size: i32, x: i32, y: i32) -> bool {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + size + 1).min(level.width as i32);
    let y1 = (y + size + 1).min(level.height as i32);

    for py in y0..y1 {
        for px in x0..x1 {
            if mat_from_tc(tc, level.pixel(px, py)).rock() {
                return false;
            }
        }
    }
    true
}

/// Compute material from TC materials table for a palette index.
#[inline(always)]
fn mat_from_tc(tc: &Tc, idx: u8) -> Material {
    tc.data.constants.materials
        .get(idx as usize)
        .map(|&b| Material(b))
        .unwrap_or(Material::BACKGROUND)
}

/// Compute material from a precomputed slice (for selectSpawn).
#[inline(always)]
fn mat_at(level: &Level, tc_materials: &[Material], x: i32, y: i32) -> Material {
    if x < 0 || y < 0 || x >= level.width as i32 || y >= level.height as i32 {
        return Material::BACKGROUND;
    }
    tc_materials.get(level.pixel(x, y) as usize).copied().unwrap_or(Material::BACKGROUND)
}

/// A pixel is "free" for spawn purposes if it is background or any dirt.
#[inline(always)]
fn is_free(m: Material) -> bool {
    m.background() || m.any_dirt()
}
