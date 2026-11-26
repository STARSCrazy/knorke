// Light rows (ADR 0019): pattern obstacles (fences, grilles; Chiaro's pattern.hlsli) in the rows. Chiaro reads a
// pattern where a ray coming from the receiver enters the obstacle, at u along its outline and at the ray's height
// there. u and the distance of that point from the lamp depend on lamp and angle only, so the pattern pass
// (lightrow_pattern.frag) finds them once per angle with `PatternExit`; the light reads the pattern at the height
// its own ray has at that distance (`PatternPass`). A tile grid's crossing also names the entered cell, whose
// word gives pattern, opacity and heights; `PatternExit` seeks it among the solid cells around the piece, as
// Chiaro's PatternEntryGrid does around its hit sample.
// A program includes it once for the records, declares its bindings, then includes it again with
// ROW_PATTERN_GEOMETRY (needs PatternObstacles, PatternData, PatternMaskToWorldX/Y) or ROW_PATTERN_LIGHT (needs
// PatternObstacles, PatternData, Patterns, PatternAtlas with PatternAtlasSampler, PatternAtlasSize) defined.
// Origin of the records, PatternIsFinite, SamplePatternAtlas, the entry steps of `PatternExit` and the end of
// `PatternPass`: chiaro @ 9bc5b9b, shaders/src/pattern.hlsli (PatternEntryBox, PatternRectEntry, PatternRectNearest,
// PatternEntryCircle, PatternEntryPolygon, PatternEntryTileCell, PatternEntryGrid, PatternOfCell, PatternEdgeU,
// PatternArcU, PatternCrossingOpacity). Copyright (C) 2026
// Manuel Vögele. zlib license: this software is provided 'as-is', without any express or implied warranty; the
// origin must not be misrepresented; altered source versions must be plainly marked as such (this file is altered:
// the entry of a whole crossing seen from its far end, a simpler nearest-point fallback, the grid's cells sought
// around the piece's middle) and this notice
// may not be removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.
#ifndef KNORKE_LIGHTROW_PATTERN_HLSLI
#define KNORKE_LIGHTROW_PATTERN_HLSLI

#include "light_common.hlsli"

static const uint kPatternBox = 0u;
static const uint kPatternPolygon = 1u;
static const uint kPatternCircle = 2u;
static const uint kPatternTileGrid = 3u;
static const uint kPatternNoIndices = 0xFFFFFFFFu; // GpuPatternObstacle.data.w of a grid without cell indices
static const int kPatternGridReach = 4;            // most cells sought from the piece's middle on each side
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
    uint4 kind;       // x kPattern*, y kPatternFiniteHeight, z pattern as 1 + index into Patterns, 0 none (a grid:
                      // the pattern of cells with index 0)
    float4 transform; // xy world origin (a grid: the corner of cell (0, 0)), zw cos and sin of the rotation:
                      // local = R^T (world - origin)
    float4 shape;     // box: half extents; circle: radius; grid: xy cell size, z edge width
    float4 heights;   // x base, y top, z grid: level height (0: every cell spans x .. y), w the obstacle's opacity
    uint4 data;       // polygon: x first corner word in PatternData, y corner count; grid: x columns, y rows,
                      // z first cell word (value | bottom << 8 | cell opacity << 16 | edges << 24, row-major),
                      // w first index word (4 cells a word) or kPatternNoIndices
    uint4 table;      // grid: x first word of the pattern table (1 + index into Patterns), y entries
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

