// Light rows (ADR 0019): the pass after the row build in frames with pattern obstacles, one texel per angle. Of the
// partly transparent runs (a pattern obstacle's mask holds its mean, never opaque) the first two whose far end lies
// on a pattern obstacle get its crossing (lightrow_pattern.hlsli), each as two floats:
//   id + 65536 * run + (dq + 1) / 4   id: 1 + index into PatternObstacles, run: its slot, dq: how far before the
//                                     run's stored far end the crossing lies, in texels within [-1, 3) (6 bits)
//   u                                 along the outline in world units
// 0 = none. The id is the last one found walking back from the run's far end: the build's half-texel steps may
// end a run one texel late, past a texel they stepped over. The entry is sought from one texel beyond the run's end
// to two before it: a thin obstacle's outline may lie just outside the texels the mask gave it.
#include "lightrow.hlsli"
#include "lightrow_pattern.hlsli"

Texture2D<float4> RowFirst : register(t0, space2);
Texture2D<float4> RowSecond : register(t1, space2);
Texture2D<float4> PatternIds : register(t2, space2); // R16_UNORM: 1 + index into PatternObstacles, / 65535
SamplerState RowFirstSampler : register(s0, space2);  // nearest
SamplerState RowSecondSampler : register(s1, space2); // nearest
SamplerState PatternIdSampler : register(s2, space2); // nearest
StructuredBuffer<GpuPatternObstacle> PatternObstacles : register(t3, space2);
StructuredBuffer<uint> PatternData : register(t4, space2);

// `RowLookupParams` of knorke-light2d.
cbuffer LookupParams : register(b0, space3)
{
    float4 RowSize; // x angles, y rows of the atlas
    float4 Domain;  // xy mask size in texels
    float4 WorldFromMask;
};

// `PatternParams` of knorke-light2d (Chiaro's pattern.hlsli).
cbuffer PatternParams : register(b1, space3)
{
    float4 PatternMaskToWorldX;
    float4 PatternMaskToWorldY;
    float4 PatternAtlasSize;
};

#define ROW_PATTERN_GEOMETRY
#include "lightrow_pattern.hlsli"
#include "lightrow_runs.hlsli"

float4 main(SurfaceLightVaryings i) : SV_Target0
{
    float a = floor(i.scene.x * RowSize.x);
    float angle = (a + 0.5) / RowSize.x * kTwoPi; // as the build's ray
    float2 dir = float2(cos(angle), sin(angle));
    float2 lamp = ROW_GROUND(i);
    float4 runs[kRuns];
    float4 pass;
    Runs(a, ROW_SLOT(i), runs, pass);
    float4 o = float4(0.0, 0.0, 0.0, 0.0);
    int n = 0;
    for (int k = 0; k < kRuns; ++k) {
        float4 run = runs[k];
        if (n >= 2 || pass[k] <= 0.0 || run.y <= 0.0) {
            continue;
        }
        uint id = 0u;
        for (float q = run.y - 0.25; id == 0u && q > run.x; q -= 0.25) {
            int2 texel = int2(floor(lamp + dir * q));
            if (all(texel >= 0) && all(texel < int2(Domain.xy))) {
                id = uint(FetchTexel(PatternIds, PatternIdSampler, texel, 1.0 / Domain.xy).r * 65535.0 + 0.5);
            }
        }
        if (id == 0u) {
            continue;
        }
        float2 exit = PatternExit(id, lamp, dir, run.y - 2.0, run.y + 1.0).xy;
        float dq = clamp(run.y - exit.x, -1.0, 2.9375); // (dq + 1) / 4 below 1 after rounding
        o[2 * n] = float(id) + 65536.0 * float(k) + (dq + 1.0) * 0.25;
        o[2 * n + 1] = exit.y;
        ++n;
    }
    return o;
}
