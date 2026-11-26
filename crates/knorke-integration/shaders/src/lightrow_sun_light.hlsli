// Light rows (ADR 0019, plan 17.13 L3): a sun's light with its shadow from the map of lightrow_sun_build.frag
// (lightrow_sun_surface.hlsli). A hard sun's receiver at `start` reads the map texel whose middle c lies nearest; its
// ray from c is the receiver's shifted by a = dot(start - c, DirSlope.xy) along the light, so its height there is
// z - DirSlope.z * a (exact); the offset across the light (at most an eighth of a texel) is the error. A soft sun
// (MapBuild.z > 0) blends the four nearest map texels bilinearly, each with its own shift, and widens an interval's
// end at distance q along the ray to a ramp of +-MapBuild.z * q: the rays of the sun's disc meet it that much higher
// or lower (plan 17.13 L3, "weich 2x bilinear"); they spread across the light too, which the map does not hold.
// lightrow_sun_light.frag, lightrow_sun_light_exact.frag and lightrow_sun_light_direction.frag are this file for the
// three target layouts of the light pass, lightrow_sun_light_flat.frag for a frame without surface and material buffer.
// The map is stored wrapped (SunWrap). A map with partly transparent runs (MapPartial.x) multiplies by what each run
// whose interval holds the height lets through: its pass, or on a pattern obstacle the pattern where the receiver's
// own ray enters it (PatternExit from a texel before the run's begin to two after, the begin moved by the shift
// along the light), at that ray's height there (PatternPass): c's ray would read the pattern up to an eighth of a
// texel aside. A soft sun reads it on each of its four texels' own rays (their spread across the light stands in for
// the disc's) and ramps a run's ends as an interval's, both at the distance of its begin.
#include "lightrow_sun_surface.hlsli"
#include "lightrow_pattern.hlsli"

Texture2D<float4> Map : register(t3, space2);   // RGBA32F, two intervals (lo, hi)
SamplerState MapSampler : register(s3, space2); // nearest
Texture2D<float4> Reach : register(t4, space2);   // RGBA16F, their ends' distances (soft; a hard sun binds Map)
SamplerState ReachSampler : register(s4, space2); // nearest
Texture2D<float4> Run0 : register(t5, space2); // RGBA32F, the partly transparent runs (lo, hi, q, pass code)
Texture2D<float4> Run1 : register(t6, space2);
SamplerState Run0Sampler : register(s5, space2); // nearest
SamplerState Run1Sampler : register(s6, space2); // nearest
Texture2D<float4> PatternAtlas : register(t7, space2); // read for pattern runs only
SamplerState PatternAtlasSampler : register(s7, space2); // nearest
// The storage buffers follow all samplers (the material buffer's too).
#ifdef CHI_MATERIAL_EXACT
StructuredBuffer<GpuPatternObstacle> PatternObstacles : register(t9, space2);
StructuredBuffer<GpuPattern> Patterns : register(t10, space2);
StructuredBuffer<uint> PatternData : register(t11, space2);
#else
StructuredBuffer<GpuPatternObstacle> PatternObstacles : register(t8, space2);
StructuredBuffer<GpuPattern> Patterns : register(t9, space2);
StructuredBuffer<uint> PatternData : register(t10, space2);
#endif

// `PatternParams` of knorke-light2d; all 0 in a frame without pattern obstacles.
cbuffer PatternParams : register(b2, space3)
{
    float4 PatternMaskToWorldX;
    float4 PatternMaskToWorldY;
    float4 PatternAtlasSize;
};

#define ROW_PATTERN_GEOMETRY
#define ROW_PATTERN_LIGHT
#include "lightrow_pattern.hlsli"

// Map texel ij (clamped to the map), its middle c and the receiver's height zc on c's ray.
int2 SunTexel(float2 ij, float2 start, float z, out float zc, out float2 c)
{
    ij = clamp(ij, 0.0, MapOrigin.zw - 1.0);
    c = MapOrigin.xy + (ij.x + 0.5) * MapAxes.xy + (ij.y + 0.5) * MapAxes.zw;
    zc = z - DirSlope.z * dot(start - c, DirSlope.xy);
    return SunWrap(asint(MapLattice.xy) + int2(ij), int2(MapOrigin.zw));
}

// What run r = (lo, hi, q, code) of a texel whose middle lies `a` along the light before `start` lets through the
// receiver there at height z, where it blocks.
float RunPass(float4 r, float2 start, float z, float a)
{
    if (r.w >= 0.0 || PatternAtlasSize.z <= 0.0) { // no records bound: the mean (a kept texel cannot name one)
        return SunMeanPass(r.w);
    }
    float id = floor(-r.w);
    float3 exit = PatternExit(uint(id), start, DirSlope.xy, r.z - a + 2.0, r.z - a - 1.0);
    return PatternPass(uint(id), exit.y, uint(exit.z), z + DirSlope.z * exit.x, 2.0 * (-r.w - id));
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

// What the partly transparent runs of `texel` (middle c) let through the receiver at `start`, height z (zc on c's
// ray; hard: where their interval holds it).
float Through(int2 texel, float2 c, float zc, float2 start, float z, bool soft)
{
    if (MapPartial.x <= 0.0) {
        return 1.0;
    }
    float4 runs[2] = {FetchTexel(Run0, Run0Sampler, texel, 1.0 / MapOrigin.zw),
                      FetchTexel(Run1, Run1Sampler, texel, 1.0 / MapOrigin.zw)};
    float seen = 1.0;
    [unroll] for (int k = 0; k < 2; ++k) {
        float4 r = runs[k];
        float share = soft ? SunRamp(r.xy, r.zz, zc) : (zc >= r.x && zc < r.y ? 1.0 : 0.0);
        if (share > 0.0) { // HLSL's ?: would run both sides
            seen *= 1.0 - share * (1.0 - RunPass(r, start, z, dot(start - c, DirSlope.xy)));
        }
    }
    return seen;
}

float SoftSight(float2 ij, float2 start, float z)
{
    float zc;
    float2 c;
    int2 texel = SunTexel(ij, start, z, zc, c);
    float4 iv = FetchTexel(Map, MapSampler, texel, 1.0 / MapOrigin.zw);
    float4 w = FetchTexel(Reach, ReachSampler, texel, 1.0 / MapOrigin.zw);
    return (1.0 - max(SunRamp(iv.xy, w.xy, zc), SunRamp(iv.zw, w.zw, zc))) * Through(texel, c, zc, c, zc, true);
}

float SunSight(float2 start, float z)
{
    float2 rel = start - MapOrigin.xy;
    float2 m = float2(dot(MapInverse.xy, rel), dot(MapInverse.zw, rel));
    if (MapBuild.z <= 0.0) {
        float zc;
        float2 c;
        int2 texel = SunTexel(floor(m), start, z, zc, c);
        float4 iv = FetchTexel(Map, MapSampler, texel, 1.0 / MapOrigin.zw);
        return (zc >= iv.x && zc < iv.y) || (zc >= iv.z && zc < iv.w) ? 0.0 : Through(texel, c, zc, start, z, false);
    }
    float2 base = floor(m - 0.5);
    float2 f = m - 0.5 - base;
    float a = lerp(SoftSight(base, start, z), SoftSight(base + float2(1.0, 0.0), start, z), f.x);
    float b = lerp(SoftSight(base + float2(0.0, 1.0), start, z), SoftSight(base + float2(1.0, 1.0), start, z), f.x);
    return lerp(a, b, f.y);
}
