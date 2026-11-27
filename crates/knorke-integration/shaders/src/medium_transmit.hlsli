// Origin: chiaro @ 9bc5b9b, shaders/src/medium_transmit.hlsli, copied unchanged below this notice for the light row
// programs (lightrow_*, ADR 0019), so they dim the light through the medium exactly as Chiaro's light programs do.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Transmittance through the medium world map (spec 2026-11-13-medien-volumetrik, "Abschirmen: Medium-Weltkarte"):
// the medium layer reprojected onto the world grid of the occlusion mask (E20) at a quarter of its resolution
// (medium_world.frag), RGBA16F (sigma per world unit, bottom, top, highest top) per cell; A holds the highest top of
// the whole medium this frame in every texel (medium_world_top.vert). Light on its way from a light to a
// point is dimmed by exp(-integral of sigma) along the segment, sigma counted only inside each cell's slab.
//
// The including program defines, before the include:
//   CHI_MEDIUM_MAP_SLOT          sampler slot of the map (after all its other samplers), space2
//   CHI_MEDIUM_MAP_PARAMS_SLOT   uniform slot of MediumMapParams (after its other uniform blocks), space3
// The `_medium` light programs (CHI_MEDIUM) take K = kLightMediumSamples; medium_light.frag the preset's K.
// The haze does not dim the light path (Task 5 decision): only the medium layer shields.
#ifndef CHI_MEDIUM_TRANSMIT_HLSLI
#define CHI_MEDIUM_TRANSMIT_HLSLI

#include "common.hlsli"

#define CHI_MEDIUM_MAP_PASTE(a, b) a##b
#define CHI_MEDIUM_MAP_REGISTER(kind, slot, space) register(CHI_MEDIUM_MAP_PASTE(kind, slot), space)

Texture2D<float4> MediumMap : CHI_MEDIUM_MAP_REGISTER(t, CHI_MEDIUM_MAP_SLOT, space2);
SamplerState MediumMapSampler : CHI_MEDIUM_MAP_REGISTER(s, CHI_MEDIUM_MAP_SLOT, space2); // nearest

// Matches struct MediumMapParams in src/rendering/MediumMapStage.hpp.
cbuffer MediumMapParams : CHI_MEDIUM_MAP_REGISTER(b, CHI_MEDIUM_MAP_PARAMS_SLOT, space3)
{
    float4 MediumMapX;      // xyz: map texel x = dot(xyz, (world x, world y, 1))
    float4 MediumMapY;      // xyz: map texel y, the same
    float4 MediumMapSize;   // xy map size in texels, z highest top the layer can encode (world units), w 1 = map present,
                            // 2 = with the rectangle of cells with medium in A of texels 1 to 4 of row 0 (E108)
    float4 MediumMapInvAxes; // rows of the inverse of [x_axis y_axis] (scene -> ground for fullscreen programs)
    float4 MediumMapOrigin;  // xy projection origin in scene pixels
};

// Transmission samples of the light programs (all presets).
static const uint kLightMediumSamples = 8u;

// The ground point (world xy at z = 0) under scene position `scene`.
float2 MediumMapGround(float2 scene)
{
    float2 v = scene - MediumMapOrigin.xy;
    return float2(dot(MediumMapInvAxes.xy, v), dot(MediumMapInvAxes.zw, v));
}

// The highest top of the medium this frame (world units), a little above it: the map's A (fp16), raised by its
// rounding. 0 without medium. Never above the highest top the layer can encode.
float MediumMapTop()
{
    float top = FetchTexel(MediumMap, MediumMapSampler, int2(0, 0), 1.0 / MediumMapSize.xy).a;
    return top > 0.0 ? min(top * 1.001 + 1e-3, MediumMapSize.z) : 0.0;
}

// Map position (texels) of world point xy.
float2 MediumMapPosition(float2 world)
{
    return float2(dot(MediumMapX.xyz, float3(world, 1.0)), dot(MediumMapY.xyz, float3(world, 1.0)));
}

