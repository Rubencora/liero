//! Material bitflags — exact port of `src/game/material.hpp`.
//!
//! Each terrain pixel stores a palette index.  The palette maps index → Material.
//! A `Material` is a bitmask with the flags below (matching the C++ `Material` enum).

/// Terrain material bitmask (mirrors C++ `Material` in `src/game/material.hpp`).
///
/// Bit layout:
/// ```text
/// bit 0  = Dirt       (destructible dirt)
/// bit 1  = Dirt2      (secondary dirt type)
/// bit 2  = Rock       (indestructible rock)
/// bit 3  = Background (empty / void)
/// bit 4  = SeeShadow  (shadow cast on this pixel)
/// bit 5  = WormM      (worm sprite pixel — collision only)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Material(pub u8);

impl Material {
    pub const NONE:       Self = Self(0);
    pub const DIRT:       Self = Self(1);
    pub const DIRT2:      Self = Self(2);
    pub const ROCK:       Self = Self(4);
    pub const BACKGROUND: Self = Self(8);
    pub const SEE_SHADOW: Self = Self(16);
    pub const WORM_M:     Self = Self(32);

    /// True when this pixel is empty / traversable (background bit set).
    /// C++: `(flags & Background) != 0`
    #[inline(always)]
    pub fn background(self) -> bool { (self.0 & 8) != 0 }

    /// True when this pixel blocks movement (dirt or rock).
    /// C++: `(flags & 7) != 0`   (bits Dirt | Dirt2 | Rock)
    #[inline(always)]
    pub fn dirt_rock(self) -> bool { (self.0 & 7) != 0 }

    /// True for any dirt variant (destructible).
    /// C++: `(flags & (Dirt | Dirt2)) != 0`
    #[inline(always)]
    pub fn dirt(self) -> bool { (self.0 & 3) != 0 }

    /// True for indestructible rock.
    /// C++: `(flags & Rock) != 0`
    #[inline(always)]
    pub fn rock(self) -> bool { (self.0 & 4) != 0 }

    /// True for any soft dirt variant (Dirt | Dirt2), NOT rock.
    /// C++: `(flags & (Dirt | Dirt2)) != 0`  — used by explosion dirt-particle emission.
    #[inline(always)]
    pub fn any_dirt(self) -> bool { (self.0 & 3) != 0 }

    /// True for worm-sprite collision pixels.
    /// C++: `(flags & WormM) != 0`
    #[inline(always)]
    pub fn worm_m(self) -> bool { (self.0 & 32) != 0 }

    /// True for shadow-casting pixels.
    #[inline(always)]
    pub fn see_shadow(self) -> bool { (self.0 & 16) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_flag() {
        assert!(Material::BACKGROUND.background());
        assert!(!Material::DIRT.background());
        assert!(!Material::ROCK.background());
    }

    #[test]
    fn dirt_rock_flag() {
        assert!(Material::DIRT.dirt_rock());
        assert!(Material::DIRT2.dirt_rock());
        assert!(Material::ROCK.dirt_rock());
        assert!(!Material::BACKGROUND.dirt_rock());
    }

    #[test]
    fn combined_flags() {
        // A pixel can be both Dirt and SeeShadow.
        let m = Material(Material::DIRT.0 | Material::SEE_SHADOW.0);
        assert!(m.dirt());
        assert!(m.see_shadow());
        assert!(!m.rock());
    }
}
