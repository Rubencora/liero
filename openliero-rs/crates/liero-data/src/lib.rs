//! `liero-data` — TC (TournamentConfig) data loaders.
//!
//! Loads weapon, nobject, sobject, and top-level TC config files from a
//! directory using serde + toml.  All types mirror the C++ `Common` structs
//! field-for-field so that `liero-sim` can use them directly.

pub mod weapon;
pub mod nobject;
pub mod sobject;
pub mod tc;
pub(crate) mod util;

pub use tc::TcData;
pub use weapon::Weapon;
pub use nobject::NObjectType;
pub use sobject::SObjectType;

use anyhow::{Context, Result};
use std::path::Path;

/// A single decoded audio sample (mono f32 PCM at 22050 Hz).
pub type SoundSamples = Vec<f32>;

/// Fully loaded TC directory — all weapons, nobjects, sobjects + constants.
#[derive(Clone)]
pub struct Tc {
    pub data:          TcData,
    pub weapons:       Vec<Weapon>,
    pub nobjects:      Vec<NObjectType>,
    pub sobjects:      Vec<SObjectType>,
    /// Decoded audio samples, one `Vec<f32>` per sound in `data.types.sounds`.
    /// 8-bit unsigned PCM (0..=255) normalised to `f32` in the range −1.0..1.0.
    /// Sample rate: 22050 Hz, mono.  Index matches `data.types.sounds`.
    pub sounds:        Vec<SoundSamples>,
    /// Large sprite sheet pixel data (palette indices, 8-bit).
    ///
    /// Layout: `large_sprites[frame * SPRITE_SIZE + row * SPRITE_W + col]`
    /// where `SPRITE_W = SPRITE_H = 16` and `SPRITE_SIZE = 256`.
    /// Frame 0 is the first sprite at the top of `sprites/large.tga`.
    /// Mirrors C++ `Common::largeSprites` (SpriteSet with width=16, height=16).
    pub large_sprites: Vec<u8>,
    /// Small sprite sheet pixel data (palette indices, 8-bit).
    ///
    /// Layout: `small_sprites[frame * SMALL_SPRITE_SIZE + row * SMALL_SPRITE_W + col]`
    /// where `SMALL_SPRITE_W = SMALL_SPRITE_H = 7` and `SMALL_SPRITE_SIZE = 49`.
    /// 130 frames total (7×910 TGA).
    /// Mirrors C++ `Common::smallSprites`.
    pub small_sprites: Vec<u8>,
    /// Pre-generated worm sprites for all 4 slots × 2 directions × 21 frames = 168 sprites.
    ///
    /// Each frame is 16×16 (256 bytes). Layout:
    /// `worm_sprites[(frame + dir * 21 + slot * 42) * SPRITE_SIZE + row * SPRITE_W + col]`
    /// - dir=0: left-facing (mirrored), dir=1: right-facing
    /// - slot 0-3: each slot has a distinct palette remap for worm color
    pub worm_sprites:  Vec<u8>,
    /// Raw palette data extracted from `sprites/large.tga` colormap.
    /// 768 bytes: 256 entries × 3 bytes each, stored as BGR (TGA convention).
    pub palette: Vec<u8>,
}

pub const SPRITE_W:         usize = 16;
pub const SPRITE_H:         usize = 16;
pub const SPRITE_SIZE:      usize = SPRITE_W * SPRITE_H; // 256 bytes per frame

pub const SMALL_SPRITE_W:    usize = 7;
pub const SMALL_SPRITE_H:    usize = 7;
pub const SMALL_SPRITE_SIZE: usize = SMALL_SPRITE_W * SMALL_SPRITE_H; // 49 bytes per frame

impl Tc {
    /// Load a TC from a directory using a reader closure.
    ///
    /// `reader(relative_path)` must return the raw bytes for that file within
    /// the TC directory.  On native targets, use `Tc::load(dir)` which wraps
    /// this with `std::fs::read`.  On WASM, use `Tc::load_with` with a closure
    /// that reads from an embedded `include_dir::Dir`.
    pub fn load_with<F>(reader: F) -> Result<Self>
    where
        F: Fn(&str) -> Result<Vec<u8>>,
    {
        let data = TcData::from_bytes(&reader("tc.cfg")?)?;

        let weapons = data.types.weapons.iter()
            .map(|name| {
                let raw = reader(&format!("weapons/{name}.cfg"))?;
                Weapon::from_bytes(&raw).with_context(|| format!("weapon {name}"))
            })
            .collect::<Result<Vec<_>>>()?;

        let nobjects = data.types.nobjects.iter()
            .map(|name| {
                let raw = reader(&format!("nobjects/{name}.cfg"))?;
                NObjectType::from_bytes(&raw).with_context(|| format!("nobject {name}"))
            })
            .collect::<Result<Vec<_>>>()?;

        let sobjects = data.types.sobjects.iter()
            .map(|name| {
                let raw = reader(&format!("sobjects/{name}.cfg"))?;
                SObjectType::from_bytes(&raw).with_context(|| format!("sobject {name}"))
            })
            .collect::<Result<Vec<_>>>()?;

        let (large_sprites, palette) = parse_large_sprites(&reader("sprites/large.tga")?)?;
        let small_sprites = parse_small_sprites(&reader("sprites/small.tga")?)?;
        let worm_sprites  = generate_worm_sprites(&large_sprites);

        let sounds = data.types.sounds.iter()
            .map(|name| {
                let path = format!("sounds/{}.wav", name.to_uppercase());
                reader(&path).ok()
                    .and_then(|raw| parse_wav_f32(&raw).ok())
                    .unwrap_or_default()
            })
            .collect();

        Ok(Self { data, weapons, nobjects, sobjects, large_sprites, small_sprites, worm_sprites, palette, sounds })
    }

