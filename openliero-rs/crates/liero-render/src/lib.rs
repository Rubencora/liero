//! `liero-render` — GPU renderer.
//!
//! Also provides `render_menu` / `render_game_over` for drawing
//! pixel-art menu screens into the same 320×200 framebuffer.
//!
//! CPU compositing into a 320×200 paletted framebuffer, split into two
//! 158×158 viewports (one per worm) plus a 320×42 HUD strip.
//! The buffer is uploaded as an R8Uint texture each frame; a 256×1
//! Rgba8Unorm LUT holds the palette; a fragment shader resolves indices → RGBA.
//!
//! # R2 scope
//! - 320×200 render resolution (2× → 640×400 window).
//! - Two split-screen viewports, each following its worm with a snapping camera.
//! - Worm sprites (16×16, pre-remapped per slot/direction/frame).
//! - Wobject / nobject pixels or small sprites (7×7).
//! - HUD health bars.
//! - Optional CRT scanline darkening (toggle via `set_scanlines`).

use std::sync::Arc;

use anyhow::{Context, Result};
use font8x8::UnicodeFonts;
use bytemuck::{Pod, Zeroable};
use liero_data::{Tc, SPRITE_W, SPRITE_H, SPRITE_SIZE, SMALL_SPRITE_W, SMALL_SPRITE_H, SMALL_SPRITE_SIZE};
use liero_sim::game::Game;
use liero_sim::level::{WIDTH as LEVEL_W, HEIGHT as LEVEL_H};
use wgpu::util::DeviceExt;
use winit::window::Window;

// ── Render resolution ─────────────────────────────────────────────────────────

/// Render-buffer width/height (C++ render resolution).
const RENDER_W: u32 = 320;
const RENDER_H: u32 = 200;

/// Y coordinate where the viewport area ends and the HUD strip begins.
const HUD_Y: i32 = 158;

/// Uniform block for the shader.  Must be 16-byte aligned (WGSL uniform rules).
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct ShaderOpts {
    scanlines: u32,
    _pad: [u32; 3],
}

// ── Viewport ──────────────────────────────────────────────────────────────────

/// One split-screen viewport.
struct Viewport {
    /// Worm index this viewport tracks.
    worm_idx: usize,
    /// Level coordinates of the top-left corner visible in this viewport (current, smoothed).
    cam_x: i32,
    cam_y: i32,
    /// Smoothing target (where the camera wants to be, before lerp).
    target_cam_x: i32,
    target_cam_y: i32,
    /// Screen rectangle `[x0, y0, x1, y1)` (exclusive end).
    rect: [i32; 4],
}

impl Viewport {
    fn w(&self) -> i32 { self.rect[2] - self.rect[0] }
    fn h(&self) -> i32 { self.rect[3] - self.rect[1] }
    /// X offset: add to a level X to get screen X within this viewport.
    fn off_x(&self) -> i32 { self.rect[0] - self.cam_x }
    /// Y offset: add to a level Y to get screen Y within this viewport.
    fn off_y(&self) -> i32 { self.rect[1] - self.cam_y }
}

/// Two-player split-screen layout: `[screen_rect, worm_idx]`.
const VP_LAYOUT_2P: [([i32; 4], usize); 2] = [
    ([0,   0, 158, HUD_Y], 0),
    ([160, 0, 318, HUD_Y], 1),
];

// ── Renderer ──────────────────────────────────────────────────────────────────

/// GPU renderer. Create with [`Renderer::new`], call [`Renderer::render`] each frame.
pub struct Renderer {
    surface:    wgpu::Surface<'static>,
    device:     wgpu::Device,
    queue:      wgpu::Queue,
    config:     wgpu::SurfaceConfiguration,
    pipeline:   wgpu::RenderPipeline,
    frame_tex:  wgpu::Texture,
    #[allow(dead_code)]
    lut_tex:    wgpu::Texture,  // kept alive to prevent GPU resource drop
    opts_buf:   wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// CPU compositing buffer: 320×200 palette indices.
    frame_buf:  Vec<u8>,
    /// Active viewports (updated each frame by `update_cameras`).
    viewports:  Vec<Viewport>,
    /// CRT scanlines toggle (passed to shader as uniform).
    pub crt_scanlines: bool,
}

