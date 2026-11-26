// Light rows (ADR 0019): builds one lamp's row, one texel per angle.
// Marches once outward from the lamp's ground point over the height texture of the occlusion mask, to the lamp's
// radius. Consecutive texels with the same height interval are one run (rn, rf, base, top). A run is dropped if a
// nearer run standing on the ground already blocks every slope it would block; of more than four, two pieces of one
// split obstacle become one (`Squeeze`), else the last slot takes the hull of the rest, which can only hide too
// much. The run the lamp stands in is skipped; a window lamp's march begins at its window (`WindowStart`).
// A partly transparent run (E25: mask opacity below kOpaqueOpacity) carries the transmittance 1 - o of its first texel,
// as Chiaro reads o where a crossing enters (the mask's rim texels of an obstacle may hold less); it never ends the
// march and never hides a farther run. An angle with one keeps three runs (squeezed or hulled as above), the fourth
// slot holds the transmittances (lightrow_runs.hlsli).
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

float Cross(float2 a, float2 b)
{
    return a.x * b.y - a.y * b.x;
}

// A window lamp's row (RowLamp.window, carried as i.area and i.areaMask) begins where the ray enters the window's
// rectangle on the ground, plus the 1.5 texels by which Chiaro's ray stops short of a window (MaskShadowStop in
// light_common.hlsli): nothing nearer the lamp counts for a place the window lights (lightrow_light.hlsli clips at
// that place's own crossing), and an infinitely tall wall the window sits in must not end the march. An upright
// window's rectangle is a line on the ground, met at one distance; a tilted one is met over a stretch, (near end,
// far end): a run beginning within it counts for some places only, so it neither hides a farther run nor ends the
// march (`Keep`). 0 without a window, or where the ray misses a tilted one; past every place a ray behind an upright
// one reaches (the window lights none of them).
float2 WindowStart(float4 window, float4 windowV, float2 dir)
{
    float2 c = window.xy;
    float2 u = window.zw;
    float2 v = windowV.xy;
    if (dot(u, u) + dot(v, v) <= 0.0) {
        return float2(0.0, 0.0);
    }
    float area = Cross(u, v);
    if (abs(area) < 0.25) {
        float2 l = dot(u, u) >= dot(v, v) ? u : v;
        float s = Cross(dir, l);
        float q = abs(s) > 1e-6 ? Cross(c, l) / s : -1.0;
        return (q > 0.0 ? q + 1.5 : 1e30).xx;
    }
    // c + a u + b v with |a|, |b| <= 1: |q k - m| <= |area| for (k, m) of a and of b.
    float2 range = float2(0.0, 1e30);
    float2 k = float2(Cross(dir, v), Cross(u, dir));
    float2 m = float2(Cross(c, v), Cross(u, c));
    for (int j = 0; j < 2; ++j) {
        if (abs(k[j]) > 1e-6) {
            float2 q = (m[j] + float2(-1.0, 1.0) * abs(area)) / k[j];
            range = float2(max(range.x, min(q.x, q.y)), min(range.y, max(q.x, q.y)));
        } else if (abs(m[j]) > abs(area)) {
            range = float2(1.0, 0.0);
        }
    }
    return range.x <= range.y ? range + 1.5 : float2(0.0, 0.0);
}

float4 Hull(float4 near, float4 far)
{
    return float4(near.x, far.y, min(near.z, far.z), max(near.w, far.w));
}