    /// Load a TC from `dir` (e.g. `TC/openliero`).
    pub fn load(dir: &Path) -> Result<Self> {
        Self::load_with(|rel_path| {
            let p = dir.join(rel_path);
            std::fs::read(&p).with_context(|| format!("reading {}", p.display()))
        })
    }

    /// Return the pixel data for large sprite frame `n` (16×16, top-to-bottom).
    #[inline]
    pub fn large_sprite(&self, frame: usize) -> &[u8] {
        let start = frame * SPRITE_SIZE;
        &self.large_sprites[start .. start + SPRITE_SIZE]
    }

    /// Return the pixel data for small sprite frame `n` (7×7, top-to-bottom).
    #[inline]
    pub fn small_sprite(&self, frame: usize) -> &[u8] {
        let start = frame * SMALL_SPRITE_SIZE;
        &self.small_sprites[start .. start + SMALL_SPRITE_SIZE]
    }

    /// Return the pre-generated worm sprite for `(worm_slot, dir, frame)` (16×16).
    /// - `worm_slot`: 0-3
    /// - `dir`: 0=left, 1=right
    /// - `frame`: 0-20 (angle + walk animation frame)
    #[inline]
    pub fn worm_sprite(&self, worm_slot: usize, dir: usize, frame: usize) -> &[u8] {
        let idx   = frame + dir * 21 + worm_slot * 42;
        let start = idx * SPRITE_SIZE;
        &self.worm_sprites[start .. start + SPRITE_SIZE]
    }

    /// Return the palette as 256 RGBA8 entries (converted from BGR TGA storage).
    pub fn palette_rgba(&self) -> Vec<[u8; 4]> {
        self.palette
            .chunks(3)
            .map(|bgr| [bgr[2], bgr[1], bgr[0], 255])
            .collect()
    }
}

/// Parse `sprites/large.tga` bytes and return `(pixels, palette)`.
///
/// - `pixels`: palette indices, top-to-bottom row order (rows reversed from TGA).
/// - `palette`: raw colormap bytes, 256 × 3 bytes BGR (TGA convention).
///
/// The TGA file stores rows bottom-to-top (standard TGA, no flip flag).
/// This function reverses the rows so the result is top-to-bottom, matching
/// C++ `readSpriteTga` which also performs the row reversal.
pub fn parse_large_sprites(raw: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    // TGA header (18 bytes):
    //   byte 0  = ID length
    //   byte 1  = color map type (must be 1 for palette)
    //   byte 2  = image type (must be 1 = color-mapped)
    //   bytes 3-7 = color map spec: first=0, length=256, bpp=24
    //   bytes 8-9 = x-origin (ignored)
    //   bytes 10-11 = y-origin (ignored)
    //   bytes 12-13 = image width
    //   bytes 14-15 = image height
    //   byte 16  = pixel depth (must be 8)
    //   byte 17  = image descriptor (bit 5: 0 = bottom-to-top)
    anyhow::ensure!(raw.len() >= 18, "large.tga too short for header");

    let id_len    = raw[0] as usize;
    let cm_type   = raw[1];
    let img_type  = raw[2];
    let cm_len    = u16::from_le_bytes([raw[5], raw[6]]) as usize;
    let cm_bpp    = raw[7] as usize;
    let img_w     = u16::from_le_bytes([raw[12], raw[13]]) as usize;
    let img_h     = u16::from_le_bytes([raw[14], raw[15]]) as usize;
    let pixel_bpp = raw[16];
    let img_desc  = raw[17];

    anyhow::ensure!(cm_type  == 1, "large.tga: expected color-mapped (cm_type=1)");
    anyhow::ensure!(img_type == 1, "large.tga: expected uncompressed color-mapped (img_type=1)");
    anyhow::ensure!(pixel_bpp == 8, "large.tga: expected 8-bit indexed pixels");
    anyhow::ensure!(img_w == SPRITE_W, "large.tga: expected width={SPRITE_W}");
    anyhow::ensure!(img_h % SPRITE_H == 0, "large.tga: height not multiple of {SPRITE_H}");

    let n_frames = img_h / SPRITE_H;
    let cm_bytes = cm_len * (cm_bpp / 8);
    let palette_offset = 18 + id_len;
    let pixel_offset   = palette_offset + cm_bytes;
    anyhow::ensure!(
        raw.len() >= pixel_offset + img_w * img_h,
        "large.tga: file too short for pixel data"
    );

    // Extract palette (256 × 3 bytes BGR).
    let palette = raw[palette_offset .. pixel_offset].to_vec();

    let file_pixels = &raw[pixel_offset .. pixel_offset + img_w * img_h];

    // TGA rows are stored bottom-to-top unless bit 5 of img_desc is set.
    let top_to_bottom = (img_desc >> 5) & 1 != 0;

    // Output: n_frames × 256 bytes, rows top-to-bottom.
    let mut out = vec![0u8; n_frames * SPRITE_SIZE];

    for row in 0..img_h {
        // File-order row → in-memory row (flip if bottom-to-top TGA).
        let mem_row = if top_to_bottom { row } else { img_h - 1 - row };
        let file_slice = &file_pixels[row * img_w .. (row + 1) * img_w];
        let out_slice  = &mut out[mem_row * img_w .. (mem_row + 1) * img_w];
        out_slice.copy_from_slice(file_slice);
    }

    Ok((out, palette))
}