// Whether the map positions a and b (map texels, before the -0.5 of MediumMapSigma) may reach a cell with medium:
// false when the rectangle spanned by them, widened by the bilinear footprint and fp16 rounding, misses the rectangle
// of cells with medium (E108). Every tap MediumMapSigma takes between them then holds sigma 0, so skipping the march
// gives the same 1. Always true without the rectangle (a map narrower than 5 texels).
bool MediumMapMayHit(float2 a, float2 b)
{
    if (MediumMapSize.w < 1.5) {
        return true;
    }
    float2 invSize = 1.0 / MediumMapSize.xy;
    float4 rect = float4(FetchTexel(MediumMap, MediumMapSampler, int2(1, 0), invSize).a,
                         FetchTexel(MediumMap, MediumMapSampler, int2(2, 0), invSize).a,
                         FetchTexel(MediumMap, MediumMapSampler, int2(3, 0), invSize).a,
                         FetchTexel(MediumMap, MediumMapSampler, int2(4, 0), invSize).a);
    float2 lo = min(a, b) - 0.5 - 3.0; // the taps floor(p - 0.5) and + 1, 2 texels of fp16 rounding beyond 2048
    float2 hi = max(a, b) - 0.5 + 3.0;
    return all(hi >= rect.xy) && all(lo <= rect.zw);
}

// Mean extinction over the height interval [za, zb] (either order) above world point xy: the 2x2 cells around it,
// bilinear weights, each cell counting sigma times the share of the interval inside its slab. A flat interval counts
// sigma where it lies inside the slab. Cells outside the map hold no medium.
float MediumMapSigma(float2 world, float za, float zb)
{
    float2 p = float2(dot(MediumMapX.xyz, float3(world, 1.0)), dot(MediumMapY.xyz, float3(world, 1.0))) - 0.5;
    int2 i0 = int2(floor(p));
    float2 f = p - float2(i0);
    float lo = min(za, zb), hi = max(za, zb);
    float span = hi - lo;
    int2 size = int2(MediumMapSize.xy);
    float sigma = 0.0;
    [unroll] for (int k = 0; k < 4; ++k) {
        int2 o = int2(k & 1, k >> 1);
        int2 tap = i0 + o;
        if (all(tap >= 0) && all(tap < size)) {
            float4 c = FetchTexel(MediumMap, MediumMapSampler, tap, 1.0 / MediumMapSize.xy);
            float inside = span > 1e-5 ? saturate((min(hi, c.b) - max(lo, c.g)) / span)
                                       : (lo >= c.g && lo <= c.b ? 1.0 : 0.0);
            float2 b = float2(o.x != 0 ? f.x : 1.0 - f.x, o.y != 0 ? f.y : 1.0 - f.y);
            sigma += b.x * b.y * c.r * inside;
        }
    }
    return sigma;
}

// Transmittance of the medium on the segment from -> to (world units): K intervals of equal length over the part
// below the highest top this frame (MediumMapTop), each with the mean sigma of its height range at its middle.
// Returns 1 without a map.
float3 MediumTransmit(float3 from, float3 to, uint K)
{
    float3 d = to - from;
    float len = length(d);
    if (MediumMapSize.w == 0.0 || K == 0u || len < 1e-5) {
        return float3(1.0, 1.0, 1.0);
    }
    float top = MediumMapTop();
    float t0 = 0.0, t1 = 1.0;
    if (d.z > 1e-6) {
        t1 = min(t1, (top - from.z) / d.z);
    } else if (d.z < -1e-6) {
        t0 = max(t0, (top - from.z) / d.z);
    } else if (from.z > top) {
        return float3(1.0, 1.0, 1.0);
    }
    if (t1 <= t0) {
        return float3(1.0, 1.0, 1.0);
    }
    float2 xa = from.xy + d.xy * t0, xb = from.xy + d.xy * t1;
    if (!MediumMapMayHit(MediumMapPosition(xa), MediumMapPosition(xb))) {
        return float3(1.0, 1.0, 1.0);
    }
    float dt = (t1 - t0) / float(K);
    float tau = 0.0;
    [loop] for (uint k = 0u; k < K; ++k) {
        float ta = t0 + float(k) * dt;
        float2 xy = from.xy + d.xy * (ta + 0.5 * dt);
        tau += MediumMapSigma(xy, from.z + d.z * ta, from.z + d.z * (ta + dt));
    }
    return exp(-tau * dt * len).xxx;
}

// Transmittance from `from` toward a directional light (unit vector `toLight`, world space) up to just above the
// highest top of the medium this frame (MediumMapTop), so the K intervals cover the medium, not the empty heights up
// to the highest top the layer could encode. A light near the horizon is taken at no less than kMinMediumSunZ, so the
// path stays finite.
static const float kMinMediumSunZ = 0.05;

float3 MediumTransmitToward(float3 from, float3 toLight, uint K)
{
    if (MediumMapSize.w == 0.0) {
        return float3(1.0, 1.0, 1.0);
    }
    float rise = MediumMapTop() - from.z;
    if (rise <= 0.0) {
        return float3(1.0, 1.0, 1.0);
    }
    return MediumTransmit(from, from + toLight * (rise / max(toLight.z, kMinMediumSunZ)), K);
}

#endif
