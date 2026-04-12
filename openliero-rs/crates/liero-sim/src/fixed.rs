//! Fixed-point arithmetic — 1 pixel = 65 536 units (16 fractional bits).
//!
//! All ops use `wrapping_*` to guarantee bit-exact cross-platform results,
//! matching the C++ fixed-point physics in `src/game/`.
//!
//! C++ equivalents:
//!   `itof(v)  = v << 16`    →  `Fixed::from_int(v)`
//!   `ftoi(v)  = v >> 16`    →  `fixed.to_int()`
//!   `fmul(a,b)= (a*b)>>16`  →  `a.mul(b)`

/// Fixed-point scalar: Q16.16 signed integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Fixed(pub i32);

impl Fixed {
    /// Construct from whole pixels (shift left 16).
    #[inline(always)]
    pub const fn from_int(v: i32) -> Self {
        Self(v.wrapping_shl(16))
    }

    /// Convert to whole pixels (arithmetic right-shift 16).
    #[inline(always)]
    pub const fn to_int(self) -> i32 {
        self.0 >> 16
    }

    /// Multiply two fixed-point values: `(a * b) >> 16`.
    #[inline(always)]
    pub fn mul(self, rhs: Self) -> Self {
        let product = (self.0 as i64).wrapping_mul(rhs.0 as i64);
        Self((product >> 16) as i32)
    }

    /// Scale by integer numerator/denominator: `(self * num) / den`.
    #[inline(always)]
    pub fn scale(self, num: i32, den: i32) -> Self {
        let v = (self.0 as i64).wrapping_mul(num as i64) / den as i64;
        Self(v as i32)
    }
}

impl std::ops::Add for Fixed {
    type Output = Self;
    #[inline(always)]
    fn add(self, rhs: Self) -> Self { Self(self.0.wrapping_add(rhs.0)) }
}

impl std::ops::Sub for Fixed {
    type Output = Self;
    #[inline(always)]
    fn sub(self, rhs: Self) -> Self { Self(self.0.wrapping_sub(rhs.0)) }
}

impl std::ops::Neg for Fixed {
    type Output = Self;
    #[inline(always)]
    fn neg(self) -> Self { Self(self.0.wrapping_neg()) }
}

impl std::ops::AddAssign for Fixed {
    #[inline(always)]
    fn add_assign(&mut self, rhs: Self) { self.0 = self.0.wrapping_add(rhs.0); }
}

impl std::ops::SubAssign for Fixed {
    #[inline(always)]
    fn sub_assign(&mut self, rhs: Self) { self.0 = self.0.wrapping_sub(rhs.0); }
}

/// 2D fixed-point vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FixedVec {
    pub x: Fixed,
    pub y: Fixed,
}

impl FixedVec {
    pub const ZERO: Self = Self { x: Fixed(0), y: Fixed(0) };

    pub fn new(x: Fixed, y: Fixed) -> Self { Self { x, y } }

    /// Integer pixel coordinates.
    pub fn to_int(self) -> (i32, i32) { (self.x.to_int(), self.y.to_int()) }

    /// Squared integer-pixel distance to another point (overflow-safe for game coords).
    pub fn dist2_px(self, other: Self) -> i64 {
        let dx = (self.x.to_int() - other.x.to_int()) as i64;
        let dy = (self.y.to_int() - other.y.to_int()) as i64;
        dx * dx + dy * dy
    }
}

impl std::ops::Add for FixedVec {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Self { x: self.x + rhs.x, y: self.y + rhs.y } }
}

impl std::ops::AddAssign for FixedVec {
    fn add_assign(&mut self, rhs: Self) { self.x += rhs.x; self.y += rhs.y; }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_to_int_roundtrip() {
        for v in [-100, -1, 0, 1, 100, 319] {
            assert_eq!(Fixed::from_int(v).to_int(), v);
        }
    }

    #[test]
    fn add_wrapping() {
        let a = Fixed(i32::MAX);
        let b = Fixed(1);
        let _ = a + b; // must not panic
    }

    #[test]
    fn fixed_vec_dist2() {
        let a = FixedVec::new(Fixed::from_int(0), Fixed::from_int(0));
        let b = FixedVec::new(Fixed::from_int(3), Fixed::from_int(4));
        assert_eq!(a.dist2_px(b), 25);
    }
}
