// Origin: chiaro @ 9bc5b9b, shaders/src/medium_polar.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Light shafts in the medium (spec 2026-11-13-medien-volumetrik, "Lichtstreifen: Polar-Sichtkarte"): the visibility
// V_i(s) of a scatter sample s from a point or spot light, read from the light's row of the polar atlas
// (medium_polar.frag: per angle the first two obstacles as (distance, top), distances in mask texels).
//
// s is hidden by an obstacle on its angle if the obstacle lies closer to the light than s (in the ground plane) and
// the line from the light to s passes it below its top minus the tolerance of E22. The world -> mask map is affine,
// so the line's height grows linearly with the mask distance under every projection. The two angle texels around
// s are evaluated each and their visibilities blended linearly (filtering the stored distances would mix an
// obstacle with "none").
//
// Used by medium_light.frag: t4 the atlas (nearest), b2 PolarQueryParams.
#ifndef CHI_MEDIUM_POLAR_HLSLI
#define CHI_MEDIUM_POLAR_HLSLI

#include "common.hlsli"

Texture2D<float4> PolarAtlas : register(t4, space2);
SamplerState PolarSampler : register(s4, space2); // nearest

// Matches struct PolarQueryParams in src/rendering/MediumStage.cpp.
cbuffer PolarQueryParams : register(b2, space3)
{
    float4 PolarMaskX; // xyz: mask texel x = dot(xyz, (world x, world y, 1))
    float4 PolarMaskY; // xyz: mask texel y, the same
    float4 PolarAtlasSize; // x angles per row, y atlas height in texels, z tolerance below a top (world units)
};

// 1 if the line from the light (zLight) to a point dist mask texels away at height z passes both obstacles of one
// atlas texel.
float PolarTexelVisibility(float4 row, float dist, float zLight, float z)
{
    float slope = (z - zLight) / dist;
    bool first = row.x < dist && zLight + slope * row.x < row.y - PolarAtlasSize.z;
    bool second = row.z < dist && zLight + slope * row.z < row.w - PolarAtlasSize.z;
    return (first || second) ? 0.0 : 1.0;
}

// V of point p (world) seen from the light at lightPos (world) with atlas row `row`.
float PolarVisibility(uint row, float3 lightPos, float3 p)
{
    float2 l = float2(dot(PolarMaskX.xyz, float3(lightPos.xy, 1.0)), dot(PolarMaskY.xyz, float3(lightPos.xy, 1.0)));
    float2 s = float2(dot(PolarMaskX.xyz, float3(p.xy, 1.0)), dot(PolarMaskY.xyz, float3(p.xy, 1.0)));
    float2 v = s - l;
    float dist = length(v);
    if (dist < 1e-3) {
        return 1.0; // straight above or below the light: no obstacle in between in the ground plane
    }
    float angles = PolarAtlasSize.x;
    float a = atan2(v.y, v.x) / 6.28318530718 * angles - 0.5; // texel i holds angle (i + 0.5) / angles * 2 pi
    float i0 = floor(a);
    float f = a - i0;
    float2 invSize = 1.0 / PolarAtlasSize.xy;
    int x0 = int(i0 - angles * floor(i0 / angles)); // wrap into [0, angles)
    int x1 = x0 + 1 >= int(angles) ? 0 : x0 + 1;
    float v0 = PolarTexelVisibility(FetchTexel(PolarAtlas, PolarSampler, int2(x0, int(row)), invSize), dist,
                                    lightPos.z, p.z);
    float v1 = PolarTexelVisibility(FetchTexel(PolarAtlas, PolarSampler, int2(x1, int(row)), invSize), dist,
                                    lightPos.z, p.z);
    return lerp(v0, v1, f);
}

#endif
