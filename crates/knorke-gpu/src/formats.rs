// Ported from Chiaro (https://github.com/STARSCrazy/chiaro), revision 9bc5b9b,
// src/core/Formats.hpp and src/core/Formats.cpp.
//
// Original notice (zlib):
// Copyright (C) 2026 Manuel Vögele
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Altered: rewritten in Rust for the Knorke Engine; `FormatText` returns a `Cow` instead of
// writing into a caller buffer, `OutputEncodingFor` returns `Option`.

//! Tables over SDL texture formats: names for messages, sRGB and sampleability, and what an
//! output shader must encode for a target format.

use std::borrow::Cow;

use sdl3_sys::gpu::SDL_GPUTextureFormat as F;

/// Name of a format for messages (`"R8G8B8A8_UNORM_SRGB"`); `None` for formats without a
/// name here.
pub fn format_name(format: F) -> Option<&'static str> {
    Some(match format {
        F::INVALID => "INVALID",
        F::A8_UNORM => "A8_UNORM",
        F::R8_UNORM => "R8_UNORM",
        F::R8G8_UNORM => "R8G8_UNORM",
        F::R8G8B8A8_UNORM => "R8G8B8A8_UNORM",
        F::R16_UNORM => "R16_UNORM",
        F::R16G16B16A16_UNORM => "R16G16B16A16_UNORM",
        F::R10G10B10A2_UNORM => "R10G10B10A2_UNORM",
        F::B8G8R8A8_UNORM => "B8G8R8A8_UNORM",
        F::R8G8B8A8_UNORM_SRGB => "R8G8B8A8_UNORM_SRGB",
        F::B8G8R8A8_UNORM_SRGB => "B8G8R8A8_UNORM_SRGB",
        F::R16_FLOAT => "R16_FLOAT",
        F::R16G16B16A16_FLOAT => "R16G16B16A16_FLOAT",
        F::R32_FLOAT => "R32_FLOAT",
        F::R32G32_FLOAT => "R32G32_FLOAT",
        F::R32G32B32A32_FLOAT => "R32G32B32A32_FLOAT",
        F::R11G11B10_UFLOAT => "R11G11B10_UFLOAT",
        F::R8G8B8A8_UINT => "R8G8B8A8_UINT",
        F::D16_UNORM => "D16_UNORM",
        F::D24_UNORM => "D24_UNORM",
        F::D32_FLOAT => "D32_FLOAT",
        F::D24_UNORM_S8_UINT => "D24_UNORM_S8_UINT",
        F::D32_FLOAT_S8_UINT => "D32_FLOAT_S8_UINT",
        _ => return None,
    })
}

/// [`format_name`], or the enum value as a number (`"123"`) for formats without a name.
pub fn format_text(format: F) -> Cow<'static, str> {
    match format_name(format) {
        Some(name) => Cow::Borrowed(name),
        None => Cow::Owned(format.0.to_string()),
    }
}

/// True for formats whose sampler decodes sRGB in hardware.
pub fn is_srgb_format(format: F) -> bool {
    matches!(
        format,
        F::R8G8B8A8_UNORM_SRGB
            | F::B8G8R8A8_UNORM_SRGB
            | F::BC1_RGBA_UNORM_SRGB
            | F::BC2_RGBA_UNORM_SRGB
            | F::BC3_RGBA_UNORM_SRGB
            | F::BC7_RGBA_UNORM_SRGB
    )
}

/// True for color formats shaders can sample as float (no depth/stencil, no integer formats).
pub fn is_sampleable_color_format(format: F) -> bool {
    !matches!(
        format,
        F::INVALID
            | F::D16_UNORM
            | F::D24_UNORM
            | F::D32_FLOAT
            | F::D24_UNORM_S8_UINT
            | F::D32_FLOAT_S8_UINT
            | F::R8_UINT
            | F::R8G8_UINT
            | F::R8G8B8A8_UINT
            | F::R16_UINT
            | F::R16G16_UINT
            | F::R16G16B16A16_UINT
            | F::R32_UINT
            | F::R32G32_UINT
            | F::R32G32B32A32_UINT
            | F::R8_INT
            | F::R8G8_INT
            | F::R8G8B8A8_INT
            | F::R16_INT
            | F::R16G16_INT
            | F::R16G16B16A16_INT
            | F::R32_INT
            | F::R32G32_INT
            | F::R32G32B32A32_INT
    )
}