/// Parse `sprites/small.tga` bytes and return raw palette-index pixels (7×7 per frame).
///
/// small.tga is a 7×910 image (130 frames × 7px height each).
/// Rows are stored bottom-to-top (standard TGA); this function reverses them.
pub fn parse_small_sprites(raw: &[u8]) -> Result<Vec<u8>> {
    anyhow::ensure!(raw.len() >= 18, "small.tga too short for header");

    let id_len    = raw[0] as usize;
    let cm_type   = raw[1];
    let img_type  = raw[2];
    let cm_len    = u16::from_le_bytes([raw[5], raw[6]]) as usize;
    let cm_bpp    = raw[7] as usize;
    let img_w     = u16::from_le_bytes([raw[12], raw[13]]) as usize;
    let img_h     = u16::from_le_bytes([raw[14], raw[15]]) as usize;
    let pixel_bpp = raw[16];
    let img_desc  = raw[17];

    anyhow::ensure!(cm_type  == 1, "small.tga: expected color-mapped (cm_type=1)");
    anyhow::ensure!(img_type == 1, "small.tga: expected uncompressed color-mapped (img_type=1)");
    anyhow::ensure!(pixel_bpp == 8, "small.tga: expected 8-bit indexed pixels");
    anyhow::ensure!(img_w == SMALL_SPRITE_W, "small.tga: expected width={SMALL_SPRITE_W}");
    anyhow::ensure!(img_h % SMALL_SPRITE_H == 0, "small.tga: height not multiple of {SMALL_SPRITE_H}");

    let n_frames  = img_h / SMALL_SPRITE_H;
    let cm_bytes  = cm_len * (cm_bpp / 8);
    let pal_off   = 18 + id_len;
    let pix_off   = pal_off + cm_bytes;
    anyhow::ensure!(
        raw.len() >= pix_off + img_w * img_h,
        "small.tga: file too short for pixel data"
    );

    let file_pixels    = &raw[pix_off .. pix_off + img_w * img_h];
    let top_to_bottom  = (img_desc >> 5) & 1 != 0;
    let mut out        = vec![0u8; n_frames * SMALL_SPRITE_SIZE];

    for row in 0..img_h {
        let mem_row    = if top_to_bottom { row } else { img_h - 1 - row };
        let file_slice = &file_pixels[row * img_w .. (row + 1) * img_w];
        let out_slice  = &mut out[mem_row * img_w .. (mem_row + 1) * img_w];
        out_slice.copy_from_slice(file_slice);
    }

    Ok(out)
}

