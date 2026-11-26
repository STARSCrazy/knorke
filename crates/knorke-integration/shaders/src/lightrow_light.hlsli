// Light rows (ADR 0019): the light with the shadow looked up in the lamp's row. Two texels per angle hold the four
// runs (rn, rf, base, top); a run blocks if the heights the ray passes over its piece before the receiver meet its
// interval (ADR 0019, "Die Lichtzeile"). As in Chiaro's trace the last texel before the receiver does not count.
// A lamp with a size reads two more texels, written by lightrow_edge.frag, and is soft along and across the ray.
// A partly transparent run that blocks multiplies the light by its transmittance, once per run as Chiaro once per
// crossing (shadow_trace.hlsli). For a lamp with a size that is decided on the ray to the lamp's middle and the result
// is the smaller of it and the opaque runs' sight, as Chiaro's min(visibility, transmittance): Chiaro's thin fringe
// outside such a shadow (opening narrowed by the opacity) is left out, its edge stays hard.
// lightrow_light.frag, lightrow_light_exact.frag and lightrow_light_direction.frag are this file for the three
// target layouts of the light pass (surface_light.hlsli).
// The profile atlas after all other samplers, as in Chiaro's light_sdf_surface.frag: a lamp without a profile does
// not read it (profile.hlsli).
#ifdef CHI_MATERIAL_EXACT
#define CHI_PROFILE_SLOT 8
#else
#define CHI_PROFILE_SLOT 7
#endif
#include "lightrow_surface.hlsli"

Texture2D<float4> RowFirst : register(t3, space2);
Texture2D<float4> RowSecond : register(t4, space2);
SamplerState RowFirstSampler : register(s3, space2);  // nearest
SamplerState RowSecondSampler : register(s4, space2); // nearest
Texture2D<float4> EdgeFirst : register(t5, space2);   // lightrow_edge.frag; read for lamps with a size only
Texture2D<float4> EdgeSecond : register(t6, space2);
SamplerState EdgeFirstSampler : register(s5, space2);  // nearest
SamplerState EdgeSecondSampler : register(s6, space2); // nearest

#include "lightrow_runs.hlsli"

// slope: the ray's height at distance q from the lamp is zl - slope * q.
bool Blocks(float4 run, float zl, float slope, float r)
{
    if (run.y <= 0.0 || run.x >= r - RowSize.z) {
        return false; // empty slot, or the run begins behind the place
    }
    float a = zl - slope * run.x;
    float b = zl - slope * min(run.y, r - RowSize.z);
    return max(a, b) >= run.z && min(a, b) < run.w - RowSize.w;
}

// The lamp heights from which a run hides the place (r, z), as (from, to); empty if from >= to. The ray from height
// c' reaches height c at distance q if c' = (c - z q / r) / (1 - q / r); over the run's piece that is a stretch
// of heights between the values at its two ends. With the lamp's own height this is `Blocks`.
float2 Hidden(float4 run, float z, float r)
{
    if (run.y <= 0.0 || run.x >= r - RowSize.z) {
        return float2(1e30, 1e30);
    }
    float2 q = float2(run.x, min(run.y, r - RowSize.z)) / r;
    float2 from = (run.z - z * q) / (1.0 - q);
    float2 to = (run.w - RowSize.w - z * q) / (1.0 - q);
    return float2(min(from.x, from.y), max(to.x, to.y));
}

// What the blocking runs let through: all of them (`hard`), or the partly transparent ones only.
float Through(float4 runs[kRuns], float4 pass, float zl, float slope, float r, bool hard)
{
    float through = 1.0;
    for (int k = 0; k < kRuns; ++k) {
        through *= (hard || pass[k] > 0.0) && Blocks(runs[k], zl, slope, r) ? pass[k] : 1.0;
    }
    return through;
}

static const float kFar = 1e30;

// The part of the lamp a run hides from the place (r, z), as (across from, across to, share of the heights): the
// lamp is a square, across the ray in mask texels within +-wide, upright within zl +- size. `angles` are the
// angles from the place's ray to the obstacle's outline on either side, `along` where on the run (0 near end, 1 far
// end) each outline stands; a lamp point x texels across sees the place past an outline at distance d
// x (1 - d / r) / d off that ray.
float3 Part(float4 run, float2 angles, float2 along, float zl, float size, float wide, float z, float r)
{
    float2 h = Hidden(run, z, r);
    float share = (min(h.y, zl + size) - max(h.x, zl - size)) / (2.0 * size);
    if (share <= 0.0) {
        return float3(0.0, 0.0, 0.0);
    }
    float2 d = max(lerp(run.x, min(run.y, r - RowSize.z), along), 0.25);
    return float3(clamp(angles * d * r / (r - d), -wide, wide), share);
}