// Entry of the piece a + t d, t in [0, 1], into [lo, hi] (slab method), only if a lies outside; xSide: through an x
// face (on an exact corner the x face wins).
bool PatternRectEntry(float2 lo, float2 hi, float2 a, float2 d, out float t, out bool xSide)
{
    float enter = -1.0;
    float leave = 2.0;
    int axis = -1;
    [unroll] for (int k = 0; k < 2; ++k) {
        if (abs(d[k]) < kPatternTiny) {
            leave = a[k] < lo[k] || a[k] > hi[k] ? -1.0 : leave; // parallel to this slab and outside it
            continue;
        }
        float2 s = (float2(lo[k], hi[k]) - a[k]) / d[k];
        if (min(s.x, s.y) > enter) {
            enter = min(s.x, s.y);
            axis = k;
        }
        leave = min(leave, max(s.x, s.y));
    }
    t = enter;
    xSide = axis == 0;
    return axis >= 0 && enter >= 0.0 && enter <= 1.0 && enter <= leave;
}

// The point of [lo, hi] nearest to b (b itself inside) and whether the face nearest to b is an x face; returns the
// squared distance to the outline (0 inside).
float PatternRectNearest(float2 lo, float2 hi, float2 b, out float2 p, out bool xSide)
{
    p = clamp(b, lo, hi);
    float2 off = abs(b - p);
    float2 inner = min(b - lo, hi - b);
    xSide = any(off > 0.0) ? off.x >= off.y : inner.x <= inner.y;
    return dot(off, off);
}

// Piece k (kPatternSide* order: x low, x high, y low, y high) of the cell [lo, hi]: with `whole` the cell itself,
// else the strip w wide along that edge.
void PatternCellPiece(bool whole, int k, float2 lo, float2 hi, float2 w, out float2 pieceLo, out float2 pieceHi)
{
    pieceLo = lo;
    pieceHi = hi;
    if (!whole) {
        if (k == 0) {
            pieceHi.x = lo.x + w.x;
        } else if (k == 1) {
            pieceLo.x = hi.x - w.x;
        } else if (k == 2) {
            pieceHi.y = lo.y + w.y;
        } else {
            pieceLo.y = hi.y - w.y;
        }
    }
}

