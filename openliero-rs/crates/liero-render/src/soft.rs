//! Software (CPU) renderer for Canvas 2D WASM target.
//! No wgpu/winit dependencies.

use font8x8::UnicodeFonts;
use liero_data::{
    Tc, SMALL_SPRITE_H, SMALL_SPRITE_SIZE, SMALL_SPRITE_W, SPRITE_H, SPRITE_SIZE, SPRITE_W,
};
use liero_sim::game::Game;
use liero_sim::level::{HEIGHT as LEVEL_H, WIDTH as LEVEL_W};
use liero_sim::math::COSSIN_TABLE;

const RENDER_W: u32 = 320;
const RENDER_H: u32 = 200;
const HUD_Y: i32 = 158;

struct Viewport {
    worm_idx: usize,
    cam_x: i32,
    cam_y: i32,
    target_cam_x: i32,
    target_cam_y: i32,
    rect: [i32; 4],
}

impl Viewport {
    fn w(&self) -> i32 {
        self.rect[2] - self.rect[0]
    }
    fn h(&self) -> i32 {
        self.rect[3] - self.rect[1]
    }
    fn off_x(&self) -> i32 {
        self.rect[0] - self.cam_x
    }
    fn off_y(&self) -> i32 {
        self.rect[1] - self.cam_y
    }
}

const VP_LAYOUTS: [&[([i32; 4], usize)]; 5] = [
    &[],
    &[],
    // 2P: left / right
    &[([0, 0, 158, HUD_Y], 0), ([160, 0, 318, HUD_Y], 1)],
    // 3P: top-left, top-right, bottom-center
    &[
        ([0, 0, 158, 79], 0),
        ([160, 0, 318, 79], 1),
        ([80, 79, 238, HUD_Y], 2),
    ],
    // 4P: quadrants
    &[
        ([0, 0, 158, 78], 0),
        ([160, 0, 318, 78], 1),
        ([0, 80, 158, HUD_Y], 2),
        ([160, 80, 318, HUD_Y], 3),
    ],
];

pub struct SoftRenderer {
    frame_buf: Vec<u8>,
    palette_rgba: Vec<[u8; 4]>,
    viewports: Vec<Viewport>,
    health_color: u8,
    screen_flash: i32,
}

fn closest_palette_color(palette: &[[u8; 4]], r: u8, g: u8, b: u8) -> u8 {
    let mut best = 0u8;
    let mut best_dist = i32::MAX;
    for (i, &[pr, pg, pb, _]) in palette.iter().enumerate() {
        let d = (r as i32 - pr as i32).pow(2)
            + (g as i32 - pg as i32).pow(2)
            + (b as i32 - pb as i32).pow(2);
        if d < best_dist {
            best_dist = d;
            best = i as u8;
        }
    }
    best
}

impl SoftRenderer {
    pub fn new(tc: &Tc) -> Self {
        let palette_rgba = tc.palette_rgba();
        let health_color = closest_palette_color(&palette_rgba, 220, 30, 30);
        Self {
            frame_buf: vec![0u8; (RENDER_W * RENDER_H) as usize],
            palette_rgba,
            viewports: Vec::new(),
            health_color,
            screen_flash: 0,
        }
    }

    pub fn set_player_count(&mut self, n: usize) {
        let layout = VP_LAYOUTS[n.clamp(2, 4)];
        self.viewports = layout
            .iter()
            .map(|&(rect, worm_idx)| Viewport {
                worm_idx,
                cam_x: 0,
                cam_y: 0,
                target_cam_x: 0,
                target_cam_y: 0,
                rect,
            })
            .collect();
    }

    /// Write the current frame as RGBA bytes into `out` (must be 320*200*4 bytes).
    /// Applies screen flash (palette brightness boost) if active.
    pub fn write_rgba(&self, out: &mut [u8]) {
        let fl = self.screen_flash.clamp(0, 100) as u32;
        for (i, &idx) in self.frame_buf.iter().enumerate() {
            let [r, g, b, a] = self.palette_rgba[idx as usize];
            let dst = i * 4;
            out[dst] = (r as u32 + fl * 2).min(255) as u8;
            out[dst + 1] = (g as u32 + fl * 2).min(255) as u8;
            out[dst + 2] = (b as u32 + fl * 2).min(255) as u8;
            out[dst + 3] = a;
        }
    }

    pub fn render(&mut self, game: &Game) {
        self.screen_flash = game.screen_flash;
        self.update_cameras(game);
        self.compose_frame(game);
        self.apply_color_anim(game);
    }

    pub fn render_menu(&mut self, title: &str, options: &[&str], cursor: usize, extra: &[&str]) {
        const C_BG: u8 = 0;
        const C_PNL: u8 = 1;
        const C_NRM: u8 = 7;
        const C_SEL: u8 = 15;
        const C_TTL: u8 = 14;
        const C_HNT: u8 = 4;

        self.frame_buf.fill(C_BG);
        self.fb_rect(0, 0, RENDER_W as i32, 18, C_PNL);
        self.fb_text_center(title, 5, C_TTL);
        self.fb_rect(0, 18, RENDER_W as i32, 1, C_NRM);

        let base_y = 26i32;
        let line_h = 13i32;
        for (i, &opt) in options.iter().enumerate() {
            let y = base_y + i as i32 * line_h;
            if i == cursor {
                self.fb_rect(4, y - 2, RENDER_W as i32 - 8, line_h - 1, C_PNL);
                self.fb_glyph('>', 6, y, C_TTL);
                self.fb_text(opt, 18, y, C_SEL);
            } else {
                self.fb_text(opt, 18, y, C_NRM);
            }
        }

        if !extra.is_empty() {
            let hint_y = RENDER_H as i32 - 10 - extra.len() as i32 * 10;
            self.fb_rect(0, hint_y - 4, RENDER_W as i32, 1, C_PNL);
            for (i, &line) in extra.iter().enumerate() {
                self.fb_text_center(line, hint_y + i as i32 * 10, C_HNT);
            }
        }
    }

