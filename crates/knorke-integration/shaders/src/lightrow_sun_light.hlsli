// Light rows (ADR 0019, plan 17.13 L3): a sun's light with its shadow from the map of lightrow_sun_build.frag
// (lightrow_sun_surface.hlsli). A hard sun's receiver at `start` reads the map texel whose middle c lies nearest; its
// ray from c is the receiver's shifted by a = dot(start - c, DirSlope.xy) along the light, so its height there is
// z - DirSlope.z * a (exact); the offset across the light (at most an eighth of a texel) is the error. A soft sun
// (MapBuild.z > 0) blends the four nearest map texels bilinearly, each with its own shift, and widens an interval's
// end at distance q along the ray to a ramp of +-MapBuild.z * q: the rays of the sun's disc meet it that much higher
// or lower (plan 17.13 L3, "weich 2x bilinear"); they spread across the light too, which the map does not hold.
// lightrow_sun_light.frag, lightrow_sun_light_exact.frag and lightrow_sun_light_direction.frag are this file for the
// three target layouts of the light pass, lightrow_sun_light_flat.frag for a frame without surface and material buffer.
// The map is stored wrapped (SunWrap).
#include "lightrow_sun_surface.hlsli"

Texture2D<float4> Map : register(t3, space2);   // RGBA32F, two intervals (lo, hi)
SamplerState MapSampler : register(s3, space2); // nearest
Texture2D<float4> Reach : register(t4, space2);   // RGBA16F, their ends' distances (soft; a hard sun binds Map)
SamplerState ReachSampler : register(s4, space2); // nearest

// Map texel ij (clamped to the map), its middle c and the receiver's height zc on c's ray.
int2 SunTexel(float2 ij, float2 start, float z, out float zc)
{
    ij = clamp(ij, 0.0, MapOrigin.zw - 1.0);
    float2 c = MapOrigin.xy + (ij.x + 0.5) * MapAxes.xy + (ij.y + 0.5) * MapAxes.zw;
    zc = z - DirSlope.z * dot(start - c, DirSlope.xy);
    return SunWrap(asint(MapLattice.xy) + int2(ij), int2(MapOrigin.zw));
}

// Shadow of interval i = (lo, hi) at height z, its ends at distances w = (q of hi, q of lo) widened.
float SunRamp(float2 i, float2 w, float z)
{
    if (i.x > i.y) {
        return 0.0;
    }
    float2 half_ = max(MapBuild.z * w, 1e-4);
    return saturate((i.y - z) / (2.0 * half_.x) + 0.5) * saturate((z - i.x) / (2.0 * half_.y) + 0.5);
}

float SoftSight(float2 ij, float2 start, float z)
{
    float zc;
    int2 texel = SunTexel(ij, start, z, zc);
    float4 iv = FetchTexel(Map, MapSampler, texel, 1.0 / MapOrigin.zw);
    float4 w = FetchTexel(Reach, ReachSampler, texel, 1.0 / MapOrigin.zw);
    return 1.0 - max(SunRamp(iv.xy, w.xy, zc), SunRamp(iv.zw, w.zw, zc));
}

float SunSight(float2 start, float z)
{
    float2 rel = start - MapOrigin.xy;
    float2 m = float2(dot(MapInverse.xy, rel), dot(MapInverse.zw, rel));
    if (MapBuild.z <= 0.0) {
        float zc;
        int2 texel = SunTexel(floor(m), start, z, zc);
        float4 iv = FetchTexel(Map, MapSampler, texel, 1.0 / MapOrigin.zw);
        return (zc >= iv.x && zc < iv.y) || (zc >= iv.z && zc < iv.w) ? 0.0 : 1.0;
    }
    float2 base = floor(m - 0.5);
    float2 f = m - 0.5 - base;
    float a = lerp(SoftSight(base, start, z), SoftSight(base + float2(1.0, 0.0), start, z), f.x);
    float b = lerp(SoftSight(base + float2(0.0, 1.0), start, z), SoftSight(base + float2(1.0, 1.0), start, z), f.x);
    return lerp(a, b, f.y);
}
