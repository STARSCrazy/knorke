//! Uniform blocks as plain structs: the fields are the layout, the bytes follow from them.
//!
//! The GPU reads a block as little-endian 32-bit values in declaration order. Instead of every
//! block hand-writing its byte layout (and getting an offset wrong in a way only a golden image
//! shows), [`uniform_block!`](crate::uniform_block) declares the struct once and derives
//! `to_bytes` from it. No `unsafe`: every value writes itself through [`UniformField`].

/// A value that writes itself as little-endian 32-bit words: `f32`, `u32` and arrays of fields.
pub trait UniformField {
    /// Number of bytes written.
    const SIZE: usize;

    /// Writes the value to the first [`Self::SIZE`] bytes of `out`.
    fn write(&self, out: &mut [u8]);
}

impl UniformField for f32 {
    const SIZE: usize = 4;
    fn write(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.to_le_bytes());
    }
}

impl UniformField for u32 {
    const SIZE: usize = 4;
    fn write(&self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.to_le_bytes());
    }
}

impl<T: UniformField, const N: usize> UniformField for [T; N] {
    const SIZE: usize = N * T::SIZE;
    fn write(&self, out: &mut [u8]) {
        for (value, bytes) in self.iter().zip(out.chunks_mut(T::SIZE)) {
            value.write(bytes);
        }
    }
}

/// Writes `values` as consecutive little-endian `f32`s from the start of `out`; stops at the
/// shorter of the two. For blocks whose length is data, not a struct (tile heights, atlases).
pub fn write_f32s<'a>(out: &mut [u8], values: impl IntoIterator<Item = &'a f32>) {
    for (bytes, value) in out.as_chunks_mut::<4>().0.iter_mut().zip(values) {
        *bytes = value.to_le_bytes();
    }
}

/// Declares a uniform block struct with `SIZE` and `to_bytes`.
///
/// The struct is `#[repr(C)]` and its fields, in order, are the bytes the shader reads
/// (`f32`, `u32`, arrays of them, or another block). Padding stays a field. A compile-time check
/// fails the build if the fields do not add up to the declared size.
///
/// ```
/// knorke_math::uniform_block! {
///     /// `cbuffer Demo`.
///     #[derive(Clone, Copy, Debug, Default, PartialEq)]
///     pub struct Demo[16] {
///         /// Scene size in pixels.
///         pub size: [f32; 2],
///         /// Index of the first instance.
///         pub first: u32,
///         /// Padding.
///         pub pad: u32,
///     }
/// }
/// let demo = Demo { size: [1.0, 2.0], first: 3, pad: 0 };
/// assert_eq!(&demo.to_bytes()[4..8], &2.0f32.to_le_bytes());
/// assert_eq!(Demo::SIZE, 16);
/// ```
#[macro_export]
macro_rules! uniform_block {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident [$size:expr] {
            $( $(#[$field_meta:meta])* $field_vis:vis $field:ident : $ty:ty ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr(C)]
        $vis struct $name {
            $( $(#[$field_meta])* $field_vis $field: $ty, )*
        }

        const _: () = assert!(
            0 $( + <$ty as $crate::uniform::UniformField>::SIZE )* == $size,
            "uniform block: the fields do not add up to the declared size",
        );

        impl $name {
            /// Size in bytes.
            pub const SIZE: usize = $size;

            /// The bytes as the shader reads them (little endian).
            pub fn to_bytes(&self) -> [u8; Self::SIZE] {
                let mut bytes = [0u8; Self::SIZE];
                <Self as $crate::uniform::UniformField>::write(self, &mut bytes);
                bytes
            }
        }

        impl $crate::uniform::UniformField for $name {
            const SIZE: usize = $size;

            fn write(&self, out: &mut [u8]) {
                let mut at = 0;
                $(
                    $crate::uniform::UniformField::write(&self.$field, &mut out[at..]);
                    at += <$ty as $crate::uniform::UniformField>::SIZE;
                )*
                let _ = at;
            }
        }
    };
}
