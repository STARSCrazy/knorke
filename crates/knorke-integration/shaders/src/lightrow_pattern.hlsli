// Light rows (ADR 0019): pattern obstacles (fences, grilles; Chiaro's pattern.hlsli) in the rows. Chiaro reads a
// pattern where a ray coming from the receiver enters the obstacle, at u along its outline and at the ray's height
// there. u and the distance of that point from the lamp depend on lamp and angle only, so the pattern pass
// (lightrow_pattern.frag) finds them once per angle with `PatternExit`; the light reads the pattern at the height
// its own ray has at that distance (`PatternOpacity`). Tile grids are left to Chiaro's programs (`RowBlockers`).
// A program includes it once for the records, declares its bindings, then includes it again with
// ROW_PATTERN_GEOMETRY (needs PatternObstacles, PatternData, PatternMaskToWorldX/Y) or ROW_PATTERN_LIGHT (needs
// PatternObstacles, Patterns, PatternAtlas with PatternAtlasSampler, PatternAtlasSize) defined.
// Origin of the records, PatternIsFinite, SamplePatternAtlas, the entry steps of `PatternExit` and the end of
// `PatternOpacity`: chiaro @ 9bc5b9b, shaders/src/pattern.hlsli (PatternEntryBox, PatternRectEntry,
// PatternEntryCircle, PatternEntryPolygon, PatternEdgeU, PatternArcU, PatternCrossingOpacity). Copyright (C) 2026
// Manuel Vögele. zlib license: this software is provided 'as-is', without any express or implied warranty; the
// origin must not be misrepresented; altered source versions must be plainly marked as such (this file is altered:
// the entry of a whole crossing seen from its far end, no nearest-point fallback, no tile grids) and this notice
// may not be removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.
#ifndef KNORKE_LIGHTROW_PATTERN_HLSLI
#define KNORKE_LIGHTROW_PATTERN_HLSLI

#include "light_common.hlsli"

static const uint kPatternBox = 0u;
static const uint kPatternPolygon = 1u;
static const uint kPatternCircle = 2u;
static const uint kPatternFiniteHeight = 1u;
static const float kPatternTiny = 1e-12;
static const float kPatternBelowOne = 0.99999994;

// `GpuPattern` of knorke-light2d (pattern_occluders).
struct GpuPattern
{
    uint4 rect;    // xy atlas texel of the top-left texel, zw width, height
    uint4 mean;    // xy atlas texel of column 0's mean
    float4 repeat; // x repeat width, y repeat height (0: stretched over the height span)
};

// `GpuPatternObstacle` of knorke-light2d (pattern_occluders); the id texture holds 1 + its index.
struct GpuPatternObstacle
{
    uint4 kind;       // x kPattern*, y kPatternFiniteHeight, z pattern as 1 + index into Patterns, 0 none
    float4 transform; // xy world origin, zw cos and sin of the rotation: local = R^T (world - origin)
    float4 shape;     // box: half extents; circle: radius
    float4 heights;   // x base, y top, w the obstacle's opacity
    uint4 data;       // polygon: x first corner word in PatternData, y corner count
    uint4 table;
};

#endif

#if defined(ROW_PATTERN_GEOMETRY) && !defined(KNORKE_LIGHTROW_PATTERN_GEOMETRY)
#define KNORKE_LIGHTROW_PATTERN_GEOMETRY

float2 PatternToLocal(GpuPatternObstacle o, float2 mask)
{
    float2 d = float2(dot(PatternMaskToWorldX.xyz, float3(mask, 1.0)), dot(PatternMaskToWorldY.xyz, float3(mask, 1.0)))
               - o.transform.xy;
    return float2(o.transform.z * d.x + o.transform.w * d.y, -o.transform.w * d.x + o.transform.z * d.y);
}

float2 PatternCorner(uint first, uint i)
{
    return asfloat(uint2(PatternData[first + 2u * i], PatternData[first + 2u * i + 1u]));
}

float PatternCross(float2 p, float2 q)
{
    return p.x * q.y - p.y * q.x;
}

float PatternArcU(float2 p, float radius)
{
    float angle = atan2(p.y, p.x);
    return (angle < 0.0 ? angle + 6.28318530717958647692 : angle) * radius;
}

// u along edge e at p, the direction sign-normalised so opposite edges of a thin obstacle agree.
float PatternEdgeU(float2 p, float2 e)
{
    float len = length(e);
    float2 dir = len < kPatternTiny ? float2(0.0, 0.0) : e / len;
    dir = (abs(dir.x) < 1e-6 ? dir.y < 0.0 : dir.x < 0.0) ? -dir : dir;
    return dot(p, dir);
}

