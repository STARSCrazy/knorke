// Light rows (ADR 0019): builds one lamp's row, one texel per angle.
// Marches once outward from the lamp's ground point over the height texture of the occlusion mask, to the lamp's
// radius. Consecutive texels with the same height interval are one run (rn, rf, base, top). A run is dropped if a
// nearer run standing on the ground already blocks every slope it would block; of more than four the last slot
// takes the hull of the rest, which can only hide too much. The run the lamp stands in is skipped.
// A partly transparent run (E25: mask opacity below kOpaqueOpacity) carries the transmittance 1 - o of its first texel,
// as Chiaro reads o where a crossing enters (the mask's rim texels of an obstacle may hold less); it never ends the
// march and never hides a farther run. An angle with one keeps three runs,
// the fourth slot holds the transmittances (lightrow_runs.hlsli).
// An edge lies where the ray enters
// the texel of the first sample with another interval, not on the sample.
#include "lightrow.hlsli"

Texture2D<float4> Heights : register(t0, space2); // RGBA16F (-base, top); free texels hold -65504
Texture2D<float4> Mask : register(t1, space2);    // opacity in r; read only in frames with a partly transparent one
SamplerState HeightSampler : register(s0, space2); // nearest
SamplerState MaskSampler : register(s1, space2);   // nearest

cbuffer RowParams : register(b0, space3)
{
    float4 WorldFromMask; // world = (dot(xy, m), dot(zw, m)) for a vector m in mask texels
    float4 Domain;        // xy mask size in texels, z march step in texels, w 1 if an obstacle is partly transparent
};

struct RowOutput
{
    float4 first : SV_Target0;  // runs 0 and 1
    float4 second : SV_Target1; // runs 2 and 3
};

static const int kMaxSteps = 1200;

static const float3 kNothing = float3(1.0, -1.0, 0.0);

// (base, top, transmittance) of the texel under p; top below base where nothing stands or p is outside the mask,
// transmittance 0 where the obstacle is opaque.
float3 SampleAt(float2 p)
{
    if (any(p < 0.0) || any(p >= Domain.xy)) {
        return kNothing;
    }
    float2 uv = (floor(p) + 0.5) / Domain.xy;
    float2 i = Heights.SampleLevel(HeightSampler, uv, 0.0).rg;
    if (!(i.y > -i.x)) {
        return kNothing;
    }
    float o = Domain.w > 0.5 ? Mask.SampleLevel(MaskSampler, uv, 0.0).r : 1.0;
    return float3(max(-i.x, 0.0), i.y, o >= kOpaqueOpacity ? 0.0 : 1.0 - o);
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

float4 Hull(float4 near, float4 far)
{
    return float4(near.x, far.y, min(near.z, far.z), max(near.w, far.w));
}

// Adds a closed run that lets `through` of the light pass to the kept ones; false once nothing behind it can matter.
// The hull lets through the product of what it took: opaque if any of it is.
bool Keep(inout float4 runs[kRuns], inout float pass[kRuns], inout int count, float4 run, float through, float zl)
{
    run.x = max(run.x, 0.25); // a run that begins at the lamp: no division by zero
    float fall = (zl - run.w) / run.y;
    float rise = (zl - run.w) / run.x;
    int n = min(count, kRuns);
    for (int k = 0; k < n; ++k) {
        float4 o = runs[k];
        if (pass[k] <= 0.0 && o.z <= 0.0 && (zl - o.w) / o.y <= fall && (zl - o.w) / o.x <= rise) {
            return true;
        }
    }
    if (count < kRuns) {
        runs[count] = run;
        pass[count] = through;
    } else {
        runs[kRuns - 1] = Hull(runs[kRuns - 1], run);
        pass[kRuns - 1] *= through;
    }
    ++count;
    return !(through <= 0.0 && run.z <= 0.0 && run.w >= kInfiniteTop);
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

    float3 own = SampleAt(lamp);
    bool skipping = own.y >= own.x && own.x <= zl && zl <= own.y;

    float4 runs[kRuns];
    float pass[kRuns];
    for (int k = 0; k < kRuns; ++k) {
        runs[k] = float4(0.0, 0.0, 0.0, 0.0);
        pass[k] = 0.0;
    }
    int count = 0;
    float3 current = kNothing;
    float begin = 0.0;
    float t = 0.0;
    [loop] for (int s = 1; s <= steps; ++s) {
        t = float(s) * stepLength;
        float3 here = SampleAt(lamp + dir * t);
        if (skipping) {
            if (all(here == own)) {
                here = kNothing;
            } else {
                skipping = false;
            }
        }
        if (any(here.xy != current.xy) || (here.z > 0.0) != (current.z > 0.0)) {
            float edge = Entry(lamp, dir, t, stepLength);
            if (current.y >= current.x && !Keep(runs, pass, count, float4(begin, edge, current.xy), current.z, zl)) {
                current = kNothing;
                break;
            }
            current = here;
            begin = edge;
        }
    }
    if (current.y >= current.x) {
        Keep(runs, pass, count, float4(begin, t, current.xy), current.z, zl);
    }
    bool partial = pass[0] > 0.0 || pass[1] > 0.0 || pass[2] > 0.0 || pass[3] > 0.0;
    if (partial) {
        runs[2] = count > 3 ? Hull(runs[2], runs[3]) : runs[2];
        pass[2] *= count > 3 ? pass[3] : 1.0;
    }
    RowOutput o;
    o.first = float4(PackRun(runs[0]), PackRun(runs[1]));
    o.second = float4(PackRun(runs[2]), partial ? PackPasses(pass[0], pass[1], pass[2]) : PackRun(runs[3]));
    o.first.x = partial ? -1.0 - o.first.x : o.first.x;
    return o;
}