    pub fn render_game_over(&mut self, msg: &str, scores: &[(i32, i32)]) {
        const C_BG: u8 = 0;
        const C_PNL: u8 = 1;
        const C_MSG: u8 = 15;
        const C_HNT: u8 = 7;
        const C_DIM: u8 = 4;

        self.frame_buf.fill(C_BG);
        let panel_h = if scores.is_empty() { 44 } else { 58 };
        let panel_y = RENDER_H as i32 / 2 - panel_h / 2;
        self.fb_rect(20, panel_y, RENDER_W as i32 - 40, panel_h, C_PNL);
        self.fb_rect(20, panel_y, RENDER_W as i32 - 40, 1, C_MSG);
        self.fb_rect(20, panel_y + panel_h - 1, RENDER_W as i32 - 40, 1, C_MSG);
        self.fb_text_center(msg, panel_y + 10, C_MSG);

        if !scores.is_empty() {
            let n = scores.len();
            let entry_w = 63i32;
            let total_w = entry_w * n as i32 - 18;
            let mut sx = (RENDER_W as i32 - total_w) / 2;
            for (pi, &(kills, _)) in scores.iter().enumerate() {
                let label = format!("P{}:{}", pi + 1, kills);
                self.fb_text(&label, sx, panel_y + 28, C_DIM);
                sx += entry_w;
            }
        }

        let hint_y = panel_y + panel_h - 14;
        self.fb_text_center("PRESS ENTER TO RETURN", hint_y, C_HNT);
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn update_cameras(&mut self, game: &Game) {
        const LERP_DIV: i32 = 8;
        for (vi, vp) in self.viewports.iter_mut().enumerate() {
            let wi = vp.worm_idx;
            if wi >= game.worms.len() {
                continue;
            }
            let worm = &game.worms[wi];
            let wx = worm.pos.x.0 >> 16;
            let wy = worm.pos.y.0 >> 16;
            let vw = vp.w();
            let vh = vp.h();
            let tx = (wx - vw / 2).clamp(0, LEVEL_W as i32 - vw);
            let ty = (wy - vh / 2).clamp(0, LEVEL_H as i32 - vh);
            vp.target_cam_x = tx;
            vp.target_cam_y = ty;

            let dx = tx - vp.cam_x;
            let dy = ty - vp.cam_y;
            vp.cam_x += if dx.abs() <= 1 { dx } else { dx / LERP_DIV };
            vp.cam_y += if dy.abs() <= 1 { dy } else { dy / LERP_DIV };

            // Task 2: Screen shake with smooth sinusoidal decay
            // Replace discrete jitter with exponentially decaying sine wave
            let shake_fp = game.viewport_shake.get(vi).copied().unwrap_or(0);
            let real_shake = shake_fp >> 16; // ftoi
            if real_shake > 0 {
                // Use sine/cosine approximation via COSSIN_TABLE for smooth oscillation
                // Oscillation phase based on cycle count scaled by frequency
                let t = ((game.cycles as u32).wrapping_mul(51)) & 0x7f; // Oscillation frequency: 0–127 index
                let (sin_dy, cos_dx) = COSSIN_TABLE[t as usize];

                // Decay: normalize shake_magnitude and apply exponential decay
                let decay = (real_shake as f32 / 65536.0).clamp(0.0, 1.0);
                let offset_x = ((cos_dx as f32 * decay * 4.0) / 65536.0) as i32;
                let offset_y = ((sin_dy as f32 * 0.7 * decay * 4.0) / 65536.0) as i32;

                vp.cam_x = (vp.cam_x + offset_x).clamp(0, LEVEL_W as i32 - vw);
                vp.cam_y = (vp.cam_y + offset_y).clamp(0, LEVEL_H as i32 - vh);
            }
        }
    }

    fn compose_frame(&mut self, game: &Game) {
        self.frame_buf.fill(0);
        let vp_count = self.viewports.len().min(game.worms.len() + 1);

        for vi in 0..vp_count {
            let (rect, ox, oy) = {
                let vp = &self.viewports[vi];
                (vp.rect, vp.off_x(), vp.off_y())
            };

            // Terrain
            for sy in rect[1]..rect[3] {
                for sx in rect[0]..rect[2] {
                    let idx = game.level.pixel(sx - ox, sy - oy);
                    self.frame_buf[(sy as u32 * RENDER_W + sx as u32) as usize] = idx;
                }
            }

            // Wobjects
            for wob in &game.wobjects {
                if wob.weapon_idx >= game.tc.weapons.len() {
                    continue;
                }
                let weapon = &game.tc.weapons[wob.weapon_idx];
                let px = (wob.pos.x.0 >> 16) + ox;
                let py = (wob.pos.y.0 >> 16) + oy;
                if weapon.start_frame >= 0 {
                    let sprite_count = game.tc.small_sprites.len() / SMALL_SPRITE_SIZE;
                    if sprite_count == 0 {
                        continue;
                    }
                    let frame = if weapon.shot_type == 4 {
                        // STLaser = 4: cur_frame is the angle (0-127), map to sprite frame 0-15
                        (wob.cur_frame / 8).clamp(0, 15)
                    } else if weapon.shot_type == 2 {
                        // STSteerable = 2: angle 12-127 maps to sprite frame 0-12
                        ((wob.cur_frame.max(12) - 12) / 8).clamp(0, 12)
                    } else {
                        // Animation cycling for other projectiles
                        wob.cur_frame.clamp(0, weapon.num_frames as i32 - 1)
                    };

                    let frame_idx =
                        ((weapon.start_frame as i32 + frame) as usize).min(sprite_count - 1);
                    blit_small(
                        &mut self.frame_buf,
                        RENDER_W,
                        rect,
                        &game.tc.small_sprites,
                        frame_idx,
                        px,
                        py,
                    );
                } else {
                    // Pixel-mode bullet: cur_frame holds the palette colour index.
                    let color = wob.cur_frame as u8;
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, px, py, color);

                    // Task 1: Add shadow for pixel-mode wobjects if weapon has shadow flag
                    if weapon.shadow {
                        let shadow_px = px + 1;
                        let shadow_py = py + 1;
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, shadow_px, shadow_py, 0);
                        // Black shadow
                    }
                }
            }

            // Nobjects (particles, blood, splinters)
            for nob in &game.nobjects {
                if nob.nobj_type >= game.tc.nobjects.len() {
                    continue;
                }
                let ntype = &game.tc.nobjects[nob.nobj_type];
                let px = (nob.pos.x.0 >> 16) + ox;
                let py = (nob.pos.y.0 >> 16) + oy;
                if ntype.start_frame >= 0 {
                    let sprite_count = game.tc.small_sprites.len() / SMALL_SPRITE_SIZE;
                    if sprite_count == 0 {
                        continue;
                    }
                    let frame =
                        ((ntype.start_frame as i32 + nob.cur_frame) as usize).min(sprite_count - 1);
                    blit_small(
                        &mut self.frame_buf,
                        RENDER_W,
                        rect,
                        &game.tc.small_sprites,
                        frame,
                        px,
                        py,
                    );
                } else {
                    // Pixel-mode particle: cur_frame is the colour index.
                    let color = nob.cur_frame as u8;
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, px, py, color);

                    // Task 2: Add shadow for pixel-mode nobjects at (x+1, y+1)
                    let shadow_px = px + 1;
                    let shadow_py = py + 1;
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, shadow_px, shadow_py, 0);
                    // Black shadow
                }
            }

            // SObjects
            for sobj in &game.sobjects {
                if sobj.type_idx >= game.tc.sobjects.len() {
                    continue;
                }
                let stype = &game.tc.sobjects[sobj.type_idx];
                let frame_idx = ((stype.start_frame as i32 + sobj.cur_frame) as usize)
                    .min((game.tc.large_sprites.len() / SPRITE_SIZE).saturating_sub(1));
                blit_large(
                    &mut self.frame_buf,
                    RENDER_W,
                    rect,
                    &game.tc.large_sprites,
                    frame_idx,
                    sobj.x + ox,
                    sobj.y + oy,
                );

                // Explosion ring effect — expanding ring on early frames
                if stype.num_frames > 0 && sobj.cur_frame < 8 {
                    let center_x = sobj.x + ox + 16; // Center of 32×32 sprite
                    let center_y = sobj.y + oy + 16;
                    let radius = sobj.cur_frame as i32 * 3;
                    let _fade = 255 - (sobj.cur_frame as i32 * 30).min(200); // Fade to transparent
                    let color = closest_palette_color(&self.palette_rgba, 255, 200, 0); // Orange/yellow

                    // Draw circle outline using bresenham-like sampling
                    for angle_deg in (0..360).step_by(5) {
                        let angle_rad = (angle_deg as f32).to_radians();
                        let cx = (center_x as f32 + radius as f32 * angle_rad.cos()) as i32;
                        let cy = (center_y as f32 + radius as f32 * angle_rad.sin()) as i32;
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, cx, cy, color);
                    }
                }
            }

            // Bonuses (weapon crates / health packs)
            for bonus in &game.bonuses {
                if let Some(b) = bonus {
                    if b.used {
                        continue;
                    }
                    // Bonus flicker: only draw when timer is high or every 4th frame
                    let should_draw = b.timer > game.tc.data.constants.bonus_flicker_time
                        || (game.cycles % 4 == 0);
                    if !should_draw {
                        continue;
                    }
                    let bx = (b.x >> 16) + ox;
                    let by = (b.y >> 16) + oy;

                    // Try to use associated sobject sprite if available
                    let mut rendered = false;
                    if b.frame >= 0 && b.frame < game.tc.data.constants.bonuses.len() as i32 {
                        let bonus_type = &game.tc.data.constants.bonuses[b.frame as usize];
                        if let Some(ref sobj_name) = bonus_type.sobj {
                            // Find the sobject type index by name in data.types.sobjects
                            if let Some(sobj_idx) = game
                                .tc
                                .data
                                .types
                                .sobjects
                                .iter()
                                .position(|name| name == sobj_name)
                            {
                                if sobj_idx < game.tc.sobjects.len() {
                                    let stype = &game.tc.sobjects[sobj_idx];
                                    let frame_idx = ((stype.start_frame as i32) as usize).min(
                                        (game.tc.large_sprites.len() / SPRITE_SIZE)
                                            .saturating_sub(1),
                                    );
                                    blit_large(
                                        &mut self.frame_buf,
                                        RENDER_W,
                                        rect,
                                        &game.tc.large_sprites,
                                        frame_idx,
                                        bx,
                                        by,
                                    );
                                    rendered = true;
                                }
                            }
                        }
                    }

                    // Try to use large sprite at index bonus.frame + 20 if not already rendered
                    if !rendered && b.frame >= 0 {
                        let sprite_idx = (b.frame as usize + 20)
                            .min((game.tc.large_sprites.len() / SPRITE_SIZE).saturating_sub(1));
                        blit_large(
                            &mut self.frame_buf,
                            RENDER_W,
                            rect,
                            &game.tc.large_sprites,
                            sprite_idx,
                            bx,
                            by,
                        );
                        rendered = true;
                    }

                    // Fallback: draw simple colored rectangle
                    if !rendered {
                        // Weapon crate (frame=0): yellow
                        // Health pack (frame=1): red/magenta
                        let color = if b.frame == 0 { 226 } else { 240 };
                        for dy in 0..5 {
                            for dx in 0..5 {
                                put_pixel(
                                    &mut self.frame_buf,
                                    RENDER_W,
                                    rect,
                                    bx + dx,
                                    by + dy,
                                    color,
                                );
                            }
                        }
                    }

                    // Draw bonus weapon name overlay (C++ viewport.cpp:316-327)
                    // Only for weapon crates (frame == 0)
                    if b.frame == 0 && b.weapon >= 0 && (b.weapon as usize) < game.tc.weapons.len()
                    {
                        let weapon = &game.tc.weapons[b.weapon as usize];
                        let name = &weapon.name;
                        let text_width = name.len() as i32 * 9;
                        let text_x = bx - text_width / 2;
                        let text_y = by - 10;
                        const BONUS_TEXT_COLOR: u8 = 255; // White
                        for (i, c) in name.chars().enumerate() {
                            let char_x = text_x + i as i32 * 9;
                            self.draw_char_to_buf(c, char_x, text_y, BONUS_TEXT_COLOR, rect);
                        }
                    }
                }
            }

            // Render race checkpoints
            if let liero_sim::game::GameMode::Race { ref checkpoints, .. } = game.mode {
                for (cp_idx, &(cpx, cpy)) in checkpoints.iter().enumerate() {
                    let screen_x = cpx + ox;
                    let screen_y = cpy + oy;

                    // Check if any worm is targeting this checkpoint
                    let is_next_for_any = game.worms.iter().enumerate().any(|(wi, w)| {
                        w.alive && game.worm_checkpoint[wi] as usize == cp_idx
                    });

                    // Draw checkpoint marker
                    let color = if is_next_for_any {
                        closest_palette_color(&self.palette_rgba, 0, 255, 0) // bright green = active
                    } else {
                        closest_palette_color(&self.palette_rgba, 100, 100, 100) // gray = done/future
                    };

                    // Draw a small diamond at checkpoint position
                    let r = 5i32;
                    for dy in -r..=r {
                        for dx in -r..=r {
                            if dx.abs() + dy.abs() <= r {
                                // diamond shape
                                let px = screen_x + dx;
                                let py = screen_y + dy;
                                put_pixel(&mut self.frame_buf, RENDER_W, rect, px, py, color);
                            }
                        }
                    }

                    // Draw checkpoint number as text
                    let num_str = format!("{}", cp_idx + 1);
                    if let Some(first_char) = num_str.chars().next() {
                        self.draw_char_to_buf(first_char, screen_x - 3, screen_y - 10, 255, rect);
                    }
                }
            }

            // Ghost worm at spawn location (Task 2: allowViewingSpawnPoint)
            // When a worm is dead and waiting to respawn, show a checkerboard ghost preview
            const GHOST_WORM_COLOR: u8 = 50; // dim color
            for worm in &game.worms {
                if worm.alive || worm.killed_timer <= 0 {
                    continue;
                }
                let wx = (worm.pos.x.0 >> 16) + ox;
                let wy = (worm.pos.y.0 >> 16) + oy;
                // Draw checkerboard pattern (only every other pixel)
                for dy in -4..=4 {
                    for dx in -4..=4 {
                        if (dx + dy) % 2 == 0 {
                            continue; // Skip for checkerboard effect
                        }
                        let px = wx + dx;
                        let py = wy + dy;
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, px, py, GHOST_WORM_COLOR);
                    }
                }
            }

            // Worm shadows — dark ellipse below each visible worm.
            const SHADOW_COLOR: u8 = 1; // very dark palette color
            for worm in &game.worms {
                if !worm.visible {
                    continue;
                }
                let wx = (worm.pos.x.0 >> 16) + ox;
                let wy = (worm.pos.y.0 >> 16) + oy + 16; // 16px = bottom of worm sprite + 2
                                                         // Draw 8px wide oval shadow.
                for sx in 0..8i32 {
                    put_pixel(
                        &mut self.frame_buf,
                        RENDER_W,
                        rect,
                        wx + sx - 4,
                        wy,
                        SHADOW_COLOR,
                    );
                    put_pixel(
                        &mut self.frame_buf,
                        RENDER_W,
                        rect,
                        wx + sx - 4,
                        wy + 1,
                        SHADOW_COLOR,
                    );
                }
            }

            // Worms
            for worm in &game.worms {
                if !worm.visible {
                    continue;
                }
                let wx = (worm.pos.x.0 >> 16) + ox;
                let wy = (worm.pos.y.0 >> 16) + oy;
                let slot = worm.index & 3;
                let dir = worm.direction.clamp(0, 1) as usize;
                let frame = worm.cur_frame.clamp(0, 20) as usize;
                blit_large(
                    &mut self.frame_buf,
                    RENDER_W,
                    rect,
                    &game.tc.worm_sprites,
                    frame + dir * 21 + slot * 42,
                    wx - 7,
                    wy - 5,
                );

                // Task 1: Damage flash overlay — red tint when worm takes damage
                if worm.damage_flash > 0 {
                    // Blend toward red for non-background pixels in the worm sprite area
                    // Worm sprite is 32×32, centered at (wx, wy); top-left is (wx - 7, wy - 5)
                    let flash_x = wx - 7;
                    let flash_y = wy - 5;

                    for dy in 0..32i32 {
                        for dx in 0..32i32 {
                            let px = flash_x + dx;
                            let py = flash_y + dy;

                            // Check bounds against viewport
                            if px < rect[0] || py < rect[1] || px >= rect[2] || py >= rect[3] {
                                continue;
                            }

                            let idx = (py as u32 * RENDER_W + px as u32) as usize;
                            if idx >= self.frame_buf.len() {
                                continue;
                            }

                            let current_color = self.frame_buf[idx];

                            // Skip background (palette index 0)
                            if current_color == 0 {
                                continue;
                            }

                            // Get current pixel's RGB
                            let [r, g, b, _] = self.palette_rgba[current_color as usize];

                            // Blend toward red: (r + 255) / 2, g / 2, b / 2
                            let blend_r = ((r as i32 + 255) / 2).min(255) as u8;
                            let blend_g = (g as i32 / 2) as u8;
                            let blend_b = (b as i32 / 2) as u8;

                            // Find closest palette color for blended RGB
                            let new_color = closest_palette_color(&self.palette_rgba, blend_r, blend_g, blend_b);

                            // Only update if we found a good match
                            self.frame_buf[idx] = new_color;
                        }
                    }
                }
            }

            // Laser sight line — drawn for weapons with laser_sight=true.
            // Dotted line from worm toward aim, stops at terrain or 150px.
            for worm in &game.worms {
                if !worm.visible {
                    continue;
                }
                let cur_slot = worm.current_weapon.clamp(0, 4) as usize;
                let cur_weapon_idx = worm.weapon_slots.get(cur_slot).copied().unwrap_or(cur_slot);
                let weapon = match game.tc.weapons.get(cur_weapon_idx) {
                    Some(w) => w,
                    None => continue,
                };
                if !weapon.laser_sight {
                    continue;
                }
                let angle = (worm.aiming_angle >> 16) as usize & 0x7f;
                let (dx, dy) = COSSIN_TABLE[angle];
                let wx = (worm.pos.x.0 >> 16) + ox;
                let wy = (worm.pos.y.0 >> 16) + oy;
                // Step along laser direction 1px at a time (max 150px).
                for dist in 1..=150i32 {
                    let lx = wx + (dx * dist) / 65536;
                    let ly = wy + (dy * dist) / 65536;
                    // Stop at terrain.
                    let world_x = lx - ox;
                    let world_y = ly - oy;
                    if !game.level.inside(world_x, world_y) {
                        break;
                    }
                    if game
                        .level
                        .material(world_x, world_y, &game.tc_materials)
                        .dirt_rock()
                    {
                        break;
                    }
                    // Draw every other pixel (dotted line effect).
                    if dist % 2 == 0 {
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, lx, ly, 255);
                        // white
                    }
                }
            }

            // Crosshairs — mirrors C++ viewport.cpp:
            // pos = worm.pos + (-1,-2) + cossinTable[angle]*16, then blit smallSprites[43].
            // Small sprite is 7×7 pixels; sprite 43 = white cross, 44 = green (makeSightGreen).
            const SMALL_W: i32 = 7;
            const SMALL_H: i32 = 7;
            const SMALL_SZ: usize = 49;
            for worm in &game.worms {
                if !worm.visible {
                    continue;
                }
                let angle = (worm.aiming_angle >> 16) as usize & 0x7f;
                let (dx, dy) = COSSIN_TABLE[angle];
                // Worm hotspot offset: (-1, -2) from top-left; aim direction 16px.
                let wx = (worm.pos.x.0 >> 16) + ox - 1;
                let wy = (worm.pos.y.0 >> 16) + oy - 2;
                let cx = wx + (dx * 16) / 65536;
                let cy = wy + (dy * 16) / 65536;
                // Use small sprite 43 (white crosshair) — palette index 0 = transparent.
                let sprite_idx = 43usize;
                let sprite_start = sprite_idx * SMALL_SZ;
                if let Some(sprite) = game
                    .tc
                    .small_sprites
                    .get(sprite_start..sprite_start + SMALL_SZ)
                {
                    for row in 0..SMALL_H {
                        for col in 0..SMALL_W {
                            let px = sprite[(row * SMALL_W + col) as usize];
                            if px != 0 {
                                put_pixel(
                                    &mut self.frame_buf,
                                    RENDER_W,
                                    rect,
                                    cx + col,
                                    cy + row,
                                    px,
                                );
                            }
                        }
                    }
                } else {
                    // Fallback: 5-pixel plus in white if sprite not available.
                    let color: u8 = 255;
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, cx, cy, color);
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, cx - 1, cy, color);
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, cx + 1, cy, color);
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, cx, cy - 1, color);
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, cx, cy + 1, color);
                }
            }

            // Worm labels (P1/P2/P3/P4) and weapon names
            const WORM_LABEL_COLORS: [u8; 4] = [200, 210, 220, 230];
            for worm in &game.worms {
                if !worm.visible {
                    continue;
                }
                let label = match worm.index {
                    0 => "P1",
                    1 => "P2",
                    2 => "P3",
                    _ => "P4",
                };
                let wx = (worm.pos.x.0 >> 16) + ox;
                let wy = (worm.pos.y.0 >> 16) + oy;

                // Draw weapon name above the worm in bright yellow
                let cur_slot = worm.current_weapon.clamp(0, 4) as usize;
                let cur_weapon_idx = worm.weapon_slots.get(cur_slot).copied().unwrap_or(cur_slot);
                if let Some(weapon) = game.tc.weapons.get(cur_weapon_idx) {
                    let weapon_name = &weapon.name;
                    let weapon_color = closest_palette_color(&self.palette_rgba, 255, 255, 0); // Yellow
                    let text_width = weapon_name.len() as i32 * 9;
                    let wx_text = wx - text_width / 2;
                    let wy_text = wy - 26; // above the player label
                    for (i, c) in weapon_name.chars().enumerate() {
                        let char_x = wx_text + i as i32 * 9;
                        self.draw_char_to_buf(c, char_x, wy_text, weapon_color, rect);
                    }
                }

                // Draw player label
                let lx = wx - 8; // center 2-char label (2*9=18px) around worm
                let ly = wy - 14; // above the worm sprite
                let color = WORM_LABEL_COLORS[worm.index & 3];
                self.fb_text(label, lx, ly, color);
            }

            // Task 3: Floating damage numbers rendering
            let damage_text_color = closest_palette_color(&self.palette_rgba, 255, 255, 0); // Bright yellow
            for fd in &game.floating_damages {
                // Convert world position to screen position
                let screen_x = (fd.x >> 16) + ox;
                let screen_y = (fd.y >> 16) + oy;

                // Only render if still visible (timer > 0)
                if fd.timer <= 0 {
                    continue;
                }

                // Blink effect near end: skip render on odd frames in last 10 frames
                if fd.timer < 10 && fd.timer % 2 == 0 {
                    continue;
                }

                // Format damage number as "+{amount}"
                let damage_text = format!("+{}", fd.amount);

                // Center the text horizontally around the damage position
                let text_width = damage_text.len() as i32 * 9;
                let text_x = screen_x - text_width / 2;
                let text_y = screen_y;

                // Draw each character
                for (i, c) in damage_text.chars().enumerate() {
                    let char_x = text_x + i as i32 * 9;
                    self.draw_char_to_buf(c, char_x, text_y, damage_text_color, rect);
                }
            }

            // Fire cone animation (cooling reload flame)
            for worm in &game.worms {
                if !worm.visible || worm.fire_cone <= 0 {
                    continue;
                }
                let cur_slot = worm.current_weapon.clamp(0, 4) as usize;
                let cur_weapon_idx = worm.weapon_slots.get(cur_slot).copied().unwrap_or(cur_slot);
                if let Some(weapon) = game.tc.weapons.get(cur_weapon_idx) {
                    if weapon.fire_cone <= 0 {
                        continue;
                    }
                }
                // Task 5: Draw simple fire cone indicator — 3-5 pixels in the aiming direction
                let angle = (worm.aiming_angle >> 16) as usize & 0x7f;
                let (dx, dy) = COSSIN_TABLE[angle];
                let wx = (worm.pos.x.0 >> 16) + ox - 1;
                let wy = (worm.pos.y.0 >> 16) + oy - 2;
                // Draw a simple triangular flash of 5 pixels in the fire direction
                const FIRE_CONE_COLOR: u8 = 226; // Orange/yellow
                for dist in 1..=5i32 {
                    let fx = wx + (dx * dist) / 65536;
                    let fy = wy + (dy * dist) / 65536;
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, fx, fy, FIRE_CONE_COLOR);
                }
            }

            // Ninja rope lines
            for worm in &game.worms {
                if !worm.rope_active {
                    continue;
                }
                let hx = (worm.rope_hook.x.0 >> 16) + ox;
                let hy = (worm.rope_hook.y.0 >> 16) + oy;
                let wx = (worm.pos.x.0 >> 16) + ox;
                let wy = (worm.pos.y.0 >> 16) + oy;
                const ROPE_COLOR: u8 = 7;
                // Task 6: Draw rope shadow before main line
                const ROPE_SHADOW_COLOR: u8 = 0; // Black
                draw_rope_line(
                    &mut self.frame_buf,
                    RENDER_W,
                    rect,
                    wx + 1,
                    wy + 1,
                    hx + 1,
                    hy + 1,
                    ROPE_SHADOW_COLOR,
                );
                draw_rope_line(
                    &mut self.frame_buf,
                    RENDER_W,
                    rect,
                    wx,
                    wy,
                    hx,
                    hy,
                    ROPE_COLOR,
                );

                // Render rope hook sprite if attached
                if worm.rope_attached {
                    let sprite_idx = 84usize;
                    let sprite_size = SPRITE_SIZE;
                    let start = sprite_idx * sprite_size;
                    if start + sprite_size <= game.tc.large_sprites.len() {
                        blit_large(
                            &mut self.frame_buf,
                            RENDER_W,
                            rect,
                            &game.tc.large_sprites,
                            sprite_idx,
                            hx - 8,
                            hy - 8,
                        );
                    } else {
                        // Fallback: draw a 3x3 pixel cross in rope color if sprite not available
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, hx, hy, ROPE_COLOR);
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, hx - 1, hy, ROPE_COLOR);
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, hx + 1, hy, ROPE_COLOR);
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, hx, hy - 1, ROPE_COLOR);
                        put_pixel(&mut self.frame_buf, RENDER_W, rect, hx, hy + 1, ROPE_COLOR);
                    }
                }
            }

            // "PRESS FIRE" message when worm is dead and respawn-pending
            if vi < game.worms.len() {
                let worm = &game.worms[vi];
                if !worm.visible && worm.killed_timer <= 0 && !worm.ready {
                    // Draw centered text in the viewport
                    let text = "PRESS FIRE";
                    let text_width = text.len() as i32 * 9; // 9px per character
                    let vp_center_x = rect[0] + (rect[2] - rect[0]) / 2;
                    let vp_center_y = rect[1] + (rect[3] - rect[1]) / 2;
                    let text_x = vp_center_x - text_width / 2;
                    let text_y = vp_center_y - 4; // slightly above center
                                                  // Draw text with black shadow and white foreground (like in C++)
                    for (i, c) in text.chars().enumerate() {
                        let char_x = text_x + i as i32 * 9;
                        // Shadow (dark)
                        self.draw_char_to_buf(c, char_x + 1, text_y + 1, 0, rect);
                        // Foreground (bright white)
                        self.draw_char_to_buf(c, char_x, text_y, 50, rect);
                    }
                }

                // Task 2: Damage flash vignette overlay — red edges when worm is hit
                if vi < game.worms.len() && game.worms[vi].damage_flash > 0 {
                    let flash_intensity = game.worms[vi].damage_flash.min(8);

                    // Draw red vignette at edges of viewport
                    const VIGNETTE_WIDTH: i32 = 20;
                    for y in rect[1]..rect[3] {
                        for x in rect[0]..rect[2] {
                            // Calculate distance to nearest edge
                            let edge_dist_left = x - rect[0];
                            let edge_dist_right = rect[2] - x;
                            let edge_dist_top = y - rect[1];
                            let edge_dist_bottom = rect[3] - y;

                            let min_edge_dist = edge_dist_left
                                .min(edge_dist_right)
                                .min(edge_dist_top)
                                .min(edge_dist_bottom);

                            // Only draw vignette at edges (within VIGNETTE_WIDTH)
                            if min_edge_dist < VIGNETTE_WIDTH {
                                // Fade effect based on distance from edge
                                let fade = (VIGNETTE_WIDTH - min_edge_dist) as f32 / VIGNETTE_WIDTH as f32;
                                let intensity = (fade * flash_intensity as f32 * 0.3).min(0.5);

                                // Blend current color toward red
                                let idx = (y as u32 * RENDER_W + x as u32) as usize;
                                if idx < self.frame_buf.len() {
                                    let current_color = self.frame_buf[idx];
                                    let [r, g, b, _] = self.palette_rgba[current_color as usize];

                                    let new_r = (r as f32 + (200.0 - r as f32) * intensity) as u8;
                                    let new_g = (g as f32 * (1.0 - intensity)) as u8;
                                    let new_b = (b as f32 * (1.0 - intensity)) as u8;

                                    let vignette_color =
                                        closest_palette_color(&self.palette_rgba, new_r, new_g, new_b);
                                    self.frame_buf[idx] = vignette_color;
                                }
                            }
                        }
                    }
                }
            }
        }

        self.draw_hud(game);
    }

    fn draw_hud(&mut self, game: &Game) {
        const FULL_BAR_W: i32 = 80;
        const BAR_GAP: i32 = 160;
        const HUD_BG: u8 = 0;
        const TEXT_Y: i32 = HUD_Y + 2;
        const HP_TOP: i32 = HUD_Y + 12;
        const HP_H: i32 = 4;
        const RL_TOP: i32 = HP_TOP + HP_H + 3;
        const RL_H: i32 = 3;

        let clip = [0i32, HUD_Y, RENDER_W as i32, RENDER_H as i32];

        // Pass 1: background fill and text labels (borrows self fully via fb_rect / fb_text)
        self.fb_rect(0, HUD_Y, RENDER_W as i32, RENDER_H as i32 - HUD_Y, HUD_BG);

        for (i, worm) in game.worms.iter().enumerate() {
            let bar_x = (i as i32) * BAR_GAP;
            let hp = worm.health.clamp(0, 100);
            // Only show lives counter for game modes where lives are limited
            let label = if matches!(game.mode, liero_sim::game::GameMode::LastManStanding) {
                format!(
                    "P{} {:3}% K:{} L:{}",
                    i + 1,
                    hp,
                    worm.kills,
                    worm.lives.max(0)
                )
            } else {
                format!("P{} {:3}% K:{}", i + 1, hp, worm.kills)
            };
            let health_color = self.health_color;
            self.fb_text(&label, bar_x + 2, TEXT_Y, health_color);
        }

        // Pass 2: bars — only borrows self.frame_buf
        for (i, worm) in game.worms.iter().enumerate() {
            let bar_x = (i as i32) * BAR_GAP;
            let hp = worm.health.clamp(0, 100);

            // Health bar background
            for px in bar_x..bar_x + FULL_BAR_W {
                for py in HP_TOP..HP_TOP + HP_H {
                    put_pixel(&mut self.frame_buf, RENDER_W, clip, px, py, HUD_BG);
                }
            }
            // Health bar fill with gradient color based on health percentage
            // C++: lifebarWidth / 10 + 234, clamped to 244 max
            // Maps: 100% health → 234 (green), 50% → 239 (yellow), 0% → 244+ (red)
            let health_w = hp * FULL_BAR_W / 100;
            let health_pct_div_10 = (hp / 10).clamp(0, 10);
            let health_bar_color = (234 + health_pct_div_10).clamp(234, 244) as u8;
            for px in bar_x..bar_x + health_w {
                for py in HP_TOP..HP_TOP + HP_H {
                    put_pixel(
                        &mut self.frame_buf,
                        RENDER_W,
                        clip,
                        px,
                        py,
                        health_bar_color,
                    );
                }
            }

            // Reload bar background
            for px in bar_x..bar_x + FULL_BAR_W {
                for py in RL_TOP..RL_TOP + RL_H {
                    put_pixel(&mut self.frame_buf, RENDER_W, clip, px, py, HUD_BG);
                }
            }
            // Reload bar fill with gradient color based on reload percentage
            // C++: ammoPct / 10 + 245, clamped to 255 max
            // Maps: 100% ammo → 245 (full), 50% → 250 (halfway), 0% → 255+ (empty)
            let slot = worm.current_weapon.clamp(0, 4) as usize;
            let weapon_idx = worm.weapon_slots.get(slot).copied().unwrap_or(0);
            let loading_time = game
                .tc
                .weapons
                .get(weapon_idx)
                .map(|w| w.loading_time)
                .unwrap_or(0);
            let (reload_w, reload_bar_color) = if loading_time > 0 {
                let reload_left = worm.reload_timers.get(slot).copied().unwrap_or(0).max(0);
                let ready_pct =
                    (1.0 - (reload_left as f32 / loading_time as f32).clamp(0.0, 1.0)) * 100.0;
                let ready_pct_int = ready_pct as i32;
                let pct_div_10 = (ready_pct_int / 10).clamp(0, 10);
                let color = (245 + pct_div_10).clamp(245, 255) as u8;
                ((ready_pct * FULL_BAR_W as f32 / 100.0) as i32, color)
            } else {
                (FULL_BAR_W, 245u8)
            };
            for px in bar_x..bar_x + reload_w {
                for py in RL_TOP..RL_TOP + RL_H {
                    put_pixel(
                        &mut self.frame_buf,
                        RENDER_W,
                        clip,
                        px,
                        py,
                        reload_bar_color,
                    );
                }
            }

            // Draw "RELOADING" text flash: only when weapon is not ready and every other second
            let reload_left = worm.reload_timers.get(slot).copied().unwrap_or(0).max(0);
            if reload_left > 0 && (game.cycles % 20) > 10 {
                // Bright yellow: find closest palette color
                let reload_text_color = closest_palette_color(&self.palette_rgba, 255, 255, 0);
                let text_y = RL_TOP - 10;
                for (i, c) in "RELOADING".chars().enumerate() {
                    let char_x = bar_x + 2 + i as i32 * 9;
                    self.draw_char_to_buf(c, char_x, text_y, reload_text_color, clip);
                }
            }
        }

        // Game-mode specific indicators
        self.draw_game_mode_hud(game, clip);

        // Minimap in bottom-right corner
        self.draw_minimap(game, clip);
    }

    fn draw_game_mode_hud(&mut self, game: &Game, clip: [i32; 4]) {
        use liero_sim::game::GameMode;

        match game.mode {
            GameMode::KingOfHill { time_limit: _ } => {
                // Draw KotH zone indicator: show which worm is in the zone (if any)
                let mut in_zone_count = 0;
                let mut zone_holder_idx = 0usize;
                for (i, ticks) in game.koth_ticks.iter().enumerate() {
                    if *ticks > 0 {
                        in_zone_count += 1;
                        zone_holder_idx = i;
                    }
                }

                // Show zone status at top-center
                let text = if in_zone_count == 1 {
                    format!("P{} IN ZONE", zone_holder_idx + 1)
                } else if in_zone_count > 1 {
                    "CONTESTED".to_string()
                } else {
                    "ZONE EMPTY".to_string()
                };

                let text_width = text.len() as i32 * 9;
                let x = (RENDER_W as i32 - text_width) / 2;
                let y = 2i32;

                let color = if in_zone_count == 1 {
                    // Green for holder
                    closest_palette_color(&self.palette_rgba, 0, 200, 0)
                } else if in_zone_count > 1 {
                    // Yellow for contested
                    closest_palette_color(&self.palette_rgba, 255, 200, 0)
                } else {
                    // Gray for empty
                    closest_palette_color(&self.palette_rgba, 128, 128, 128)
                };

                for (i, c) in text.chars().enumerate() {
                    let char_x = x + i as i32 * 9;
                    self.draw_char_to_buf(c, char_x, y, color, clip);
                }
            }
            GameMode::GameOfTag { time_limit: _ } => {
                // Task 3: Draw who is "it" at top-center
                if let Some(tag_idx) = game.tag_worm {
                    let text = format!("IT: P{}", tag_idx + 1);
                    let text_width = text.len() as i32 * 9;
                    let x = (RENDER_W as i32 - text_width) / 2;
                    let y = 2i32;

                    // Flash red if this is the local player's worm (warning effect)
                    let color = if tag_idx < game.worms.len() && tag_idx < 4 {
                        // Red for warning (the current player is "it")
                        closest_palette_color(&self.palette_rgba, 255, 0, 0)
                    } else {
                        // White for others
                        closest_palette_color(&self.palette_rgba, 255, 255, 255)
                    };

                    for (i, c) in text.chars().enumerate() {
                        let char_x = x + i as i32 * 9;
                        self.draw_char_to_buf(c, char_x, y, color, clip);
                    }
                }
            }
            GameMode::ScalesOfJustice => {
                // Task 4: Draw Scales of Justice indicator at top-center
                let text = "SCALES";
                let text_width = text.len() as i32 * 9;
                let x = (RENDER_W as i32 - text_width) / 2;
                let y = 2i32;

                // Use a balanced color (cyan/light blue for balance theme)
                let color = closest_palette_color(&self.palette_rgba, 0, 200, 255);

                for (i, c) in text.chars().enumerate() {
                    let char_x = x + i as i32 * 9;
                    self.draw_char_to_buf(c, char_x, y, color, clip);
                }
            }
            GameMode::TeamDeathmatch => {
                // Could add team score display here in future
            }
            _ => {
                // Other modes don't need special HUD indicators for now
            }
        }
    }

    /// Apply color animation (palette cycling) to the frame buffer.
    /// Pre-computes a 256-entry remap table once per frame, then applies it in a single pass.
    fn apply_color_anim(&mut self, game: &Game) {
        // Build remap table (256 entries, identity by default)
        let mut remap = [0u8; 256];
        for i in 0..256 {
            remap[i] = i as u8;
        }

        // Apply each color_anim range to the remap table
        for anim in &game.tc.data.constants.color_anim {
            let from = anim.from.clamp(0, 255) as usize;
            let to = anim.to.clamp(0, 255) as usize;
            if from >= to {
                continue; // Invalid range
            }
            let range_len = to - from + 1;
            let offset = (game.cycles as usize) % range_len;

            for i in 0..range_len {
                remap[from + i] = (from + ((i + offset) % range_len)) as u8;
            }
        }

        // Single pass over pixels using the precomputed remap table
        for px in self.frame_buf.iter_mut() {
            *px = remap[*px as usize];
        }
    }

    fn draw_minimap(&mut self, game: &Game, clip: [i32; 4]) {
        const MINIMAP_W: i32 = 64;
        const MINIMAP_H: i32 = 40;
        const MINIMAP_X: i32 = RENDER_W as i32 - MINIMAP_W - 2;
        const MINIMAP_Y: i32 = HUD_Y + 2;

        // Draw black background for minimap
        const BLACK: u8 = 0;
        for py in MINIMAP_Y..MINIMAP_Y + MINIMAP_H {
            for px in MINIMAP_X..MINIMAP_X + MINIMAP_W {
                put_pixel(&mut self.frame_buf, RENDER_W, clip, px, py, BLACK);
            }
        }

        // Draw minimap terrain (sample every Nth pixel)
        let sample_rate = ((LEVEL_W + MINIMAP_W as u32 - 1) / MINIMAP_W as u32).max(1);
        for mmy in 0..MINIMAP_H {
            for mmx in 0..MINIMAP_W {
                let world_x = (mmx as u32 * sample_rate).min(LEVEL_W - 1);
                let world_y = (mmy as u32 * sample_rate).min(LEVEL_H - 1);
                let terrain_color = game.level.pixel(world_x as i32, world_y as i32);
                let px = MINIMAP_X + mmx;
                let py = MINIMAP_Y + mmy;
                put_pixel(&mut self.frame_buf, RENDER_W, clip, px, py, terrain_color);
            }
        }

        // Draw worms as colored 2×2 dots
        for worm in &game.worms {
            let wx = (worm.pos.x.0 >> 16) as i32;
            let wy = (worm.pos.y.0 >> 16) as i32;
            let mmx = MINIMAP_X + wx * MINIMAP_W as i32 / LEVEL_W as i32;
            let mmy = MINIMAP_Y + wy * MINIMAP_H as i32 / LEVEL_H as i32;

            // Draw 2×2 dot in worm's color
            let worm_color = match worm.index {
                0 => closest_palette_color(&self.palette_rgba, 255, 0, 0), // Red
                1 => closest_palette_color(&self.palette_rgba, 0, 255, 0), // Green
                2 => closest_palette_color(&self.palette_rgba, 0, 0, 255), // Blue
                _ => closest_palette_color(&self.palette_rgba, 255, 255, 0), // Yellow
            };
            for dy in 0..2i32 {
                for dx in 0..2i32 {
                    put_pixel(
                        &mut self.frame_buf,
                        RENDER_W,
                        clip,
                        mmx + dx,
                        mmy + dy,
                        worm_color,
                    );
                }
            }
        }

        // Draw bonuses as white 1×1 dots
        const BONUS_COLOR: u8 = 255; // White
        for bonus in &game.bonuses {
            if let Some(b) = bonus {
                if !b.used && b.timer > 0 {
                    let bx = (b.x >> 16) as i32;
                    let by = (b.y >> 16) as i32;
                    let mmx = MINIMAP_X + bx * MINIMAP_W as i32 / LEVEL_W as i32;
                    let mmy = MINIMAP_Y + by * MINIMAP_H as i32 / LEVEL_H as i32;
                    put_pixel(&mut self.frame_buf, RENDER_W, clip, mmx, mmy, BONUS_COLOR);
                }
            }
        }

        // Draw minimap border
        const BORDER_COLOR: u8 = 7; // Light gray
        for px in MINIMAP_X..MINIMAP_X + MINIMAP_W {
            put_pixel(
                &mut self.frame_buf,
                RENDER_W,
                clip,
                px,
                MINIMAP_Y - 1,
                BORDER_COLOR,
            );
            put_pixel(
                &mut self.frame_buf,
                RENDER_W,
                clip,
                px,
                MINIMAP_Y + MINIMAP_H,
                BORDER_COLOR,
            );
        }
        for py in MINIMAP_Y..MINIMAP_Y + MINIMAP_H {
            put_pixel(
                &mut self.frame_buf,
                RENDER_W,
                clip,
                MINIMAP_X - 1,
                py,
                BORDER_COLOR,
            );
            put_pixel(
                &mut self.frame_buf,
                RENDER_W,
                clip,
                MINIMAP_X + MINIMAP_W,
                py,
                BORDER_COLOR,
            );
        }
    }

    fn fb_put(&mut self, x: i32, y: i32, color: u8) {
        if x < 0 || y < 0 || x >= RENDER_W as i32 || y >= RENDER_H as i32 {
            return;
        }
        self.frame_buf[(y as u32 * RENDER_W + x as u32) as usize] = color;
    }

    fn fb_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u8) {
        for py in y..y + h {
            for px in x..x + w {
                self.fb_put(px, py, color);
            }
        }
    }

    fn fb_glyph(&mut self, c: char, x: i32, y: i32, color: u8) {
        let bits = font8x8::BASIC_FONTS
            .get(c)
            .or_else(|| font8x8::BASIC_FONTS.get(' '))
            .unwrap_or([0u8; 8]);
        for (row, &byte) in bits.iter().enumerate() {
            for col in 0..8i32 {
                if byte & (1 << col) != 0 {
                    self.fb_put(x + col, y + row as i32, color);
                }
            }
        }
    }

    fn fb_text(&mut self, s: &str, x: i32, y: i32, color: u8) {
        for (i, c) in s.chars().enumerate() {
            self.fb_glyph(c, x + i as i32 * 9, y, color);
        }
    }

    fn fb_text_center(&mut self, s: &str, y: i32, color: u8) {
        let w = s.len() as i32 * 9;
        let x = ((RENDER_W as i32) - w) / 2;
        self.fb_text(s, x, y, color);
    }

    fn draw_char_to_buf(&mut self, c: char, x: i32, y: i32, color: u8, clip: [i32; 4]) {
        let bits = font8x8::BASIC_FONTS
            .get(c)
            .or_else(|| font8x8::BASIC_FONTS.get(' '))
            .unwrap_or([0u8; 8]);
        for (row, &byte) in bits.iter().enumerate() {
            for col in 0..8i32 {
                if byte & (1 << col) != 0 {
                    put_pixel(
                        &mut self.frame_buf,
                        RENDER_W,
                        clip,
                        x + col,
                        y + row as i32,
                        color,
                    );
                }
            }
        }
    }
}