// Where the ray lamp + dir * q (mask texels) enters obstacle `id` (1 + index into PatternObstacles) coming back from
// qFar towards qNear, as Chiaro's ray from a receiver beyond does: (q of that point, u along the outline, the
// entered cell of a tile grid as a float, else 0). Without an entry (the piece starts inside, or the mask's texels
// hold more than the outline), u of the outline point nearest to qNear and qNear, as Chiaro's fallback. A grid's
// cells are sought within half the piece of its middle (at most kPatternGridReach cells to each side; with cells
// far smaller than a texel the nearest found stands in, as in Chiaro); a whole cell reads u along the entered side
// as a box does, an x strip u = y and a y strip u = x, in grid coordinates.
float3 PatternExit(uint id, float2 lamp, float2 dir, float qNear, float qFar)
{
    GpuPatternObstacle o = PatternObstacles[id - 1u];
    float2 a = PatternToLocal(o, lamp + dir * qFar);
    float2 b = PatternToLocal(o, lamp + dir * qNear);
    float2 d = b - a;
    float t = 2.0; // above 1: no entry
    float u = 0.0;
    uint cell = 0u;
    if (o.kind.x == kPatternBox) {
        float2 hi = o.shape.xy;
        float2 p;
        bool xSide;
        PatternRectNearest(-hi, hi, b, p, xSide);
        float s;
        bool xEntry;
        if (PatternRectEntry(-hi, hi, a, d, s, xEntry)) {
            t = s;
            p = a + t * d;
            xSide = xEntry;
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
    } else if (o.kind.x == kPatternTileGrid) {
        float2 size = o.shape.xy;
        float2 w = clamp(o.shape.zz, float2(0.0, 0.0), size);
        int2 home = int2(floor(0.5 * (a + b) / size));
        int2 reach = clamp(int2(ceil(0.5 * abs(d) / size)), 1, kPatternGridReach);
        float nearest = -1.0;
        [loop] for (int dy = -reach.y; dy <= reach.y; ++dy) {
            [loop] for (int dx = -reach.x; dx <= reach.x; ++dx) {
                int2 c = home + int2(dx, dy);
                if (any(c < 0) || c.x >= int(o.data.x) || c.y >= int(o.data.y)) {
                    continue;
                }
                uint index = uint(c.y) * o.data.x + uint(c.x);
                uint word = PatternData[o.data.z + index];
                if ((word & 0xFFu) == 0u || float((word >> 16u) & 0xFFu) * o.heights.w < 0.5) {
                    continue; // empty, or a cell opacity that rounds to 0 (tile.frag discards both)
                }
                float2 lo = float2(c) * size;
                uint edges = word >> 24u;
                bool whole = edges == 0u || edges > 15u;
                // A piece that starts inside a strip enters none of them (whole: the entry checks a itself).
                bool inside = false;
                [unroll] for (int k = 0; k < 4; ++k) {
                    float2 pLo, pHi;
                    PatternCellPiece(whole, k, lo, lo + size, w, pLo, pHi);
                    inside = inside || (!whole && ((edges >> uint(k)) & 1u) != 0u && all(a >= pLo) && all(a <= pHi));
                }
                [unroll] for (int k = 0; k < 4; ++k) {
                    if (!(whole ? k == 0 : ((edges >> uint(k)) & 1u) != 0u)) {
                        continue;
                    }
                    float2 pLo, pHi;
                    PatternCellPiece(whole, k, lo, lo + size, w, pLo, pHi);
                    float s;
                    bool xSide;
                    if (!inside && PatternRectEntry(pLo, pHi, a, d, s, xSide) && s < t) {
                        float2 p = a + s * d;
                        t = s;
                        u = (whole ? xSide : k < 2) ? p.y : p.x;
                        cell = index;
                    }
                    float2 p;
                    float dist = PatternRectNearest(pLo, pHi, b, p, xSide);
                    if (t > 1.0 && (nearest < 0.0 || dist < nearest)) {
                        nearest = dist;
                        u = (whole ? xSide : k < 2) ? p.y : p.x;
                        cell = index;
                    }
                }
            }
        }
    }
    return float3(t <= 1.0 ? lerp(qFar, qNear, t) : qNear, u, float(cell));
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
// there; `pass` (the mask's) where nothing patterns it. A tile grid's `cell` (PatternExit) picks pattern (its index
// byte: 0 the grid's, else an entry of the grid's table), opacity (the obstacle's times the cell's byte) and, with
// a level height, the heights bottom .. value levels, as Chiaro's PatternCrossingOpacity.
float PatternPass(uint id, float u, uint cell, float z, float pass)
{
    GpuPatternObstacle o = PatternObstacles[id - 1u];
    uint pattern = o.kind.z;
    float opacity = o.heights.w;
    float base = o.heights.x;
    float top = o.heights.y;
    bool finite = (o.kind.y & kPatternFiniteHeight) != 0u;
    if (o.kind.x == kPatternTileGrid) {
        uint word = PatternData[o.data.z + cell];
        uint index = o.data.w == kPatternNoIndices ? 0u
                                                   : (PatternData[o.data.w + cell / 4u] >> (8u * (cell & 3u))) & 0xFFu;
        pattern = index == 0u ? pattern : (index <= o.table.y ? PatternData[o.table.x + index - 1u] : 0u);
        opacity *= float((word >> 16u) & 0xFFu) / 255.0;
        if (o.heights.z > 0.0) {
            base = float((word >> 8u) & 0xFFu) * o.heights.z;
            top = float(word & 0xFFu) * o.heights.z;
            finite = true;
        }
    }
    if (pattern == 0u) {
        return pass;
    }
    GpuPattern p = Patterns[pattern - 1u];
    bool hasHeight = finite && top > base;
    float v = 0.0;
    if (hasHeight) {
        // A ray that enters through the top (or bottom) reads the pattern at the edge it passes.
        float h = clamp(z, base, top) - base;
        v = p.repeat.y > 0.0 ? h / p.repeat.y : min(h / (top - base), kPatternBelowOne);
    }
    return 1.0 - SamplePatternAtlas(p, u / p.repeat.x, v, hasHeight) * opacity;
}

#endif
