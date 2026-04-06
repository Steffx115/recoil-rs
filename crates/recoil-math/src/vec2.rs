use serde::{Deserialize, Serialize};

use crate::SimFloat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimVec2 {
    pub x: SimFloat,
    pub y: SimFloat,
}

impl SimVec2 {
    pub const ZERO: Self = Self {
        x: SimFloat::ZERO,
        y: SimFloat::ZERO,
    };

    pub const fn new(x: SimFloat, y: SimFloat) -> Self {
        Self { x, y }
    }

    pub fn length_squared(self) -> SimFloat {
        self.x * self.x + self.y * self.y
    }
}

impl std::ops::Add for SimVec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl std::ops::Sub for SimVec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec2_add_sub() {
        let a = SimVec2::new(SimFloat::from_int(1), SimFloat::from_int(2));
        let b = SimVec2::new(SimFloat::from_int(3), SimFloat::from_int(4));
        let sum = a + b;
        assert_eq!(sum.x, SimFloat::from_int(4));
        assert_eq!(sum.y, SimFloat::from_int(6));
    }
}