// ── Shared blit helpers ───────────────────────────────────────────────────────

#[inline(always)]
fn put_pixel(buf: &mut [u8], buf_w: u32, clip: [i32; 4], x: i32, y: i32, color: u8) {
    if x < clip[0] || y < clip[1] || x >= clip[2] || y >= clip[3] {
        return;
    }
    buf[(y as u32 * buf_w + x as u32) as usize] = color;
}

fn blit_small(
    buf: &mut [u8],
    buf_w: u32,
    clip: [i32; 4],
    sprites: &[u8],
    frame: usize,
    x: i32,
    y: i32,
) {
    let start = frame * SMALL_SPRITE_SIZE;
    if start + SMALL_SPRITE_SIZE > sprites.len() {
        return;
    }
    let src = &sprites[start..start + SMALL_SPRITE_SIZE];
    for row in 0..SMALL_SPRITE_H as i32 {
        for col in 0..SMALL_SPRITE_W as i32 {
            let c = src[(row as usize) * SMALL_SPRITE_W + col as usize];
            if c != 0 {
                put_pixel(buf, buf_w, clip, x + col, y + row, c);
            }
        }
    }
}

fn blit_large(
    buf: &mut [u8],
    buf_w: u32,
    clip: [i32; 4],
    sprites: &[u8],
    frame: usize,
    x: i32,
    y: i32,
) {
    let start = frame * SPRITE_SIZE;
    if start + SPRITE_SIZE > sprites.len() {
        return;
    }
    let src = &sprites[start..start + SPRITE_SIZE];
    for row in 0..SPRITE_H as i32 {
        for col in 0..SPRITE_W as i32 {
            let c = src[(row as usize) * SPRITE_W + col as usize];
            if c != 0 {
                put_pixel(buf, buf_w, clip, x + col, y + row, c);
            }
        }
    }
}

fn draw_rope_line(
    buf: &mut [u8],
    buf_w: u32,
    clip: [i32; 4],
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: u8,
) {
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let steps = dx.max(dy).max(1);
    for i in 0..=steps {
        let x = x0 + (x1 - x0) * i / steps;
        let y = y0 + (y1 - y0) * i / steps;
        put_pixel(buf, buf_w, clip, x, y, color);
    }
}
