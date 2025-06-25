use std::{array, ops::IndexMut, slice::from_raw_parts};

use dasp::sample::{FromSample, ToSample};

pub(crate) mod instrument;
pub mod playback;
pub(crate) mod sample;

#[repr(transparent)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Frame([f32; 2]);

impl std::ops::AddAssign for Frame {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
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

impl std::ops::SubAssign for Frame {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
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
    // split into left and right.
    pub fn split_array<const N: usize>(value: [Frame; N]) -> ([f32; N], [f32; N]) {
        (
            array::from_fn(|i| value[i].0[0]),
            array::from_fn(|i| value[i].0[1]),
        )
    }

    pub fn sum_to_mono(self) -> f32 {
        self.0[0] + self.0[1]
    }

    pub fn from_mut<'a>(value: &'a mut [f32; 2]) -> &'a mut Self {
        // SAFETY: lifetime is specified, both mut, Self is repr(transparent).
        unsafe { std::mem::transmute::<&'a mut [f32; 2], &'a mut Self>(value) }
    }

    pub fn from_interleaved<'a>(value: &'a [f32]) -> &'a [Frame] {
        debug_assert!(value.len().rem_euclid(2) == 0);
        let len = value.len() / 2;
        let ptr = value.as_ptr().cast();
        // SAFETY: keeps the same lifetime and mutability.
        // [f32; 2] is a valid frame
        unsafe { from_raw_parts(ptr, len) }
    }

    pub fn from_ref<'a>(value: &'a [f32; 2]) -> &'a Self {
        // SAFETY: lifetime is specified, both not mut, Self is repr(transparent).
        unsafe { std::mem::transmute::<&'a [f32; 2], &'a Self>(value) }
    }

    pub fn to_sample<S: dasp::sample::FromSample<f32>>(self) -> [S; 2] {
        [self.0[0].to_sample_(), self.0[1].to_sample_()]
    }

    pub fn to_raw<'a>(into: &mut [Self]) -> &'a mut [[f32; 2]] {
        unsafe { std::mem::transmute(into) }
    }
}
