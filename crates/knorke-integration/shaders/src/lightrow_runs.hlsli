// Light rows (ADR 0019): reading a lamp's row. The including program declares RowFirst, RowSecond
// with their samplers, RowSize (y lines of the atlas) and HeightCode (lightrow.hlsli) before.
#ifndef KNORKE_LIGHTROW_RUNS_HLSLI
#define KNORKE_LIGHTROW_RUNS_HLSLI

// The runs of angle texel `a` (wrapped) of the lamp's row and the share of the light each lets through (0: opaque;
// PackPasses in lightrow.hlsli). No extra read: the mark rides in the two texels.
void Runs(float a, float row, out float4 runs[kRuns], out float4 pass)
{
    float2 uv = RowUV(a, row, RowSize.y);
    float scale = RowScale(row);
    float4 first = RowFirst.SampleLevel(RowFirstSampler, uv, 0.0);
    float4 second = RowSecond.SampleLevel(RowSecondSampler, uv, 0.0);
    bool partial = first.x < 0.0;
    first.x = partial ? -1.0 - first.x : first.x;
    runs[0] = UnpackRun(first.xy, HeightCode, scale);
    runs[1] = UnpackRun(first.zw, HeightCode, scale);
    runs[2] = UnpackRun(second.xy, HeightCode, scale);
    runs[3] = partial ? float4(0.0, 0.0, 0.0, 0.0) : UnpackRun(second.zw, HeightCode, scale);
    float b = second.z;
    pass = partial ? float4(fmod(b, 256.0), fmod(floor(b / 256.0), 256.0), floor(b / 65536.0), 0.0) / 255.0
                   : float4(0.0, 0.0, 0.0, 0.0);
}

// Empties the partly transparent runs: the penumbra (edge pass, SoftSight) knows opaque obstacles only, a partly
// transparent one darkens along the ray to the lamp's middle (Through in lightrow_light.hlsli; a pattern there is
// blurred over the patch the lamp shines through).
void OpaqueOnly(inout float4 runs[kRuns], float4 pass)
{
    for (int k = 0; k < kRuns; ++k) {
        runs[k] = pass[k] > 0.0 ? float4(0.0, 0.0, 0.0, 0.0) : runs[k];
    }
}

// Two runs of neighbouring angles belong to one obstacle: the same interval, stretches that overlap (one texel of
// slack).
bool Same(float4 a, float4 b)
{
    return a.y > 0.0 && b.y > 0.0 && a.z == b.z && a.w == b.w && b.x <= a.y + 1.0 && a.x <= b.y + 1.0;
}

#endif
