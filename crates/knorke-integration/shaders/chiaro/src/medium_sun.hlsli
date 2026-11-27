// Origin: chiaro @ 9bc5b9b, shaders/src/medium_sun.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Sun shadow heights in the medium (medium_sun_height.frag): the height below which a point over a ground point lies
// in a directional light's shadow. The atlas stacks one block of mask-sized rows per directional light with shadows.
//
// The including program defines, before the include:
//   CHI_SUN_HEIGHT_SLOT   sampler slot of the atlas (linear), space2
//   CHI_SUN_PARAMS_SLOT   uniform slot of SunParams, space3
#ifndef CHI_MEDIUM_SUN_HLSLI
#define CHI_MEDIUM_SUN_HLSLI

#define CHI_SUN_PASTE(a, b) a##b
#define CHI_SUN_REGISTER(kind, slot, space) register(CHI_SUN_PASTE(kind, slot), space)

Texture2D<float4> SunHeights : CHI_SUN_REGISTER(t, CHI_SUN_HEIGHT_SLOT, space2);
SamplerState SunHeightSampler : CHI_SUN_REGISTER(s, CHI_SUN_HEIGHT_SLOT, space2); // linear

// Matches struct SunParams in src/rendering/MediumStage.hpp; one per directional light in the medium.
cbuffer SunParams : CHI_SUN_REGISTER(b, CHI_SUN_PARAMS_SLOT, space3)
{
    float4 SunColor;   // rgb colour * intensity * haze factor, w the light's block of the atlas + 1 (0 = no shadows)
    float4 SunToLight; // xyz unit vector toward the light (world), w highest finite obstacle top (no shade above)
    float4 SunMaskX;   // xyz: mask texel x = dot(xyz, (world x, world y, 1))
    float4 SunMaskY;   // xyz: mask texel y, the same
    float4 SunAtlas;   // xy mask size in texels, zw atlas size in texels (blocks stacked in y; it only grows)
    float4 SunBlock;   // xy texels of one block: the mask size divided by the sun map divisor, rounded up
};

// Height below which the point over world ground point `ground` lies in the shadow of the light with atlas block
// `light`. 0 outside the mask domain (nothing casts there). Bilinear between mask texels, inside the light's block.
float SunShadowHeight(uint light, float2 ground)
{
    float2 m = float2(dot(SunMaskX.xyz, float3(ground, 1.0)), dot(SunMaskY.xyz, float3(ground, 1.0)));
    float2 size = SunAtlas.xy;
    if (any(m < 0.0) || any(m > size)) {
        return 0.0;
    }
    float2 b = clamp(m * SunBlock.xy / size, float2(0.5, 0.5), SunBlock.xy - 0.5);
    float2 uv = float2(b.x, b.y + float(light) * SunBlock.y) / SunAtlas.zw;
    return SunHeights.SampleLevel(SunHeightSampler, uv, 0.0).r;
}

#endif
