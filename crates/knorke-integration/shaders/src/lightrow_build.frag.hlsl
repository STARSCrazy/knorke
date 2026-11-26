// Light rows (ADR 0019): builds one lamp's row, one texel per angle.
// Marches once outward from the lamp's ground point over the height texture of the occlusion mask, to the lamp's
// radius. Consecutive texels with the same height interval are one run (rn, rf, base, top). A run is dropped if a
// nearer run standing on the ground already blocks every slope it would block; of more than four the last slot
// takes the hull of the rest, which can only hide too much. The run the lamp stands in is skipped.
// An edge lies where the ray enters
// the texel of the first sample with another interval, not on the sample.
#include "lightrow.hlsli"

Texture2D<float4> Heights : register(t0, space2); // RGBA16F (-base, top); free texels hold -65504
SamplerState HeightSampler : register(s0, space2); // nearest

cbuffer RowParams : register(b0, space3)
{
    float4 WorldFromMask; // world = (dot(xy, m), dot(zw, m)) for a vector m in mask texels
    float4 Domain;        // xy mask size in texels, z march step in texels
};

struct RowOutput
{
    float4 first : SV_Target0;  // runs 0 and 1
    float4 second : SV_Target1; // runs 2 and 3
};

static const int kMaxSteps = 1200;

// (base, top) of the texel under p; top below base where nothing stands or p is outside the mask.
float2 IntervalAt(float2 p)
{
    if (any(p < 0.0) || any(p >= Domain.xy)) {
        return float2(1.0, -1.0);
    }
    float2 i = Heights.SampleLevel(HeightSampler, (floor(p) + 0.5) / Domain.xy, 0.0).rg;
    return i.y > -i.x ? float2(max(-i.x, 0.0), i.y) : float2(1.0, -1.0);
}

// Where the ray lamp + dir * q enters the texel of its point at q = t, kept within the last step.
float Entry(float2 lamp, float2 dir, float t, float stepLength)
{
    float2 cell = floor(lamp + dir * t);
    float q = t - stepLength;
    if (abs(dir.x) > 1e-6) {
        q = max(q, (cell.x + (dir.x < 0.0 ? 1.0 : 0.0) - lamp.x) / dir.x);
    }
    if (abs(dir.y) > 1e-6) {
        q = max(q, (cell.y + (dir.y < 0.0 ? 1.0 : 0.0) - lamp.y) / dir.y);
    }
    return min(q, t);
}

// Adds a closed run to the kept ones; false once nothing behind it can matter.
bool Keep(inout float4 runs[kRuns], inout int count, float4 run, float zl)
{
    run.x = max(run.x, 0.25); // a run that begins at the lamp: no division by zero
    float fall = (zl - run.w) / run.y;
    float rise = (zl - run.w) / run.x;
    int n = min(count, kRuns);
    for (int k = 0; k < n; ++k) {
        float4 o = runs[k];
        if (o.z <= 0.0 && (zl - o.w) / o.y <= fall && (zl - o.w) / o.x <= rise) {
            return true;
        }
    }
    if (count < kRuns) {
        runs[count] = run;
    } else {
        float4 last = runs[kRuns - 1];
        runs[kRuns - 1] = float4(last.x, run.y, min(last.z, run.z), max(last.w, run.w));
    }
    ++count;
    return !(run.z <= 0.0 && run.w >= kInfiniteTop);
}

RowOutput main(SurfaceLightVaryings i)
{
    float angle = i.scene.x * kTwoPi; // texel a at (a + 0.5) / angles
    float2 dir = float2(cos(angle), sin(angle));
    float2 lamp = ROW_GROUND(i);
    float zl = ROW_HEIGHT(i);
    float stepLength = Domain.z;
    float2 w = float2(dot(WorldFromMask.xy, dir), dot(WorldFromMask.zw, dir));
    int steps = min(int(i.positionRadius.w / length(w) / stepLength), kMaxSteps);

    float2 own = IntervalAt(lamp);
    bool skipping = own.y >= own.x && own.x <= zl && zl <= own.y;

    float4 runs[kRuns];
    for (int k = 0; k < kRuns; ++k) {
        runs[k] = float4(0.0, 0.0, 0.0, 0.0);
    }
    int count = 0;
    float2 current = float2(1.0, -1.0);
    float begin = 0.0;
    float t = 0.0;
    [loop] for (int s = 1; s <= steps; ++s) {
        t = float(s) * stepLength;
        float2 here = IntervalAt(lamp + dir * t);
        if (skipping) {
            if (all(here == own)) {
                here = float2(1.0, -1.0);
            } else {
                skipping = false;
            }
        }
        if (any(here != current)) {
            float edge = Entry(lamp, dir, t, stepLength);
            if (current.y >= current.x && !Keep(runs, count, float4(begin, edge, current), zl)) {
                current = float2(1.0, -1.0);
                break;
            }
            current = here;
            begin = edge;
        }
    }
    if (current.y >= current.x) {
        Keep(runs, count, float4(begin, t, current), zl);
    }
    RowOutput o;
    o.first = float4(PackRun(runs[0]), PackRun(runs[1]));
    o.second = float4(PackRun(runs[2]), PackRun(runs[3]));
    return o;
}
