use serde::{Deserialize, Serialize};

use crate::SimFloat;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimVec3 {
    pub x: SimFloat,
    pub y: SimFloat,
    pub z: SimFloat,
}

impl SimVec3 {
    pub const ZERO: Self = Self {
        x: SimFloat::ZERO,
        y: SimFloat::ZERO,
        z: SimFloat::ZERO,
    };

    pub const fn new(x: SimFloat, y: SimFloat, z: SimFloat) -> Self {
        Self { x, y, z }
    }

    pub fn length_squared(self) -> SimFloat {
        self.x * self.x + self.y * self.y + self.z * self.z
    }
}

impl std::ops::Add for SimVec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::Sub for SimVec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec3_add() {
        let a = SimVec3::new(
            SimFloat::from_int(1),
            SimFloat::from_int(2),
            SimFloat::from_int(3),
        );
        let b = SimVec3::new(
            SimFloat::from_int(4),
            SimFloat::from_int(5),
            SimFloat::from_int(6),
        );
        let sum = a + b;
        assert_eq!(sum.x, SimFloat::from_int(5));
        assert_eq!(sum.y, SimFloat::from_int(7));
        assert_eq!(sum.z, SimFloat::from_int(9));
    }
}