/// What the output shader must do before writing to a target format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputEncoding {
    /// The target stores encoded values; the shader encodes to sRGB.
    Srgb,
    /// The target expects linear values (hardware sRGB or float).
    None,
}

/// Encoding for a supported output target format; `None` if the format is not supported as
/// an output target.
pub fn output_encoding_for(target: F) -> Option<OutputEncoding> {
    match target {
        F::R8G8B8A8_UNORM | F::B8G8R8A8_UNORM | F::R10G10B10A2_UNORM => Some(OutputEncoding::Srgb),
        F::R8G8B8A8_UNORM_SRGB | F::B8G8R8A8_UNORM_SRGB | F::R16G16B16A16_FLOAT => {
            Some(OutputEncoding::None)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // Ported from Chiaro tests/unit/FormatsTests.cpp.
    use super::*;

    #[test]
    fn srgb_format_detection() {
        assert!(is_srgb_format(F::R8G8B8A8_UNORM_SRGB));
        assert!(is_srgb_format(F::B8G8R8A8_UNORM_SRGB));
        assert!(!is_srgb_format(F::R8G8B8A8_UNORM));
        assert!(!is_srgb_format(F::R16G16B16A16_FLOAT));
    }

    #[test]
    fn sampled_input_formats_exclude_depth_and_integer_formats() {
        assert!(is_sampleable_color_format(F::R8G8B8A8_UNORM));
        assert!(is_sampleable_color_format(F::B8G8R8A8_UNORM_SRGB));
        assert!(is_sampleable_color_format(F::R16G16B16A16_FLOAT));
        assert!(is_sampleable_color_format(F::BC7_RGBA_UNORM));
        assert!(!is_sampleable_color_format(F::INVALID));
        assert!(!is_sampleable_color_format(F::D32_FLOAT));
        assert!(!is_sampleable_color_format(F::D24_UNORM_S8_UINT));
        assert!(!is_sampleable_color_format(F::R8G8B8A8_UINT));
        assert!(!is_sampleable_color_format(F::R32_INT));
    }

    #[test]
    fn output_encoding_per_target_format() {
        use OutputEncoding as E;
        assert_eq!(output_encoding_for(F::R8G8B8A8_UNORM), Some(E::Srgb));
        assert_eq!(output_encoding_for(F::B8G8R8A8_UNORM), Some(E::Srgb));
        assert_eq!(output_encoding_for(F::R10G10B10A2_UNORM), Some(E::Srgb));
        assert_eq!(output_encoding_for(F::R8G8B8A8_UNORM_SRGB), Some(E::None));
        assert_eq!(output_encoding_for(F::B8G8R8A8_UNORM_SRGB), Some(E::None));
        assert_eq!(output_encoding_for(F::R16G16B16A16_FLOAT), Some(E::None));
        assert_eq!(output_encoding_for(F::D32_FLOAT), None);
        assert_eq!(output_encoding_for(F::INVALID), None);
    }

    #[test]
    fn format_names_for_error_messages() {
        assert_eq!(
            format_name(F::R8G8B8A8_UNORM_SRGB),
            Some("R8G8B8A8_UNORM_SRGB")
        );
        assert_eq!(format_name(F::B8G8R8A8_UNORM), Some("B8G8R8A8_UNORM"));
        assert_eq!(
            format_name(F::R16G16B16A16_FLOAT),
            Some("R16G16B16A16_FLOAT")
        );
        assert_eq!(format_name(F::BC1_RGBA_UNORM), None);
    }

    #[test]
    fn format_text_falls_back_to_the_enum_value() {
        assert_eq!(format_text(F::B8G8R8A8_UNORM), "B8G8R8A8_UNORM");
        assert_eq!(
            format_text(F::BC1_RGBA_UNORM),
            F::BC1_RGBA_UNORM.0.to_string()
        );
    }
}
