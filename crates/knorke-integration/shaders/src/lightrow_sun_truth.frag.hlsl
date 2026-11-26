// Light rows (ADR 0019, plan 17.13 L3), a test tool of gpu_lightrow_probe only: the reference picture of a sun
// (lightrow_sun_surface.hlsli). Per pixel and direction an exact march in quarter texels from MapBuild.x texels before
// the receiver toward the sun until the ray leaves the mask or rises above the highest finite top; a sample blocks if
// the ray's height lies in its texel's finite interval, with Chiaro's tolerance below a top. A partly transparent
// texel (mask opacity o below kOpaqueOpacity) multiplies by 1 - o once per crossing at its first blocked sample, as
// Chiaro's trace; on a pattern obstacle by what its pattern lets through where the ray enters it, at the ray's height
// there (lightrow_pattern.hlsli). The result is the mean over the directions Rays[0..Rays[0].w) (one for a hard sun,
// points spread over a soft sun's disc).
#include "lightrow_sun_surface.hlsli"
#include "lightrow_pattern.hlsli"

Texture2D<float4> PatternIds : register(t3, space2); // read only with pattern obstacles (PatternAtlasSize.z > 0)
Texture2D<float4> PatternAtlas : register(t4, space2);
SamplerState PatternIdSampler : register(s3, space2);    // nearest
SamplerState PatternAtlasSampler : register(s4, space2); // nearest
StructuredBuffer<GpuPatternObstacle> PatternObstacles : register(t5, space2);
StructuredBuffer<GpuPattern> Patterns : register(t6, space2);
StructuredBuffer<uint> PatternData : register(t7, space2);

cbuffer SunRays : register(b2, space3)
{
    float4 Rays[64]; // DirSlope.xyz of each direction; Rays[0].w their count
};

cbuffer PatternParams : register(b3, space3)
{
    float4 PatternMaskToWorldX;
    float4 PatternMaskToWorldY;
    float4 PatternAtlasSize;
};

#define ROW_PATTERN_GEOMETRY
#define ROW_PATTERN_LIGHT
#include "lightrow_pattern.hlsli"

static const float kStep = 0.25;
static const int kMaxSteps = 16384;

float March(float3 ray, float2 start, float z)
{
    float seen = 1.0;
    bool inside = false;
    [loop] for (int k = 0; k < kMaxSteps; ++k) {
        float u = MapBuild.x + float(k) * kStep;
        float2 p = start + ray.xy * u;
        float h = z + ray.z * u;
        if (any(p < 0.0) || any(p >= Domain.xy) || h > Domain.w) {
            break;
        }
        int2 cell = int2(floor(p));
        float2 i = FiniteInterval(Heights, HeightSampler, cell, Domain.xy);
        bool blocked = h >= i.x && h < i.y - Domain.z;
        // Chiaro reads the opacity only in a frame with a partly transparent obstacle, as the map.
        float o = blocked && MapPartial.x > 0.0 ? FetchTexel(Mask, MaskSampler, cell, 1.0 / Domain.xy).r : 1.0;
        if (blocked && o >= kOpaqueOpacity) {
            return 0.0;
        }
        if (blocked && !inside) {
            uint id = 0u;
            if (PatternAtlasSize.z > 0.0) {
                id = uint(FetchTexel(PatternIds, PatternIdSampler, cell, 1.0 / Domain.xy).r * 65535.0 + 0.5);
            }
            if (id == 0u) { // HLSL's ?: would run both sides
                seen *= 1.0 - o;
            } else {
                float2 exit = PatternExit(id, start, ray.xy, u + 1.0, u - 1.0);
                seen *= PatternPass(id, exit.y, z + ray.z * exit.x, 1.0 - o);
            }
        }
        inside = blocked;
    }
    return seen;
}

float SunSight(float2 start, float z)
{
    int count = int(Rays[0].w);
    float sum = 0.0;
    [loop] for (int k = 0; k < count; ++k) {
        sum += March(Rays[k].xyz, start, z);
    }
    return sum / float(count);
}