impl Renderer {
    /// Initialise wgpu and create all GPU resources.
    ///
    /// `window` must outlive the renderer (tied via `'static` surface).
    pub async fn new(window: Arc<Window>, tc: &Tc) -> Result<Self> {
        let size = window.inner_size();

        // ── Instance / Surface ─────────────────────────────────────────────
        // On macOS use Metal explicitly — avoids spurious failures when wgpu
        // tries Vulkan/DX12 backends that aren't available on this platform.
        #[cfg(target_os = "macos")]
        let backends = wgpu::Backends::METAL;
        #[cfg(not(target_os = "macos"))]
        let backends = wgpu::Backends::all();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::default(),
            backend_options: Default::default(),
            display: Default::default(),
            memory_budget_thresholds: Default::default(),
        });
        // SAFETY: window lives as long as the surface because we hold Arc<Window>.
        let surface = instance.create_surface(Arc::clone(&window))?;

        // ── Adapter / Device / Queue ───────────────────────────────────────
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference:       wgpu::PowerPreference::HighPerformance,
                compatible_surface:     Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no suitable wgpu adapter found")?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label:    Some("liero-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits:   wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits()),
                    ..Default::default()
                },
            )
            .await
            .context("failed to create wgpu device")?;

        // ── Surface configuration ──────────────────────────────────────────
        let caps   = surface.get_capabilities(&adapter);
        let format = caps.formats.iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage:        wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width:        size.width.max(1),
            height:       size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode:   caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // ── Frame texture: 320×200 R8Unorm ─────────────────────────────────
        // R8Unorm is used instead of R8Uint for WebGL2 compatibility:
        // integer texture sampling (usampler2D) is unreliable in WebGL2 and
        // returns 0 for every pixel on many browsers.  R8Unorm maps each
        // palette index byte as float/255.0; the shader recovers the index
        // with round(sample.r * 255).
        let frame_tex = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("frame-tex"),
            size:            wgpu::Extent3d { width: RENDER_W, height: RENDER_H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::R8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        let frame_view = frame_tex.create_view(&Default::default());

        // ── LUT texture: 256×1 Rgba8Unorm ─────────────────────────────────
        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label:           Some("lut-tex"),
            size:            wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count:    1,
            dimension:       wgpu::TextureDimension::D2,
            format:          wgpu::TextureFormat::Rgba8Unorm,
            usage:           wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats:    &[],
        });
        let lut_view = lut_tex.create_view(&Default::default());

        // Upload palette immediately.
        let palette_rgba: Vec<u8> = tc.palette_rgba().iter()
            .flat_map(|&[r, g, b, a]| [r, g, b, a])
            .collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture:   &lut_tex,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            &palette_rgba,
            wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(256 * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
        );

        // ── Uniform buffer ─────────────────────────────────────────────────
        let opts_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("opts-buf"),
            contents: bytemuck::bytes_of(&ShaderOpts { scanlines: 0, _pad: [0; 3] }),
            usage:    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Shader ─────────────────────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        // ── Bind group layout ──────────────────────────────────────────────
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("bgl"),
            entries: &[
                // binding 0: frame_tex (texture_2d<f32>, R8Unorm — WebGL2-compatible)
                wgpu::BindGroupLayoutEntry {
                    binding:    0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                // binding 1: lut_tex (texture_2d<f32>)
                wgpu::BindGroupLayoutEntry {
                    binding:    1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type:    wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled:   false,
                    },
                    count: None,
                },
                // binding 2: opts (uniform buffer)
                wgpu::BindGroupLayoutEntry {
                    binding:    2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty:                 wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size:   None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("bind-group"),
            layout:  &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&frame_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&lut_view) },
                wgpu::BindGroupEntry { binding: 2, resource: opts_buf.as_entire_binding() },
            ],
        });

        // ── Render pipeline ────────────────────────────────────────────────
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label:              Some("pipeline-layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size:     0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label:  Some("pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module:              &shader,
                entry_point:         Some("vs_main"),
                buffers:             &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module:              &shader,
                entry_point:         Some("fs_main"),
                targets:             &[Some(wgpu::ColorTargetState {
                    format,
                    blend:      Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology:           wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face:         wgpu::FrontFace::Ccw,
                cull_mode:          None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample:   wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache:         None,
        });

        // ── Initial viewports (2-player layout) ───────────────────────────
        let viewports = VP_LAYOUT_2P.iter()
            .map(|&(rect, worm_idx)| Viewport {
                worm_idx, cam_x: 0, cam_y: 0,
                target_cam_x: 0, target_cam_y: 0, rect,
            })
            .collect();

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            frame_tex,
            lut_tex,
            opts_buf,
            bind_group,
            frame_buf: vec![0u8; (RENDER_W * RENDER_H) as usize],
            viewports,
            crt_scanlines: false,
        })
    }

    /// Handle window resize.
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 { return; }
        self.config.width  = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Enable or disable CRT scanline effect.
    pub fn set_scanlines(&mut self, on: bool) {
        self.crt_scanlines = on;
    }

    /// Composite the game state into the CPU buffer, upload to GPU, and present.
    pub fn render(&mut self, game: &Game) {
        self.update_cameras(game);
        self.compose_frame(game);
        let scanlines = if self.crt_scanlines { 1u32 } else { 0 };
        self.gpu_present(scanlines);
    }

    // ── GPU upload + present (shared by game render and menu render) ──────────

    fn gpu_present(&mut self, scanlines: u32) {
        let opts = ShaderOpts { scanlines, _pad: [0; 3] };
        self.queue.write_buffer(&self.opts_buf, 0, bytemuck::bytes_of(&opts));

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture:   &self.frame_tex,
                mip_level: 0,
                origin:    wgpu::Origin3d::ZERO,
                aspect:    wgpu::TextureAspect::All,
            },
            &self.frame_buf,
            wgpu::TexelCopyBufferLayout {
                offset:         0,
                bytes_per_row:  Some(RENDER_W),
                rows_per_image: None,
            },
            wgpu::Extent3d { width: RENDER_W, height: RENDER_H, depth_or_array_layers: 1 },
        );

        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex) | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            _ => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
        };
        let view = output.texture.create_view(&Default::default());

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame-encoder"),
        });
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           &view,
                    resolve_target: None,
                    depth_slice:    None,
                    ops: wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes:         None,
                occlusion_query_set:      None,
                multiview_mask:           None,
            });
            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.draw(0..4, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
    }

    // ── Pixel / text drawing into frame_buf ───────────────────────────────────

    #[inline]
    fn fb_put(&mut self, x: i32, y: i32, color: u8) {
        if x < 0 || y < 0 || x >= RENDER_W as i32 || y >= RENDER_H as i32 { return; }
        self.frame_buf[(y as u32 * RENDER_W + x as u32) as usize] = color;
    }

    fn fb_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u8) {
        for py in y..y + h {
            for px in x..x + w {
                self.fb_put(px, py, color);
            }
        }
    }

    /// Draw a single 8×8 glyph.  `font8x8` stores bit 0 as the leftmost pixel.
    fn fb_glyph(&mut self, c: char, x: i32, y: i32, color: u8) {
        let bits = font8x8::BASIC_FONTS.get(c)
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

    /// Draw a string; each character is 9 px wide (8 + 1 gap).
    fn fb_text(&mut self, s: &str, x: i32, y: i32, color: u8) {
        for (i, c) in s.chars().enumerate() {
            self.fb_glyph(c, x + i as i32 * 9, y, color);
        }
    }

    /// Draw a string centred horizontally on the 320-wide screen.
    fn fb_text_center(&mut self, s: &str, y: i32, color: u8) {
        let w = s.len() as i32 * 9;
        let x = ((RENDER_W as i32) - w) / 2;
        self.fb_text(s, x, y, color);
    }

    // ── Menu / game-over screens ──────────────────────────────────────────────

    /// Render a menu screen.
    ///
    /// * `title`   — screen title (displayed at top, highlighted)
    /// * `options` — list of selectable items
    /// * `cursor`  — index of the currently highlighted item
    /// * `extra`   — informational lines shown at the bottom (greyed out)
    pub fn render_menu(
        &mut self,
        title:   &str,
        options: &[&str],
        cursor:  usize,
        extra:   &[&str],
    ) {
        // Palette colour constants (tuned for typical Liero TCs).
        // Index 0  = black BG   · 1 = near-black panel
        // 7  = mid-gray normal  · 14 = yellow selection
        // 15 = white title/bold · 4  = dark-gray hint
        const C_BG:  u8 = 0;
        const C_PNL: u8 = 1;
        const C_NRM: u8 = 7;
        const C_SEL: u8 = 15;
        const C_TTL: u8 = 14;
        const C_HNT: u8 = 4;

        self.frame_buf.fill(C_BG);

        // Title bar.
        self.fb_rect(0, 0, RENDER_W as i32, 18, C_PNL);
        self.fb_text_center(title, 5, C_TTL);

        // Divider.
        self.fb_rect(0, 18, RENDER_W as i32, 1, C_NRM);

        // Options.
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

        // Extra / hint lines.
        if !extra.is_empty() {
            let hint_y = RENDER_H as i32 - 10 - extra.len() as i32 * 10;
            self.fb_rect(0, hint_y - 4, RENDER_W as i32, 1, C_PNL);
            for (i, &line) in extra.iter().enumerate() {
                self.fb_text_center(line, hint_y + i as i32 * 10, C_HNT);
            }
        }

        self.gpu_present(0);
    }

    /// Render the game-over / victory screen.
    pub fn render_game_over(&mut self, msg: &str) {
        const C_BG:  u8 = 0;
        const C_PNL: u8 = 1;
        const C_MSG: u8 = 15;
        const C_HNT: u8 = 7;

        self.frame_buf.fill(C_BG);

        // Central panel.
        let panel_y = RENDER_H as i32 / 2 - 20;
        self.fb_rect(20, panel_y, RENDER_W as i32 - 40, 44, C_PNL);
        self.fb_rect(20, panel_y, RENDER_W as i32 - 40, 1, C_MSG);
        self.fb_rect(20, panel_y + 43, RENDER_W as i32 - 40, 1, C_MSG);

        self.fb_text_center(msg, panel_y + 10, C_MSG);
        self.fb_text_center("PRESS ENTER TO RETURN", panel_y + 28, C_HNT);

        self.gpu_present(0);
    }

    // ── Private ─────────────────────────────────────────────────────────────

    /// Move each viewport's camera toward its tracked worm with a lerp (~12.5%/frame).
    fn update_cameras(&mut self, game: &Game) {
        const LERP_DIV: i32 = 8; // move 1/8 of remaining distance per frame
        for vp in &mut self.viewports {
            let wi = vp.worm_idx;
            if wi >= game.worms.len() { continue; }
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
            // Snap when 1 px away (avoids infinite 1-px jitter), lerp otherwise.
            vp.cam_x += if dx.abs() <= 1 { dx } else { dx / LERP_DIV };
            vp.cam_y += if dy.abs() <= 1 { dy } else { dy / LERP_DIV };
        }
    }

    /// Composite one frame into `self.frame_buf` (palette indices, CPU-side).
    fn compose_frame(&mut self, game: &Game) {
        // Clear to index 0 (background colour).
        self.frame_buf.fill(0);

        // Collect viewport data we need (avoid borrow conflict below).
        let vp_count = self.viewports.len().min(game.worms.len() + 1);

        for vi in 0..vp_count {
            let (rect, ox, oy) = {
                let vp = &self.viewports[vi];
                (vp.rect, vp.off_x(), vp.off_y())
            };

            // a. Terrain.
            for sy in rect[1]..rect[3] {
                for sx in rect[0]..rect[2] {
                    let lx = sx - ox;
                    let ly = sy - oy;
                    let idx = game.level.pixel(lx, ly);
                    self.frame_buf[(sy as u32 * RENDER_W + sx as u32) as usize] = idx;
                }
            }

            // b. Wobjects.
            for wob in &game.wobjects {
                let weapon = &game.tc.weapons[wob.weapon_idx];
                let px = (wob.pos.x.0 >> 16) + ox;
                let py = (wob.pos.y.0 >> 16) + oy;
                if weapon.start_frame >= 0 {
                    let frame = (weapon.start_frame as i32 + wob.cur_frame) as usize;
                    let frame = frame.min(game.tc.small_sprites.len() / SMALL_SPRITE_SIZE - 1);
                    blit_small(
                        &mut self.frame_buf, RENDER_W, rect,
                        &game.tc.small_sprites, frame, px, py,
                    );
                } else if wob.cur_frame > 0 {
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, px, py, wob.cur_frame as u8);
                }
            }

            // c. Nobjects.
            for nob in &game.nobjects {
                let ntype = &game.tc.nobjects[nob.nobj_type];
                let px = (nob.pos.x.0 >> 16) + ox;
                let py = (nob.pos.y.0 >> 16) + oy;
                if ntype.start_frame > 0 {
                    let frame = (ntype.start_frame as i32 + nob.cur_frame) as usize;
                    let frame = frame.min(game.tc.small_sprites.len() / SMALL_SPRITE_SIZE - 1);
                    blit_small(
                        &mut self.frame_buf, RENDER_W, rect,
                        &game.tc.small_sprites, frame, px, py,
                    );
                } else if nob.cur_frame > 0 {
                    put_pixel(&mut self.frame_buf, RENDER_W, rect, px, py, nob.cur_frame as u8);
                }
            }

            // d. Worms (16×16 pre-remapped sprites).
            for worm in &game.worms {
                if !worm.visible { continue; }
                let wx = (worm.pos.x.0 >> 16) + ox;
                let wy = (worm.pos.y.0 >> 16) + oy;
                // C++ draws worm sprite centred at ≈ (pos.x, pos.y-2); offset by -7,-5.
                let px = wx - 7;
                let py = wy - 5;
                let slot  = worm.index & 3;
                let dir   = worm.direction.clamp(0, 1) as usize;
                let frame = worm.cur_frame.clamp(0, 20) as usize;
                blit_large(
                    &mut self.frame_buf, RENDER_W, rect,
                    &game.tc.worm_sprites, frame + dir * 21 + slot * 42, px, py,
                );
            }
        }

        // e. HUD health bars (palette-index bars in the HUD strip).
        self.draw_hud(game);
    }

    /// Draw basic health bars in the HUD strip (y = HUD_Y … RENDER_H).
    fn draw_hud(&mut self, game: &Game) {
        const BAR_H: i32 = 4;
        const BAR_TOP: i32 = HUD_Y + 3;
        const FULL_BAR_W: i32 = 80;
        // Palette indices for each worm's health bar.
        const WORM_BAR_COLORS: [u8; 4] = [200, 210, 220, 230];

        let clip = [0i32, HUD_Y, RENDER_W as i32, RENDER_H as i32];

        for (i, worm) in game.worms.iter().enumerate() {
            let bar_x = (i as i32) * 160; // 160px per player column
            let health_w = (worm.health.max(0) * FULL_BAR_W / 100).min(FULL_BAR_W);
            let color = WORM_BAR_COLORS[i & 3];
            for px in bar_x .. bar_x + health_w {
                for py in BAR_TOP .. BAR_TOP + BAR_H {
                    put_pixel(&mut self.frame_buf, RENDER_W, clip, px, py, color);
                }
            }
        }
    }
}

