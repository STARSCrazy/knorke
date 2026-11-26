// Origin: chiaro @ 9bc5b9b, shaders/src/common.hlsli, copied unchanged below this notice for the light row
// programs (lightrow_*, ADR 0019), so they light exactly as Chiaro's light programs do.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Shared shader helpers. Formulas match src/core/ColorMath.hpp exactly.
#ifndef CHI_COMMON_HLSLI
#define CHI_COMMON_HLSLI

float3 SrgbToLinear(float3 c)
{
    float3 lo = c / 12.92;
    float3 hi = pow((max(c, 0.0) + 0.055) / 1.055, 2.4);
    return lerp(hi, lo, step(c, 0.04045)); // step(c, t) == (c <= t)
}

float3 LinearToSrgb(float3 c)
{
    float3 lo = c * 12.92;
    float3 hi = 1.055 * pow(max(c, 0.0), 1.0 / 2.4) - 0.055;
    return lerp(hi, lo, step(c, 0.0031308));
}

// Occlusion mask values (E25): a texel holds the opacity of its obstacle, 0 if free. A texel is an obstacle
// from half a step of the R8 mask on, and opaque from half a step below 1.
static const float kMinObstacleOpacity = 0.5 / 255.0;
static const float kOpaqueOpacity = 254.5 / 255.0;

// Reads texel p of a texture through a nearest clamp sampler at the texel centre.
// Texture.Load alone would make shadercross reflect the texture as a storage
// texture, which SDL_GPU binds differently from a sampled one.
float4 FetchTexel(Texture2D<float4> t, SamplerState nearestSampler, int2 p, float2 invSize)
{
    return t.SampleLevel(nearestSampler, (float2(p) + 0.5) * invSize, 0.0);
}

#endif