/// Generate worm sprites for all 4 slots × 2 directions × 21 frames from large_sprites.
///
/// Source: large.tga frames 16-36 (21 frames of 16×16).
/// For each slot (0-3):
///   For each direction (0=left, 1=right):
///     For each frame (0-20):
///       - Copy source frame (16+f) from large_sprites
///       - Remap palette indices 30-34 → 30 + 9*slot (per-worm colour)
///       - If dir=0 (left-facing): flip horizontally (mirror X)
///       Store at position: frame + dir*21 + slot*42
fn generate_worm_sprites(large_sprites: &[u8]) -> Vec<u8> {
    const WORM_ANIM_FRAMES: usize = 21;
    const WORM_SOURCE_START: usize = 16;   // frames 16..36 in large.tga
    const TOTAL_FRAMES: usize = 4 * 2 * WORM_ANIM_FRAMES; // 168

    let mut out = vec![0u8; TOTAL_FRAMES * SPRITE_SIZE];

    for slot in 0..4usize {
        for dir in 0..2usize {
            for f in 0..WORM_ANIM_FRAMES {
                let src_frame = WORM_SOURCE_START + f;
                let src_start = src_frame * SPRITE_SIZE;
                let src       = &large_sprites[src_start .. src_start + SPRITE_SIZE];

                let dst_idx   = f + dir * WORM_ANIM_FRAMES + slot * 42;
                let dst_start = dst_idx * SPRITE_SIZE;
                let dst       = &mut out[dst_start .. dst_start + SPRITE_SIZE];

                // Copy and remap palette: indices 30-34 → 30 + 9*slot.
                for row in 0..SPRITE_H {
                    for col in 0..SPRITE_W {
                        let src_col = if dir == 0 { SPRITE_W - 1 - col } else { col };
                        let mut px  = src[row * SPRITE_W + src_col];
                        if px >= 30 && px <= 34 {
                            px = 30 + (9 * slot) as u8 + (px - 30);
                        }
                        dst[row * SPRITE_W + col] = px;
                    }
                }
            }
        }
    }

    out
}

/// Parse WAV bytes into mono f32 samples.
///
/// Handles 8-bit unsigned and 16-bit signed PCM, mono or stereo (stereo mixed to mono).
/// Returns an error on any parse failure (callers wrap with `unwrap_or_default()`).
pub fn parse_wav_f32(raw: &[u8]) -> Result<SoundSamples> {
    // RIFF header: "RIFF", file_size (4), "WAVE" (4).
    anyhow::ensure!(raw.len() >= 12, "WAV too short for RIFF header");
    anyhow::ensure!(&raw[0..4] == b"RIFF", "not a RIFF file");
    anyhow::ensure!(&raw[8..12] == b"WAVE", "not a WAVE file");

    // Walk chunks until we find "fmt " and "data".
    let mut pos = 12usize;
    let mut channels: u16 = 1;
    let mut bits_per_sample: u16 = 8;
    let mut data_slice: &[u8] = &[];
    let mut found_fmt = false;

    while pos + 8 <= raw.len() {
        let tag  = &raw[pos..pos+4];
        let size = u32::from_le_bytes(raw[pos+4..pos+8].try_into().unwrap()) as usize;
        pos += 8;
        let end = pos + size;

        if tag == b"fmt " && size >= 16 {
            // PCM format chunk.
            let _audio_format = u16::from_le_bytes(raw[pos..pos+2].try_into().unwrap());
            channels       = u16::from_le_bytes(raw[pos+2..pos+4].try_into().unwrap());
            // frame_rate     = u32 at pos+4 (ignored — assumed 22050)
            bits_per_sample = u16::from_le_bytes(raw[pos+14..pos+16].try_into().unwrap());
            found_fmt = true;
        } else if tag == b"data" {
            data_slice = &raw[pos .. pos + size.min(raw.len() - pos)];
        }

        // Chunks are 2-byte aligned.
        pos = end + (size & 1);
    }

    anyhow::ensure!(found_fmt, "WAV missing fmt chunk");

    // Decode samples to f32 (mix to mono if stereo).
    let samples: Vec<f32> = match bits_per_sample {
        8 => {
            // 8-bit unsigned: 128 → 0.0, 0 → -1.0, 255 → ~1.0.
            if channels == 1 {
                data_slice.iter().map(|&b| (b as f32 - 128.0) / 128.0).collect()
            } else {
                // Stereo → mono: average pairs.
                data_slice.chunks(channels as usize)
                    .map(|ch| ch.iter().map(|&b| (b as f32 - 128.0) / 128.0).sum::<f32>() / channels as f32)
                    .collect()
            }
        }
        16 => {
            // 16-bit signed little-endian.
            if channels == 1 {
                data_slice.chunks(2).filter_map(|ch| {
                    ch.try_into().ok().map(|b: [u8;2]| i16::from_le_bytes(b) as f32 / 32768.0)
                }).collect()
            } else {
                let ch_count = channels as usize;
                data_slice.chunks(ch_count * 2).map(|frame| {
                    let sum: f32 = (0..ch_count).map(|c| {
                        let b = &frame[c*2 .. c*2+2];
                        i16::from_le_bytes(b.try_into().unwrap_or([0,0])) as f32 / 32768.0
                    }).sum();
                    sum / ch_count as f32
                }).collect()
            }
        }
        _ => anyhow::bail!("unsupported bits_per_sample: {bits_per_sample}"),
    };

    Ok(samples)
}
