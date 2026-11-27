// Light rows (ADR 0019, plan 17.13 L7): a light shaft row of the medium stage (Chiaro's polar atlas,
// medium_polar.frag) read from the lamp's row instead of marched through the distance field. One texel per angle:
//   (distance 1, top 1, distance 2, top 2)
// of the first two obstacles the ray meets within the march length, distances in mask texels from the lamp's ground
// point, tops in world units (65504 for infinitely tall ones). As Chiaro's: the far edge of an obstacle below the
// lamp, the near edge of every other; an infinitely tall one ends the ray at its near edge; a partly transparent run
// counts from opacity 0.5 on (lets through at most half); no obstacle: distance 65504. The row's angle texel holding
// the polar texel's middle stands in for it.
#include "lightrow.hlsli"

Texture2D<float4> RowFirst : register(t0, space2);
Texture2D<float4> RowSecond : register(t1, space2);
SamplerState RowFirstSampler : register(s0, space2);  // nearest
SamplerState RowSecondSampler : register(s1, space2); // nearest

// `PolarRowsParams` of knorke-light2d.
cbuffer PolarRowsParams : register(b0, space3)
{
    float4 RowSize;    // y lines of the atlas
    float4 HeightCode; // how the row stores heights (PackRun in lightrow.hlsli)
    float4 PolarLamp;  // x the lamp's row (slot code), y its height, z the march length in mask texels
};

#include "lightrow_runs.hlsli"

static const float kPolarNone = 65504.0;

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float4 runs[kRuns];
    float4 pass;
    Runs(floor(uv.x * RowAngles(PolarLamp.x)), PolarLamp.x, runs, pass);
    float2 found[2] = {float2(kPolarNone, 0.0), float2(kPolarNone, 0.0)};
    int n = 0;
    [unroll] for (int k = 0; k < kRuns; ++k) {
        float4 r = runs[k];
        if (n < 2 && r.y > 0.0 && r.x < PolarLamp.z && pass[k] <= 0.5) {
            bool infinite = r.w >= kInfiniteTop;
            found[n] = float2(infinite || r.w >= PolarLamp.y ? r.x : min(r.y, PolarLamp.z), infinite ? 65504.0 : r.w);
            n = infinite ? 2 : n + 1;
        }
    }
    return float4(found[0], found[1]);
}
