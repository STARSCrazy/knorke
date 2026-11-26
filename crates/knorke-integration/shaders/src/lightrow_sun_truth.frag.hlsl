// Light rows (ADR 0019, plan 17.13 L3), a test tool of gpu_lightrow_probe only: the reference picture of a sun
// (lightrow_sun_surface.hlsli). Per pixel and direction an exact march in quarter texels from MapBuild.x texels before
// the receiver toward the sun until the ray leaves the mask or rises above the highest finite top; a sample blocks if
// the ray's height lies in its texel's finite interval, with Chiaro's tolerance below a top. The result is the mean
// over the directions Rays[0..Rays[0].w) (one for a hard sun, points spread over a soft sun's disc).
#include "lightrow_sun_surface.hlsli"

cbuffer SunRays : register(b2, space3)
{
    float4 Rays[64]; // DirSlope.xyz of each direction; Rays[0].w their count
};

static const float kStep = 0.25;
static const int kMaxSteps = 16384;

float March(float3 ray, float2 start, float z)
{
    [loop] for (int k = 0; k < kMaxSteps; ++k) {
        float u = MapBuild.x + float(k) * kStep;
        float2 p = start + ray.xy * u;
        float h = z + ray.z * u;
        if (any(p < 0.0) || any(p >= Domain.xy) || h > Domain.w) {
            return 1.0;
        }
        float2 i = FiniteInterval(Heights, HeightSampler, int2(floor(p)), Domain.xy);
        if (h >= i.x && h < i.y - Domain.z) {
            return 0.0;
        }
    }
    return 1.0;
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
