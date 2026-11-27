// Origin: chiaro @ 9bc5b9b, shaders/src/light_mask.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Light masks (Lichtquellen 4, docs/superpowers/specs/2026-11-14-lichtmasken-design.md): a slide in front of a point or
// spot light. Only the `_mask` programs (CHI_LIGHT_MASK, always with CHI_POINT_LIGHTS) include the sampling part; the
// programs without the define are not changed by masks at all. Same formulas as MaskCoordinates and SampleMaskTexels
// in src/lighting/LightMask.cpp and src/lighting/Mask.cpp, except that the atlas is filtered by the hardware (linear,
// sRGB decoded before filtering).
//
// A program defines CHI_MASK_SLOT, the sampler slot of the atlas (right after the profile atlas), before it includes
// this file.
#ifndef CHI_LIGHT_MASK_HLSLI
#define CHI_LIGHT_MASK_HLSLI

#include "common.hlsli"

// Matches struct GpuMask in src/rendering/GpuLight.hpp: the second storage buffer of the `_mask` vertex shaders,
// parallel to the light buffer.
struct GpuMask
{
    float4 n; // xyz n' (projector: the direction; window: normal / distance), w 1 window, 0 projector
    float4 p; // xyz P' (u), w the block's first atlas v
    float4 q; // xyz Q' (v), w the block's atlas v extent
};

#ifdef CHI_MASK_SLOT

#define CHI_MASK_PASTE(a, b) a##b
#define CHI_MASK_REGISTER(kind, slot) register(CHI_MASK_PASTE(kind, slot), space2)

Texture2D<float4> MaskAtlas : CHI_MASK_REGISTER(t, CHI_MASK_SLOT);
SamplerState MaskAtlasSampler : CHI_MASK_REGISTER(s, CHI_MASK_SLOT); // linear, clamped

static const float kMaskSize = 256.0;

// The mask's colour factor toward toPixel = pixel - light, and for a window the share t of the way from the light
// at which the ray passes the window (0 for a projector or where the mask blocks). Projector: the pixel must lie in
// front of the light (w . n > 0); window: beyond the window's plane (w . n >= 1). Outside the square: 0.
float3 LightMaskTint(float3 toPixel, float4 n, float4 p, float4 q, out float t)
{
    t = 0.0;
    float wn = dot(toPixel, n.xyz);
    bool window = n.w > 0.5;
    if (window ? !(wn >= 1.0) : !(wn > 0.0)) {
        return float3(0.0, 0.0, 0.0);
    }
    float u = dot(toPixel, p.xyz) / wn;
    float v = dot(toPixel, q.xyz) / wn;
    if (!(abs(u) <= 1.0 && abs(v) <= 1.0)) {
        return float3(0.0, 0.0, 0.0);
    }
    t = window ? 1.0 / wn : 0.0;
    // Row 0 of a mask is its top (v = +1); clamp to the outer texel centres so no neighbouring block bleeds in.
    float2 uv = clamp(float2(u * 0.5 + 0.5, 0.5 - v * 0.5), 0.5 / kMaskSize, 1.0 - 0.5 / kMaskSize);
    return MaskAtlas.SampleLevel(MaskAtlasSampler, float2(uv.x, p.w + uv.y * q.w), 0.0).rgb;
}

#endif

#endif