// Where the ray lamp + dir * q (mask texels) enters obstacle `id` (1 + index into PatternObstacles) coming back from
// qFar towards qNear, as Chiaro's ray from a receiver beyond does: (q of that point, u along the outline). Without
// an entry (the piece starts inside, or the mask's texels hold more than the outline), u of the outline point
// nearest to qNear and qNear, as Chiaro's fallback.
float2 PatternExit(uint id, float2 lamp, float2 dir, float qNear, float qFar)
{
    GpuPatternObstacle o = PatternObstacles[id - 1u];
    float2 a = PatternToLocal(o, lamp + dir * qFar);
    float2 b = PatternToLocal(o, lamp + dir * qNear);
    float2 d = b - a;
    float t = 2.0; // above 1: no entry
    float u = 0.0;
    if (o.kind.x == kPatternBox) {
        float2 hi = o.shape.xy;
        float enter = -1.0;
        float leave = 2.0;
        int axis = -1;
        [unroll] for (int k = 0; k < 2; ++k) {
            if (abs(d[k]) < kPatternTiny) {
                leave = abs(a[k]) > hi[k] ? -1.0 : leave; // parallel to this slab and outside it
                continue;
            }
            float2 s = (float2(-hi[k], hi[k]) - a[k]) / d[k];
            if (min(s.x, s.y) > enter) {
                enter = min(s.x, s.y);
                axis = k;
            }
            leave = min(leave, max(s.x, s.y));
        }
        float2 p = clamp(b, -hi, hi);
        float2 off = abs(b - p);
        float2 inner = hi - abs(b);
        bool xSide = any(off > 0.0) ? off.x >= off.y : inner.x <= inner.y; // the face nearest to b
        if (axis >= 0 && enter >= 0.0 && enter <= 1.0 && enter <= leave) {
            t = enter;
            p = a + t * d;
            xSide = axis == 0;
        }
        u = xSide ? p.y + hi.y : p.x + hi.x;
    } else if (o.kind.x == kPatternCircle) {
        float r = o.shape.x;
        float qa = dot(d, d);
        float qb = dot(a, d);
        float qc = dot(a, a) - r * r; // >= 0: a outside (or on) the circle
        float disc = qb * qb - qa * qc;
        float2 p = length(b) < kPatternTiny ? float2(r, 0.0) : b;
        if (qa >= kPatternTiny && qc >= 0.0 && disc >= 0.0 && -qb - sqrt(disc) >= 0.0 && -qb - sqrt(disc) <= qa) {
            t = (-qb - sqrt(disc)) / qa;
            p = a + t * d;
        }
        u = PatternArcU(p, r);
    } else if (o.kind.x == kPatternPolygon) {
        uint first = o.data.x;
        uint n = o.data.y;
        float area2 = 0.0; // twice the signed area: > 0 counter-clockwise
        [loop] for (uint i = 0u; i < n; ++i) {
            area2 += PatternCross(PatternCorner(first, i), PatternCorner(first, i + 1u < n ? i + 1u : 0u));
        }
        float nearest = -1.0;
        [loop] for (uint j = 0u; j < n; ++j) {
            float2 v0 = PatternCorner(first, j);
            float2 e = PatternCorner(first, j + 1u < n ? j + 1u : 0u) - v0;
            float denom = PatternCross(d, e);
            // Entering through this edge (not parallel, not leaving the polygon), else its point nearest to b.
            if (abs(denom) >= kPatternTiny && denom * area2 < 0.0) {
                float tj = PatternCross(v0 - a, e) / denom;
                float s = PatternCross(v0 - a, d) / denom;
                if (tj >= 0.0 && tj <= 1.0 && s >= 0.0 && s <= 1.0 && tj < t) {
                    t = tj;
                    u = PatternEdgeU(a + t * d, e);
                }
            }
            float2 p = v0 + clamp(dot(b - v0, e) / max(dot(e, e), kPatternTiny), 0.0, 1.0) * e;
            if (t > 1.0 && (nearest < 0.0 || dot(b - p, b - p) < nearest)) {
                nearest = dot(b - p, b - p);
                u = PatternEdgeU(p, e);
            }
        }
    }
    return float2(t <= 1.0 ? lerp(qFar, qNear, t) : qNear, u);
}

#endif

#if defined(ROW_PATTERN_LIGHT) && !defined(KNORKE_LIGHTROW_PATTERN_LIGHT)
#define KNORKE_LIGHTROW_PATTERN_LIGHT

// std::isfinite as an explicit bit test (Chiaro: the compilers may fold isfinite()).
bool PatternIsFinite(float x)
{
    return (asuint(x) & 0x7F800000u) != 0x7F800000u;
}

// SamplePattern on the atlas: u, v in repeats; without height the column mean at u. [0, 1].
float SamplePatternAtlas(GpuPattern p, float u, float v, bool hasHeight)
{
    float fu = PatternIsFinite(u) ? u - floor(u) : 0.0;
    int w = int(p.rect.z);
    int x = clamp(int(fu * float(w)), 0, w - 1);
    if (!hasHeight) {
        return FetchTexel(PatternAtlas, PatternAtlasSampler, int2(p.mean.xy) + int2(x, 0), PatternAtlasSize.xy).r;
    }
    float fv = PatternIsFinite(v) ? v - floor(v) : 0.0;
    int h = int(p.rect.w);
    int fromBottom = clamp(int(fv * float(h)), 0, h - 1);
    return FetchTexel(PatternAtlas, PatternAtlasSampler, int2(p.rect.xy) + int2(x, h - 1 - fromBottom),
                      PatternAtlasSize.xy).r;
}

// The share of the light a crossing of obstacle `id` at `u` along its outline lets through, for a ray at height z
// there; `pass` (the mask's) where nothing patterns it.
float PatternPass(uint id, float u, float z, float pass)
{
    GpuPatternObstacle o = PatternObstacles[id - 1u];
    if (o.kind.z == 0u) {
        return pass;
    }
    GpuPattern p = Patterns[o.kind.z - 1u];
    float base = o.heights.x;
    float top = o.heights.y;
    bool hasHeight = (o.kind.y & kPatternFiniteHeight) != 0u && top > base;
    float v = 0.0;
    if (hasHeight) {
        // A ray that enters through the top (or bottom) reads the pattern at the edge it passes.
        float h = clamp(z, base, top) - base;
        v = p.repeat.y > 0.0 ? h / p.repeat.y : min(h / (top - base), kPatternBelowOne);
    }
    return 1.0 - SamplePatternAtlas(p, u / p.repeat.x, v, hasHeight) * o.heights.w;
}

#endif