// Frees one of the first n slots by joining two neighbouring runs that are one obstacle split: a ray grazing an
// outline steps out of the mask's staircase and back in, which leaves two runs of one interval and kind within two
// texels. The joined run lets through the product, as the two crossings do in Chiaro's trace. Used only when the
// slots overflow, where the hull behind a split partly transparent obstacle (a low building and the facade beyond
// it) hid the facade from all of the light. Failing such a pair, two partly transparent runs that touch become their
// hull: cells of one tile grid with different heights (Chiaro's trace crosses them once where the ray stays inside;
// the hull may hide a little too much over the lower one, where the last slot's hull with an opaque run behind
// would hide everything there). False if no two runs are such a pair.
bool Squeeze(inout float4 runs[kRuns], inout float pass[kRuns], int n)
{
    int k = -1;
    for (int j = 0; j + 1 < n; ++j) {
        if (all(runs[j].zw == runs[j + 1].zw) && (pass[j] > 0.0) == (pass[j + 1] > 0.0)
            && runs[j + 1].x <= runs[j].y + 2.0) {
            k = j;
            break;
        }
        if (k < 0 && pass[j] > 0.0 && pass[j + 1] > 0.0 && runs[j + 1].x <= runs[j].y + 0.5) {
            k = j;
        }
    }
    if (k < 0) {
        return false;
    }
    runs[k] = Hull(runs[k], runs[k + 1]);
    pass[k] *= pass[k + 1];
    for (int j = k + 1; j + 1 < n; ++j) {
        runs[j] = runs[j + 1];
        pass[j] = pass[j + 1];
    }
    runs[n - 1] = float4(0.0, 0.0, 0.0, 0.0);
    pass[n - 1] = 0.0;
    return true;
}

// Adds a closed run that lets `through` of the light pass to the kept ones; false once nothing behind it can matter.
// The hull lets through the product of what it took: opaque if any of it is. A run beginning before `whole` (the far
// end of a tilted window's stretch) may be cut short by the light, so it hides nothing.
bool Keep(inout float4 runs[kRuns], inout float pass[kRuns], inout int count, float4 run, float through, float zl,
          float whole)
{
    run.x = max(run.x, 0.25); // a run that begins at the lamp: no division by zero
    float fall = (zl - run.w) / run.y;
    float rise = (zl - run.w) / run.x;
    int n = min(count, kRuns);
    for (int k = 0; k < n; ++k) {
        float4 o = runs[k];
        if (pass[k] <= 0.0 && o.z <= 0.0 && o.x >= whole && (zl - o.w) / o.y <= fall && (zl - o.w) / o.x <= rise) {
            return true;
        }
    }
    if (count == kRuns && Squeeze(runs, pass, kRuns)) {
        --count;
    }
    if (count < kRuns) {
        runs[count] = run;
        pass[count] = through;
    } else {
        runs[kRuns - 1] = Hull(runs[kRuns - 1], run);
        pass[kRuns - 1] *= through;
    }
    ++count;
    return !(through <= 0.0 && run.z <= 0.0 && run.w >= kInfiniteTop && run.x >= whole);
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

    float2 window = WindowStart(i.area, i.areaMask, dir);
    float start = window.x;
    float3 own = SampleAt(lamp);
    bool skipping = start <= 0.0 && own.y >= own.x && own.x <= zl && zl <= own.y;

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
    [loop] for (int s = max(1, int(ceil(min(start, 1e6) / stepLength))); s <= steps; ++s) {
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
            if (current.y >= current.x && !Keep(runs, pass, count, float4(begin, edge, current.xy), current.z, zl, window.y)) {
                current = kNothing;
                break;
            }
            current = here;
            begin = edge;
        }
    }
    if (current.y >= current.x) {
        Keep(runs, pass, count, float4(begin, t, current.xy), current.z, zl, window.y);
    }
    bool partial = pass[0] > 0.0 || pass[1] > 0.0 || pass[2] > 0.0 || pass[3] > 0.0;
    if (partial && count > 3 && !Squeeze(runs, pass, kRuns)) {
        runs[2] = Hull(runs[2], runs[3]);
        pass[2] *= pass[3];
    }
    RowOutput o;
    o.first = float4(PackRun(runs[0]), PackRun(runs[1]));
    o.second = float4(PackRun(runs[2]), partial ? PackPasses(pass[0], pass[1], pass[2]) : PackRun(runs[3]));
    o.first.x = partial ? -1.0 - o.first.x : o.first.x;
    return o;
}
