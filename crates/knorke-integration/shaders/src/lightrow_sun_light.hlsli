// Light rows (ADR 0019, plan 17.13 L3): a hard sun's light with its shadow from the map of lightrow_sun_build.frag
// (lightrow_sun_surface.hlsli). The receiver at `start` reads the map texel whose middle c lies nearest; its ray from
// c is the receiver's shifted by a = dot(start - c, DirSlope.xy) along the light, so its height there is
// z - DirSlope.z * a (exact); the offset across the light (at most an eighth of a texel) is the error.
// lightrow_sun_light.frag, lightrow_sun_light_exact.frag and lightrow_sun_light_direction.frag are this file for the
// three target layouts of the light pass, lightrow_sun_light_flat.frag for a frame without surface and material buffer.
// The map is stored wrapped (SunWrap).
#include "lightrow_sun_surface.hlsli"

Texture2D<float4> Map : register(t3, space2);   // RGBA32F, two intervals (lo, hi)
SamplerState MapSampler : register(s3, space2); // nearest

float SunSight(float2 start, float z)
{
    float2 rel = start - MapOrigin.xy;
    float2 ij = clamp(floor(float2(dot(MapInverse.xy, rel), dot(MapInverse.zw, rel))), 0.0, MapOrigin.zw - 1.0);
    float2 c = MapOrigin.xy + (ij.x + 0.5) * MapAxes.xy + (ij.y + 0.5) * MapAxes.zw;
    float zc = z - DirSlope.z * dot(start - c, DirSlope.xy);
    int2 texel = SunWrap(asint(MapLattice.xy) + int2(ij), int2(MapOrigin.zw));
    float4 iv = FetchTexel(Map, MapSampler, texel, 1.0 / MapOrigin.zw);
    return (zc >= iv.x && zc < iv.y) || (zc >= iv.z && zc < iv.w) ? 0.0 : 1.0;
}