// ── Sprite blit helpers ───────────────────────────────────────────────────────

/// Write a single pixel at `(x, y)` (screen coords), clipped to `clip`.
/// `clip = [x0, y0, x1, y1)` (exclusive end).
#[inline(always)]
fn put_pixel(buf: &mut [u8], buf_w: u32, clip: [i32; 4], x: i32, y: i32, color: u8) {
    if x < clip[0] || y < clip[1] || x >= clip[2] || y >= clip[3] { return; }
    buf[(y as u32 * buf_w + x as u32) as usize] = color;
}

/// Blit a 7×7 small sprite, colour 0 transparent, clipped to `clip`.
fn blit_small(
    buf: &mut [u8], buf_w: u32, clip: [i32; 4],
    sprites: &[u8], frame: usize, x: i32, y: i32,
) {
    let start = frame * SMALL_SPRITE_SIZE;
    if start + SMALL_SPRITE_SIZE > sprites.len() { return; }
    let src = &sprites[start .. start + SMALL_SPRITE_SIZE];
    for row in 0..SMALL_SPRITE_H as i32 {
        for col in 0..SMALL_SPRITE_W as i32 {
            let px = x + col;
            let py = y + row;
            let c  = src[(row as usize) * SMALL_SPRITE_W + col as usize];
            if c != 0 { put_pixel(buf, buf_w, clip, px, py, c); }
        }
    }
}

/// Blit a 16×16 large sprite, colour 0 transparent, clipped to `clip`.
fn blit_large(
    buf: &mut [u8], buf_w: u32, clip: [i32; 4],
    sprites: &[u8], frame: usize, x: i32, y: i32,
) {
    let start = frame * SPRITE_SIZE;
    if start + SPRITE_SIZE > sprites.len() { return; }
    let src = &sprites[start .. start + SPRITE_SIZE];
    for row in 0..SPRITE_H as i32 {
        for col in 0..SPRITE_W as i32 {
            let px = x + col;
            let py = y + row;
            let c  = src[(row as usize) * SPRITE_W + col as usize];
            if c != 0 { put_pixel(buf, buf_w, clip, px, py, c); }
        }
    }
}
