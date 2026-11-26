// Light rows (ADR 0019): reading a lamp's row. The including program declares RowFirst, RowSecond
// with their samplers and RowSize (x angles, y lamps) before.
#ifndef KNORKE_LIGHTROW_RUNS_HLSLI
#define KNORKE_LIGHTROW_RUNS_HLSLI

// The runs of angle texel `a` (wrapped) of the lamp's row.
void Runs(float a, float row, out float4 runs[kRuns])
{
    a -= RowSize.x * floor(a / RowSize.x);
    float2 uv = float2((a + 0.5) / RowSize.x, (row + 0.5) / RowSize.y);
    float4 first = RowFirst.SampleLevel(RowFirstSampler, uv, 0.0);
    float4 second = RowSecond.SampleLevel(RowSecondSampler, uv, 0.0);
    runs[0] = UnpackRun(first.xy);
    runs[1] = UnpackRun(first.zw);
    runs[2] = UnpackRun(second.xy);
    runs[3] = UnpackRun(second.zw);
}

// Two runs of neighbouring angles belong to one obstacle: the same interval, stretches that overlap (one texel of
// slack).
bool Same(float4 a, float4 b)
{
    return a.y > 0.0 && b.y > 0.0 && a.z == b.z && a.w == b.w && b.x <= a.y + 1.0 && a.x <= b.y + 1.0;
}

#endif
