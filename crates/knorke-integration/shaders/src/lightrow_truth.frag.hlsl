// Light rows (ADR 0019): the reference picture, a test tool only. The same light, the shadow from an exact march per
// pixel over the height texture, from the lamp to one texel before the receiver in quarter texels, as many steps as
// the way needs, no distance field. A sample blocks if the ray's height there lies in the texel's interval, with
// Chiaro's tolerance below a top (shadow_trace.hlsli, WithinInterval). A lamp whose height lies in the interval of
// its own texel does not count that obstacle while the ray stays in it, as the row build does. A partly transparent
// texel (mask opacity o below kOpaqueOpacity) multiplies by 1 - o once per crossing, as Chiaro's trace; on a pattern
// obstacle by what its pattern lets through where a ray from the receiver enters it, at the height of this ray
// (lightrow_pattern.hlsli, the same steps as the rows without their angle steps and packing).
// A lamp with a source radius R is Domain.w x Domain.w points: across the ray in the ground plane and upright,
// each within +-R; the result is the mean of their marches. A line or rect light is Domain.w x Domain.w points
// spread evenly over its emitter (a line repeats each point), the mean of their hard marches.
// lightrow_truth_medium.frag (CHI_MEDIUM) multiplies by the transmittance of the medium world map to the lamp, as the
// row and Chiaro's programs do (surface_light.hlsli); its map after the other samplers, its block last.
#ifdef CHI_MEDIUM
#define CHI_MEDIUM_MAP_SLOT 5
#define CHI_MEDIUM_MAP_PARAMS_SLOT 3
#endif
#include "lightrow_surface.hlsli"
#include "lightrow_pattern.hlsli"

Texture2D<float4> PatternIds : register(t3, space2); // read only with pattern obstacles (PatternAtlasSize.z > 0)
Texture2D<float4> PatternAtlas : register(t4, space2);
SamplerState PatternIdSampler : register(s3, space2);    // nearest
SamplerState PatternAtlasSampler : register(s4, space2); // nearest
#ifdef CHI_MEDIUM
StructuredBuffer<GpuPatternObstacle> PatternObstacles : register(t6, space2);
StructuredBuffer<GpuPattern> Patterns : register(t7, space2);
StructuredBuffer<uint> PatternData : register(t8, space2);
#else
StructuredBuffer<GpuPatternObstacle> PatternObstacles : register(t5, space2);
StructuredBuffer<GpuPattern> Patterns : register(t6, space2);
StructuredBuffer<uint> PatternData : register(t7, space2);
#endif

cbuffer PatternParams : register(b2, space3)
{
    float4 PatternMaskToWorldX;
    float4 PatternMaskToWorldY;
    float4 PatternAtlasSize;
};

#define ROW_PATTERN_GEOMETRY
#define ROW_PATTERN_LIGHT
#include "lightrow_pattern.hlsli"

static const float kStep = 0.25;

// (base, top) of the texel under p; top below base where nothing stands or p is outside the mask.
float2 IntervalAt(float2 p)
{
    if (any(p < 0.0) || any(p >= Domain.xy)) {
        return float2(1.0, -1.0);
    }
    float2 i = Heights.SampleLevel(HeightSampler, (floor(p) + 0.5) / Domain.xy, 0.0).rg;
    return i.y > -i.x ? float2(max(-i.x, 0.0), i.y) : float2(1.0, -1.0);
}

// What a crossing of the ray lamp + dir * q that ends at q1 lets through: 1 - o of its first texel, or, if its last
// sample lies on a pattern obstacle, its pattern where the ray from the receiver enters it (Chiaro's piece: from a
// texel beyond the last sample to one texel before it).
float Crossing(float2 lamp, float2 dir, float q1, float zl, float z, float r, float o)
{
    uint id = 0u;
    if (PatternAtlasSize.z > 0.0) {
        id = uint(FetchTexel(PatternIds, PatternIdSampler, int2(floor(lamp + dir * q1)), 1.0 / Domain.xy).r * 65535.0
                  + 0.5);
    }
    if (id == 0u) { // HLSL's ?: would run both sides
        return 1.0 - o;
    }
    float3 exit = PatternExit(id, PatternWorld(lamp), PatternWorldStep(dir), q1 - 1.0, q1 + 1.0);
    return PatternPass(id, exit.y, uint(exit.z), lerp(zl, z, exit.x / r), 1.0 - o, 0.0);
}

// The share of the light from the lamp point (mask texels, height zl) that reaches the place `start` at height z.
float March(float2 lamp, float zl, float2 start, float z)
{
    float r = length(start - lamp);
    if (r < 1e-3) {
        return 1.0;
    }
    float2 dir = (start - lamp) / r;
    float2 own = IntervalAt(lamp);
    bool skipping = own.y >= own.x && own.x <= zl && zl <= own.y;
    int steps = int(floor((r - RowSize.z) / kStep));
    float seen = 1.0;
    bool inside = false;
    float last = 0.0; // the open crossing: its last sample and its first texel's opacity
    float o0 = 0.0;
    [loop] for (int s = 1; s <= steps; ++s) {
        float q = float(s) * kStep;
        float2 here = IntervalAt(lamp + dir * q);
        if (skipping) {
            if (all(here == own)) {
                continue;
            }
            skipping = false;
        }
        float h = lerp(zl, z, q / r);
        bool blocked = h >= here.x && h < here.y - RowSize.w;
        if (blocked) {
            float o = FetchTexel(Mask, MaskSampler, int2(floor(lamp + dir * q)), 1.0 / Domain.xy).r;
            if (o >= kOpaqueOpacity) {
                return 0.0;
            }
            o0 = inside ? o0 : o;
            last = q;
        } else if (inside) {
            seen *= Crossing(lamp, dir, last, zl, z, r, o0);
        }
        inside = blocked;
    }
    return inside ? seen * Crossing(lamp, dir, last, zl, z, r, o0) : seen;
}

float Sight(SurfaceLightVaryings l, float2 start, float z, float r, float from) // from: 0, the probe has no masks
{
    int points = int(Domain.w);
    float sum = 0.0;
    if (l.directionType.w > 1.5) {
        [loop] for (int i = 0; i < points; ++i) {
            [loop] for (int j = 0; j < points; ++j) {
                float2 ground;
                float zl;
                EmitterPoint(l, (float2(i, j) + 0.5) / float(points) * 2.0 - 1.0, ground, zl);
                sum += March(ground, zl, start, z);
            }
        }
        return sum / float(points * points);
    }
    float size = ROW_SIZE(l);
    if (size <= 0.0) {
        return March(ROW_GROUND(l), ROW_HEIGHT(l), start, z);
    }
    float2 dir = (start - ROW_GROUND(l)) / r;
    float2 across = float2(-dir.y, dir.x);
    // `size` world units across the ray, in mask texels.
    float texels = size / length(float2(dot(WorldFromMask.xy, across), dot(WorldFromMask.zw, across)));
    [loop] for (int i = 0; i < points; ++i) {
        [loop] for (int j = 0; j < points; ++j) {
            float2 at = (float2(i, j) + 0.5) / float(points) * 2.0 - 1.0;
            sum += March(ROW_GROUND(l) + across * (at.x * texels), ROW_HEIGHT(l) + at.y * size, start, z);
        }
    }
    return sum / float(points * points);
}