// The hidden area of one half of the lamp, as a length across times a share of the heights. The row's runs hide
// from the middle out to reach[k] with share[k]; the neighbouring angles' run hides from `begin` out to `wide` with
// `other`. Where several hide, the largest share counts: obstacles standing on the ground hide the lamp from below
// upward, so the shares lie in one another (not so for floating ones, which is accepted).
float Half(float reach[kRuns], float share[kRuns], float begin, float other, float wide)
{
    for (int p = 0; p < kRuns - 1; ++p) {
        for (int q = 0; q < kRuns - 1 - p; ++q) {
            if (reach[q + 1] > reach[q]) { // farthest first
                float t = reach[q];
                reach[q] = reach[q + 1];
                reach[q + 1] = t;
                t = share[q];
                share[q] = share[q + 1];
                share[q + 1] = t;
            }
        }
    }
    float area = 0.0;
    float outer = wide;
    float most = 0.0;
    for (int k = 0; k <= kRuns; ++k) {
        float inner = k < kRuns ? reach[k] : 0.0;
        float beyond = max(0.0, outer - max(inner, begin)); // the piece of [inner, outer] the other run hides too
        area += beyond * max(most, other) + (outer - inner - beyond) * most;
        most = k < kRuns ? max(most, share[k]) : most;
        outer = inner;
    }
    return area;
}

// The lit part of a lamp with a size: one minus the part of its square the runs hide together. The row's four runs
// reach as far across as the edge pass counted; the nearest run of the angles to the left and to the right that
// this angle does not hold begins where it was found.
float SoftSight(SurfaceLightVaryings l, float2 toPlace, float turn, float z, float r)
{
    float zl = ROW_HEIGHT(l);
    float size = ROW_SIZE(l);
    float2 across = float2(-toPlace.y, toPlace.x) / r;
    float wide = size / length(float2(dot(WorldFromMask.xy, across), dot(WorldFromMask.zw, across)));
    float a = floor(turn);
    float f = turn - a;
    float4 runs[kRuns];
    float4 pass;
    Runs(a, ROW_SLOT(l), runs, pass);
    float through = Through(runs, pass, zl, (zl - z) / r, r, false);
    OpaqueOnly(runs, pass);
    float2 uv = float2((a + 0.5) / RowSize.x, (ROW_SLOT(l) + 0.5) / RowSize.y);
    float4 first = EdgeFirst.SampleLevel(EdgeFirstSampler, uv, 0.0);
    float angle = kTwoPi / RowSize.x;
    float left[kRuns];
    float right[kRuns];
    float share[kRuns];
    float where = first.w;
    bool some = false;
    for (int k = 0; k < kRuns; ++k) {
        float both = k < 2 ? first.x : first.y;
        both = (k & 1) != 0 ? floor(both / 4096.0) : both;
        float2 same = float2(fmod(both, 64.0), fmod(floor(both / 64.0), 64.0)); // to the left, to the right
        float2 along = float2(fmod(where, 8.0), fmod(floor(where / 8.0), 8.0)) / 7.0;
        where = floor(where / 64.0);
        float2 angles = float2(same.x >= kEdgeFar ? -kFar : -(same.x + f) * angle,
                               same.y >= kEdgeFar ? kFar : (same.y + 1.0 - f) * angle);
        float3 part = Part(runs[k], angles, along, zl, size, wide, z, r);
        if (part.z >= 0.9999 && part.x <= -wide && part.y >= wide) {
            return 0.0; // most shadowed places: one run hides all of the lamp
        }
        left[k] = -part.x;
        right[k] = part.y;
        share[k] = part.z;
        some = some || part.z > 0.0;
    }
    float2 begin = float2(wide, wide); // of the neighbouring angles' runs, on the left and on the right half
    float2 other = float2(0.0, 0.0);
    if (first.z > 0.0) { // most angles have no such run: the second texel is read for the others only
        float4 second = EdgeSecond.SampleLevel(EdgeSecondSampler, uv, 0.0);
        float2 away = float2(fmod(first.z, 64.0), floor(first.z / 64.0));
        float2 middle = float2(0.5, 0.5);
        if (away.x > 0.0) {
            float2 angles = float2(-kFar, (1.0 - away.x - f) * angle);
            float3 part = Part(UnpackRun(second.xy), angles, middle, zl, size, wide, z, r);
            begin.x = -part.y;
            other.x = part.z;
        }
        if (away.y > 0.0) {
            float2 angles = float2((away.y - f) * angle, kFar);
            float3 part = Part(UnpackRun(second.zw), angles, middle, zl, size, wide, z, r);
            begin.y = part.x;
            other.y = part.z;
        }
    }
    if (!some && all(other <= 0.0)) {
        return through; // most lit places: nothing hides any of it
    }
    float area = Half(left, share, begin.x, other.x, wide) + Half(right, share, begin.y, other.y, wide);
    return min(through, 1.0 - area / (2.0 * wide));
}

float Sight(SurfaceLightVaryings l, float2 start, float z, float r)
{
    float2 d = start - ROW_GROUND(l);
    float turn = frac(atan2(d.y, d.x) / kTwoPi) * RowSize.x; // texel a holds the ray at a + 0.5
    if (ROW_SIZE(l) > 0.0) {
        return SoftSight(l, d, turn, z, r);
    }
    float zl = ROW_HEIGHT(l);
    float slope = (zl - z) / r;
    float4 runs[kRuns];
    float4 pass;
    Runs(floor(turn), ROW_SLOT(l), runs, pass);
    return Through(runs, pass, zl, slope, r, true);
}
