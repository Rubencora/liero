//! `liero-sim` — deterministic fixed-point simulation core.
//!
//! Zero renderer dependencies. Zero I/O. Pure logic.
//! All arithmetic uses wrapping integer ops to guarantee cross-platform
//! bit-exactness (matches the C++ fixed-point physics).
//!
//! # Module layout
//! - [`fixed`]    — Q16.16 fixed-point scalar and 2D vector
//! - [`math`]     — cossin table (128-entry), integer sqrt, vector_length
//! - [`rand`]     — MWC PRNG (bit-exact port of `gvl::mwc`)
//! - [`material`] — terrain material bitflags
//! - [`level`]    — terrain pixel map + material lookup
//! - [`worm`]     — player entity state
//! - [`wobject`]  — active projectile state
//! - [`nobject`]  — non-owner particle state
//! - [`game`]     — top-level simulation state + `step()` + `checksum()`

pub mod fixed;
pub mod math;
pub mod rand;
pub mod material;
pub mod level;
pub mod levelgen;
pub mod worm;
pub mod wobject;
pub mod nobject;
pub mod sobject;
pub mod game;
