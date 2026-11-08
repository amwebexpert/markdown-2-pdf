//! 2D affine matrices in PDF's row-vector convention (`[a b c d e f]`, point × M).

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Matrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub const fn translate(tx: f32, ty: f32) -> Self {
        Self {
            e: tx,
            f: ty,
            ..Self::IDENTITY
        }
    }

    /// Builds a matrix from six PDF number operands (`cm`, `Tm`, `/Matrix`).
    #[expect(
        clippy::many_single_char_names,
        reason = "PDF names the six matrix entries a-f"
    )]
    pub fn from_slice(values: &[f32]) -> Option<Self> {
        match *values {
            [a, b, c, d, e, f] => Some(Self { a, b, c, d, e, f }),
            _ => None,
        }
    }

    /// `self × other`: applies `self` first, then `other`.
    #[must_use]
    pub fn then(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    /// Length of the transformed x unit vector (horizontal scale).
    pub fn x_scale(self) -> f32 {
        self.a.hypot(self.b)
    }

    /// Length of the transformed y unit vector (vertical scale).
    pub fn y_scale(self) -> f32 {
        self.c.hypot(self.d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn then_applies_left_matrix_first() {
        let scale = Matrix {
            a: 2.0,
            d: 2.0,
            ..Matrix::IDENTITY
        };
        let moved = Matrix::translate(10.0, 5.0).then(scale);
        assert_eq!((moved.e, moved.f), (20.0, 10.0));
        let scaled_then_moved = scale.then(Matrix::translate(10.0, 5.0));
        assert_eq!((scaled_then_moved.e, scaled_then_moved.f), (10.0, 5.0));
    }

    #[test]
    fn from_slice_requires_six_values() {
        assert!(Matrix::from_slice(&[1.0, 0.0, 0.0, 1.0, 0.0]).is_none());
        assert_eq!(
            Matrix::from_slice(&[1.0, 0.0, 0.0, 1.0, 3.0, 4.0]),
            Some(Matrix::translate(3.0, 4.0))
        );
    }
}
