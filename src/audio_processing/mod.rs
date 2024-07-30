use std::ops::IndexMut;

use cpal::Sample;

pub mod instrument;
pub mod sample;
// pub mod resample;

#[repr(transparent)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct Frame([f32; 2]);

impl std::ops::AddAssign for Frame {
    fn add_assign(&mut self, rhs: Self) {
        *self.0.index_mut(0) += rhs.0[0];
        *self.0.index_mut(1) += rhs.0[1];
    }
}

impl std::ops::Add for Frame {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self([self.0[0] + rhs.0[0], self.0[1] + rhs.0[1]])
    }
}

impl std::ops::Sub for Frame {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self([self.0[0] - rhs.0[0], self.0[1] - rhs.0[1]])
    }
}

impl std::ops::MulAssign<f32> for Frame {
    fn mul_assign(&mut self, rhs: f32) {
        *self.0.index_mut(0) *= rhs;
        *self.0.index_mut(1) *= rhs;
    }
}

impl std::ops::Mul<f32> for Frame {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self([self.0[0] * rhs, self.0[1] * rhs])
    }
}

impl std::ops::AddAssign<Frame> for f32 {
    fn add_assign(&mut self, rhs: Frame) {
        *self += rhs.to_mono()
    }
}

impl std::iter::Sum for Frame {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.reduce(|acc, x| acc + x).unwrap_or_default()
    }
}

impl From<[f32; 2]> for Frame {
    fn from(value: [f32; 2]) -> Self {
        Self(value)
    }
}

impl From<f32> for Frame {
    fn from(value: f32) -> Self {
        Self([value, value])
    }
}

impl Frame {
    pub fn to_mono(self) -> f32 {
        self.0[0] + self.0[1]
    }

    pub fn from_mut<'a>(value: &'a mut [f32; 2]) -> &'a mut Self {
        // SAFETY: lifetime is specified, both mut, Self is repr(transparent).
        unsafe { std::mem::transmute::<&'a mut [f32; 2], &'a mut Self>(value) }
    }

    pub fn from_ref<'a>(value: &'a [f32; 2]) -> &'a Self {
        // SAFETY: lifetime is specified, both not mut, Self is repr(transparent).
        unsafe { std::mem::transmute::<&'a [f32; 2], &'a Self>(value) }
    }

    pub fn to_sample<S: cpal::FromSample<f32>>(self) -> [S; 2] {
        [self.0[0].to_sample(), self.0[1].to_sample()]
    }

    pub fn to_raw<'a>(into: &mut [Self]) -> &'a mut [[f32; 2]] {
        unsafe { std::mem::transmute(into) }
    }
}
