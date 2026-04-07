/// Macro to generate arithmetic operator trait impls for vector types.
///
/// Generates: Add, Sub, Neg, AddAssign, SubAssign, Mul<SimFloat>,
/// Mul<VecType> for SimFloat, Div<SimFloat>, and Default.
macro_rules! impl_vector_ops {
    ($VecType:ident { $($field:ident),+ }) => {
        impl ::std::ops::Add for $VecType {
            type Output = Self;
            #[inline]
            fn add(self, rhs: Self) -> Self {
                Self::new($(self.$field + rhs.$field),+)
            }
        }

        impl ::std::ops::Sub for $VecType {
            type Output = Self;
            #[inline]
            fn sub(self, rhs: Self) -> Self {
                Self::new($(self.$field - rhs.$field),+)
            }
        }

        impl ::std::ops::Neg for $VecType {
            type Output = Self;
            #[inline]
            fn neg(self) -> Self {
                Self::new($(-self.$field),+)
            }
        }

        impl ::std::ops::AddAssign for $VecType {
            #[inline]
            fn add_assign(&mut self, rhs: Self) {
                *self = *self + rhs;
            }
        }

        impl ::std::ops::SubAssign for $VecType {
            #[inline]
            fn sub_assign(&mut self, rhs: Self) {
                *self = *self - rhs;
            }
        }

        /// Scalar multiply: VecType * SimFloat
        impl ::std::ops::Mul<crate::SimFloat> for $VecType {
            type Output = Self;
            #[inline]
            fn mul(self, rhs: crate::SimFloat) -> Self {
                Self::new($(self.$field * rhs),+)
            }
        }

        /// Scalar multiply: SimFloat * VecType
        impl ::std::ops::Mul<$VecType> for crate::SimFloat {
            type Output = $VecType;
            #[inline]
            fn mul(self, rhs: $VecType) -> $VecType {
                $VecType::new($(self * rhs.$field),+)
            }
        }

        /// Scalar divide: VecType / SimFloat
        impl ::std::ops::Div<crate::SimFloat> for $VecType {
            type Output = Self;
            #[inline]
            fn div(self, rhs: crate::SimFloat) -> Self {
                Self::new($(self.$field / rhs),+)
            }
        }

        impl Default for $VecType {
            fn default() -> Self {
                Self::ZERO
            }
        }
    };
}

pub(crate) use impl_vector_ops;
