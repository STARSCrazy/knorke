// Origin: chiaro @ 9bc5b9b, shaders/src/pattern.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Pattern obstacles (Muster-Hindernisse, docs/superpowers/specs/2026-11-14-muster-hindernisse-design.md): an obstacle
// with a pass-through pattern standing upright on its faces. Only the `*_pattern` programs (CHI_PATTERN) and the id
// passes include this file; every program without the define is not changed by patterns at all.
//
// A shadow ray that enters a pattern obstacle (a texel of the id texture is not 0) intersects the piece from its
// last sample outside to the hit sample with the obstacle's geometry in world space (PatternCrossingOpacity) and
// multiplies its transmittance by 1 - alpha * opacity at the entry point, once per crossing. The mask holds
// mean * opacity clamped to [1/255, 254/255] for these obstacles (ObstacleStore::MaskOpacity), so the full block at
// kOpaqueOpacity never ends a ray before the pattern is read. Pattern obstacles do not narrow the penumbra (spec,
// Nicht-Ziele): their shadow stays hard.
//
// Part 1 (always): the records (src/rendering/GpuPatternObstacle.hpp) and the entry geometry, the same steps as
// src/shadows/PatternGeometry.cpp (EntryBox, EntryPolygon, EntryCircle, EntryTileCell): the slab method, the same tie
// rules (the first axis, edge or strip wins an exact tie), the sign normalisation with kAxisTolerance and the
// nearest-point fallback. Keep both in step. The HLSL entry also returns t and, for a fallback, the squared distance,
// which the tile grid uses to pick the entered cell among the cells around the hit.
//
// Part 2 (a light program that defines CHI_PATTERN_SLOT): the bindings and the crossing. The program defines, before
// it includes this file (directly or through shadow_trace.hlsli):
//   CHI_PATTERN_SLOT         sampler slot of the id texture (after all its other samplers, the medium map included),
//                            space2; the atlas follows at CHI_PATTERN_SLOT + 1, then the storage buffers
//                            PatternObstacles, Patterns and PatternData at t(CHI_PATTERN_SLOT + 2 ...), space2
//   CHI_PATTERN_PARAMS_SLOT  uniform slot of PatternParams (after its other uniform blocks), space3
//
//   t/s  CHI_PATTERN_SLOT       PatternIds   R16_UNORM, nearest: 1 + index into PatternObstacles, / 65535; 0 none
//   t/s  CHI_PATTERN_SLOT + 1   PatternAtlas R8, nearest: the patterns' alpha and their column means
//   t    CHI_PATTERN_SLOT + 2   PatternObstacles StructuredBuffer<GpuPatternObstacle>
//   t    CHI_PATTERN_SLOT + 3   Patterns         StructuredBuffer<GpuPattern>
//   t    CHI_PATTERN_SLOT + 4   PatternData      StructuredBuffer<uint>: polygon corners, tile cell words, cell
//                                                indices and pattern tables
//   b    CHI_PATTERN_PARAMS_SLOT PatternParams (GpuPatternParams)
#ifndef CHI_PATTERN_HLSLI
#define CHI_PATTERN_HLSLI

#include "common.hlsli"

// ---- Part 1: records and entry geometry ------------------------------------------------------------------------

// GpuPatternObstacle.kind.x (PatternKind in src/rendering/GpuPatternObstacle.hpp).
static const uint kPatternBox = 0u;
static const uint kPatternPolygon = 1u;
static const uint kPatternCircle = 2u;
static const uint kPatternTileGrid = 3u;
// GpuPatternObstacle.kind.y bits.
static const uint kPatternFiniteHeight = 1u; // heights.xy is a finite span (else no ray height: column mean)
// GpuPatternObstacle.data.w of a tile grid without cell indices (every cell takes the grid's pattern).
static const uint kPatternNoIndices = 0xFFFFFFFFu;

// Matches struct GpuPattern in src/rendering/GpuPatternObstacle.hpp: one live pattern in the atlas.
struct GpuPattern
{
    uint4 rect;    // xy atlas texel of the region's top-left texel (row 0 is the pattern's top row), zw width, height
    uint4 mean;    // xy atlas texel of column 0's mean over all rows; column x's mean lies x texels further along +x
    float4 repeat; // x repeat_width (> 0), y repeat_height (0: stretched over the height span), zw unused
};

// Matches struct GpuPatternObstacle in src/rendering/GpuPatternObstacle.hpp: one pattern obstacle of the frame.
struct GpuPatternObstacle
{
    uint4 kind;       // x kPattern* kind, y kPatternFiniteHeight, z pattern: 1 + index into Patterns, 0 none (a tile
                      // grid: the pattern of the cells whose index is 0), w unused
    float4 transform; // xy world origin (box and circle centre, polygon local origin, tile grid corner of cell
                      // (0, 0)), zw cos and sin of the rotation (counter-clockwise): local = R^T (world - origin)
    float4 shape;     // box: xy half extents (local x, y); circle: x radius; tile grid: xy cell size, z edge_width
    float4 heights;   // x base, y top (world units, only with kPatternFiniteHeight); z tile grid: level_height (0:
                      // every cell spans x .. y); w the obstacle's opacity (a tile grid: times each cell's byte)
    uint4 data;       // polygon: x first word of the corners in PatternData (2 words per corner, asfloat x, y), y
                      // corner count (<= 64); tile grid: x columns, y rows, z first cell word (one per cell,
                      // row-major: value | bottom << 8 | raw cell_opacity << 16 | edges << 24, see
                      // GpuPatternObstacle.hpp), w first word of the cell indices (4 per word, cell i in byte i % 4)
                      // or kPatternNoIndices
    uint4 table;      // tile grid: x first word of the pattern table (1 + index into Patterns, 0 none), y entries;
                      // zw unused
};

static const float kPatternTiny = 1e-12;
static const float kPatternTwoPi = 6.28318530717958647692;
// Sign normalisation of an edge direction (kAxisTolerance in src/shadows/PatternGeometry.cpp).
static const float kPatternAxisTolerance = 1e-6;
// The largest float below 1: v of a stretched pattern at the top edge still reads the top row.
static const float kPatternBelowOne = 0.99999994;

// EntryHit::side of a box or tile cell (kSide* in src/shadows/PatternGeometry.hpp).
static const int kPatternSideXLow = 0;
static const int kPatternSideXHigh = 1;
static const int kPatternSideYLow = 2;
static const int kPatternSideYHigh = 3;

// EntryHit of src/shadows/PatternGeometry.hpp plus t and dist2 (HLSL only).
struct PatternEntry
{
    bool found;
    float u;
    float worldZ;
    int side;
    float t;     // found: the entry's share of the piece a -> b; otherwise 1
    float dist2; // not found: the squared distance from b to the outline point that gave u; otherwise 0
};

PatternEntry PatternHit(float t, float u, int side, float za, float zb)
{
    PatternEntry e;
    e.found = true;
    e.u = u;
    e.worldZ = za + t * (zb - za);
    e.side = side;
    e.t = t;
    e.dist2 = 0.0;
    return e;
}

PatternEntry PatternFallback(float u, int side, float zb, float dist2)
{
    PatternEntry e;
    e.found = false;
    e.u = u;
    e.worldZ = zb;
    e.side = side;
    e.t = 1.0;
    e.dist2 = dist2;
    return e;
}

// RectEntry: entry of a + t (b - a), t in [0, 1], into [lo, hi] (slab method), only if a lies outside. On an exact
// corner the x face wins.
bool PatternRectEntry(float2 lo, float2 hi, float2 a, float2 b, out float tOut, out int sideOut)
{
    tOut = 0.0;
    sideOut = -1;
    float2 d = b - a;
    float tEnter = -1.0; // below 0: no entry (yet)
    float tExit = 2.0;
    int side = -1;
    [unroll] for (int k = 0; k < 2; ++k) {
        if (abs(d[k]) < kPatternTiny) {
            if (a[k] < lo[k] || a[k] > hi[k]) {
                return false; // parallel to this slab and outside it
            }
            continue;
        }
        float t0 = (lo[k] - a[k]) / d[k];
        float t1 = (hi[k] - a[k]) / d[k];
        int face = 2 * k; // low face
        if (d[k] < 0.0) {
            float swap = t0;
            t0 = t1;
            t1 = swap;
            face = 2 * k + 1; // moving down: enters through the high face
        }
        if (t0 > tEnter) {
            tEnter = t0;
            side = face;
        }
        tExit = t1 < tExit ? t1 : tExit;
    }
    if (side < 0 || tEnter < 0.0 || tEnter > 1.0 || tEnter > tExit) {
        return false;
    }
    tOut = tEnter;
    sideOut = side;
    return true;
}

bool PatternInside(float2 lo, float2 hi, float2 p)
{
    return p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
}

// RectNearest: the nearest point of the rectangle's outline to b, its face and the squared distance.
float PatternRectNearest(float2 lo, float2 hi, float2 b, out float2 nearest, out int side)
{
    if (PatternInside(lo, hi, b)) {
        float4 dist = float4(b.x - lo.x, hi.x - b.x, b.y - lo.y, hi.y - b.y);
        side = 0;
        [unroll] for (int k = 1; k < 4; ++k) {
            side = dist[k] < dist[side] ? k : side;
        }
        nearest = b;
        if (side == kPatternSideXLow) {
            nearest.x = lo.x;
        } else if (side == kPatternSideXHigh) {
            nearest.x = hi.x;
        } else if (side == kPatternSideYLow) {
            nearest.y = lo.y;
        } else {
            nearest.y = hi.y;
        }
        return dist[side] * dist[side];
    }
    nearest = float2(b.x < lo.x ? lo.x : (b.x > hi.x ? hi.x : b.x), b.y < lo.y ? lo.y : (b.y > hi.y ? hi.y : b.y));
    float ex = abs(b.x - nearest.x);
    float ey = abs(b.y - nearest.y);
    if (ex >= ey) {
        side = b.x < lo.x ? kPatternSideXLow : kPatternSideXHigh;
    } else {
        side = b.y < lo.y ? kPatternSideYLow : kPatternSideYHigh;
    }
    return ex * ex + ey * ey;
}

bool PatternIsXSide(int side)
{
    return side == kPatternSideXLow || side == kPatternSideXHigh;
}

// EdgeU: the point projected on the edge direction, sign-normalised so parallel opposite edges agree.
float PatternEdgeU(float2 p, float2 e)
{
    float len = sqrt(e.x * e.x + e.y * e.y);
    if (len < kPatternTiny) {
        return 0.0;
    }
    float2 dir = float2(e.x / len, e.y / len);
    bool vertical = abs(dir.x) < kPatternAxisTolerance; // dir is unit: the share of the length
    if (vertical ? dir.y < 0.0 : dir.x < 0.0) {
        dir = -dir;
    }
    return p.x * dir.x + p.y * dir.y;
}

// ArcU: arc length from local +x, counter-clockwise, in [0, 2 pi radius).
float PatternArcU(float2 p, float radius)
{
    float angle = atan2(p.y, p.x);
    if (angle < 0.0) {
        angle += kPatternTwoPi;
    }
    return angle * radius;
}

float PatternCross(float2 p, float2 q)
{
    return p.x * q.y - p.y * q.x;
}

// EntryBox: a box of half extents halfW (local x) and halfD (local y) about the local origin.
PatternEntry PatternEntryBox(float halfW, float halfD, float2 a, float2 b, float za, float zb)
{
    float2 lo = float2(-halfW, -halfD);
    float2 hi = float2(halfW, halfD);
    float t;
    int side;
    if (PatternRectEntry(lo, hi, a, b, t, side)) {
        float2 p = a + t * (b - a);
        return PatternHit(t, PatternIsXSide(side) ? p.y + halfD : p.x + halfW, side, za, zb);
    }
    float2 p;
    float dist2 = PatternRectNearest(lo, hi, b, p, side);
    return PatternFallback(PatternIsXSide(side) ? p.y + halfD : p.x + halfW, side, zb, dist2);
}

// EntryCircle: a circle of `radius` about the local origin.
PatternEntry PatternEntryCircle(float radius, float2 a, float2 b, float za, float zb)
{
    float2 d = b - a;
    float qa = d.x * d.x + d.y * d.y;
    float qb = a.x * d.x + a.y * d.y;
    float qc = a.x * a.x + a.y * a.y - radius * radius; // >= 0: a outside (or on) the circle
    if (qa >= kPatternTiny && qc >= 0.0) {
        float disc = qb * qb - qa * qc;
        if (disc >= 0.0) {
            float t = (-qb - sqrt(disc)) / qa;
            if (t >= 0.0 && t <= 1.0) {
                return PatternHit(t, PatternArcU(a + t * d, radius), 0, za, zb);
            }
        }
    }
    float len = sqrt(b.x * b.x + b.y * b.y);
    float2 p = len < kPatternTiny ? float2(radius, 0.0) : float2(b.x * radius / len, b.y * radius / len);
    float2 off = b - p;
    return PatternFallback(PatternArcU(p, radius), 0, zb, off.x * off.x + off.y * off.y);
}

// Piece k of the cell [lo, hi]: with whole the cell itself, else the edge strip along the edge of kPatternSide k,
// wx and wy wide.
void PatternCellPiece(bool whole, int k, float2 lo, float2 hi, float wx, float wy, out float2 pieceLo,
                      out float2 pieceHi)
{
    pieceLo = lo;
    pieceHi = hi;
    if (whole) {
        return;
    }
    if (k == kPatternSideXLow) {
        pieceHi.x = lo.x + wx;
    } else if (k == kPatternSideXHigh) {
        pieceLo.x = hi.x - wx;
    } else if (k == kPatternSideYLow) {
        pieceHi.y = lo.y + wy;
    } else {
        pieceLo.y = hi.y - wy;
    }
}

// EntryTileCell: cell (column, row) of a tile grid, a and b grid-local (cell (0, 0) has its corner at the origin).
// edgeBits 1..15 are strips edgeWidth wide along the named edges, 0 or above 15 (255) the whole cell. A whole cell
// reads u along the entered side, X strips u = y and Y strips u = x. One loop and one exit for both, and RectEntry
// (out parameters) called outside any && chain: the version with those calls inside && chains, local strip arrays
// and early returns read every strip as opaque on Vulkan (SPIR-V) while Direct3D 12 was right.
PatternEntry PatternEntryTileCell(float2 cellSize, uint column, uint row, uint edgeBits, float edgeWidth, float2 a,
                                  float2 b, float za, float zb)
{
    float2 lo = float2(float(column) * cellSize.x, float(row) * cellSize.y);
    float2 hi = float2(lo.x + cellSize.x, lo.y + cellSize.y);
    bool whole = edgeBits == 0u || edgeBits > 15u;
    uint pieces = whole ? 1u : edgeBits;
    float wx = clamp(edgeWidth, 0.0, cellSize.x);
    float wy = clamp(edgeWidth, 0.0, cellSize.y);

    // A ray that starts inside a strip finds no entry into any of them (whole: RectEntry checks a itself).
    bool aInside = false;
    [unroll] for (int k = 0; k < 4; ++k) {
        float2 pLo, pHi;
        PatternCellPiece(whole, k, lo, hi, wx, wy, pLo, pHi);
        bool piece = ((pieces >> uint(k)) & 1u) != 0u;
        aInside = aInside || (piece && !whole && PatternInside(pLo, pHi, a));
    }
    float bestT = 2.0;
    int bestPiece = -1;
    int bestSide = -1;
    float bestDist = -1.0;
    float bestU = 0.0;
    int nearSide = -1;
    [unroll] for (int k = 0; k < 4; ++k) {
        float2 pLo, pHi;
        PatternCellPiece(whole, k, lo, hi, wx, wy, pLo, pHi);
        bool piece = ((pieces >> uint(k)) & 1u) != 0u;
        float t = 2.0;
        int side = -1;
        bool hit = PatternRectEntry(pLo, pHi, a, b, t, side);
        bool entered = hit && piece && !aInside;
        if (entered && t < bestT) {
            bestT = t;
            bestPiece = k;
            bestSide = side;
        }
        float2 p = pLo;
        int pSide = -1;
        float dist = PatternRectNearest(pLo, pHi, b, p, pSide);
        if (piece && (bestDist < 0.0 || dist < bestDist)) {
            bestDist = dist;
            nearSide = pSide;
            bestU = PatternIsXSide(whole ? pSide : k) ? p.y : p.x;
        }
    }
    PatternEntry result = PatternFallback(bestU, nearSide, zb, max(bestDist, 0.0));
    if (bestPiece >= 0) {
        float2 p = a + bestT * (b - a);
        result = PatternHit(bestT, PatternIsXSide(whole ? bestSide : bestPiece) ? p.y : p.x, bestSide, za, zb);
    }
    return result;
}

// The pattern of tile cell `cell` (row-major index): its index byte (0 without indices) picks the grid's pattern (0)
// or table entry index - 1; an index beyond the table is none. 1 + index into Patterns, 0 none.
uint PatternOfCell(StructuredBuffer<uint> data, GpuPatternObstacle grid, uint cell)
{
    uint index = 0u;
    if (grid.data.w != kPatternNoIndices) {
        index = (data[grid.data.w + cell / 4u] >> (8u * (cell & 3u))) & 0xFFu;
    }
    if (index == 0u) {
        return grid.kind.z;
    }
    return index <= grid.table.y ? data[grid.table.x + index - 1u] : 0u;
}

// ---- Part 2: bindings and the crossing of the light programs ---------------------------------------------------
#ifdef CHI_PATTERN_SLOT

#include "light_common.hlsli"

#if CHI_PATTERN_SLOT == 3
#define CHI_PATTERN_ATLAS_SLOT 4
#define CHI_PATTERN_OBSTACLES_SLOT 5
#define CHI_PATTERN_LIST_SLOT 6
#define CHI_PATTERN_DATA_SLOT 7
#elif CHI_PATTERN_SLOT == 4
#define CHI_PATTERN_ATLAS_SLOT 5
#define CHI_PATTERN_OBSTACLES_SLOT 6
#define CHI_PATTERN_LIST_SLOT 7
#define CHI_PATTERN_DATA_SLOT 8
#elif CHI_PATTERN_SLOT == 5
#define CHI_PATTERN_ATLAS_SLOT 6
#define CHI_PATTERN_OBSTACLES_SLOT 7
#define CHI_PATTERN_LIST_SLOT 8
#define CHI_PATTERN_DATA_SLOT 9
#elif CHI_PATTERN_SLOT == 6
#define CHI_PATTERN_ATLAS_SLOT 7
#define CHI_PATTERN_OBSTACLES_SLOT 8
#define CHI_PATTERN_LIST_SLOT 9
#define CHI_PATTERN_DATA_SLOT 10
#elif CHI_PATTERN_SLOT == 7
#define CHI_PATTERN_ATLAS_SLOT 8
#define CHI_PATTERN_OBSTACLES_SLOT 9
#define CHI_PATTERN_LIST_SLOT 10
#define CHI_PATTERN_DATA_SLOT 11
#elif CHI_PATTERN_SLOT == 8
#define CHI_PATTERN_ATLAS_SLOT 9
#define CHI_PATTERN_OBSTACLES_SLOT 10
#define CHI_PATTERN_LIST_SLOT 11
#define CHI_PATTERN_DATA_SLOT 12
#else
#error "pattern.hlsli: CHI_PATTERN_SLOT must be 3 ... 8"
#endif

#define CHI_PATTERN_PASTE(a, b) a##b
#define CHI_PATTERN_REGISTER(kind, slot, space) register(CHI_PATTERN_PASTE(kind, slot), space)

Texture2D<float4> PatternIds : CHI_PATTERN_REGISTER(t, CHI_PATTERN_SLOT, space2);
SamplerState PatternIdSampler : CHI_PATTERN_REGISTER(s, CHI_PATTERN_SLOT, space2); // nearest
Texture2D<float4> PatternAtlas : CHI_PATTERN_REGISTER(t, CHI_PATTERN_ATLAS_SLOT, space2);
SamplerState PatternAtlasSampler : CHI_PATTERN_REGISTER(s, CHI_PATTERN_ATLAS_SLOT, space2); // nearest
StructuredBuffer<GpuPatternObstacle> PatternObstacles : CHI_PATTERN_REGISTER(t, CHI_PATTERN_OBSTACLES_SLOT, space2);
StructuredBuffer<GpuPattern> Patterns : CHI_PATTERN_REGISTER(t, CHI_PATTERN_LIST_SLOT, space2);
StructuredBuffer<uint> PatternData : CHI_PATTERN_REGISTER(t, CHI_PATTERN_DATA_SLOT, space2);

// Matches struct GpuPatternParams in src/rendering/GpuPatternObstacle.hpp. The mask lies in projected scene space
// (occluder.vert): mask texel = affine(world ground point); these rows are its inverse.
cbuffer PatternParams : CHI_PATTERN_REGISTER(b, CHI_PATTERN_PARAMS_SLOT, space3)
{
    float4 PatternMaskToWorldX; // xyz: world x = dot(xyz, (mask x, mask y, 1)) in mask texels, w unused
    float4 PatternMaskToWorldY; // xyz: world y, the same
    float4 PatternAtlasSize;    // xy 1 / atlas size in texels, zw atlas size
};

// The id under mask position p: the texel TexelOpacity (shadow_trace.hlsli) reads, clamped to the mask.
uint PatternIdAt(float2 p, float2 maskSize)
{
    int2 texel = clamp(int2(floor(p)), int2(0, 0), int2(maskSize) - 1);
    return uint(FetchTexel(PatternIds, PatternIdSampler, texel, 1.0 / maskSize).r * 65535.0 + 0.5);
}

// MaskOpacityAround (light_common.hlsli) and the id of the texel it picks: the highest opacity of the 2x2 texels
// around p, the first on a tie; texels outside the mask have opacity 0 and id 0.
float PatternMaskOpacityAround(Texture2D<float4> mask, SamplerState nearestSampler, float2 p, float2 maskSize,
                               out uint id)
{
    float2 corner = floor(p - 0.5) + 0.5;
    float best = -1.0;
    id = 0u;
    [unroll] for (int k = 0; k < 4; ++k) {
        float2 q = corner + float2(float(k & 1), float(k >> 1));
        float o = MaskOpacityAt(mask, nearestSampler, q, maskSize);
        if (o > best) {
            best = o;
            int2 texel = int2(floor(q));
            id = any(texel < 0) || any(texel >= int2(maskSize))
                     ? 0u
                     : uint(FetchTexel(PatternIds, PatternIdSampler, texel, 1.0 / maskSize).r * 65535.0 + 0.5);
        }
    }
    return best;
}

// std::isfinite as an explicit bit test (exponent not all ones): the compilers may fold isfinite() under relaxed
// floating-point rules, a bit test they cannot.
bool PatternIsFinite(float x)
{
    return (asuint(x) & 0x7F800000u) != 0x7F800000u;
}

// SamplePattern (src/shadows/PatternStore.cpp) on the atlas: u, v in repeats, the fraction of each (also for
// negative values) picks the texel nearest, v 0 is the bottom row and v -> 1 the top row; non-finite u or v reads as
// 0 and the texel index is clamped on both ends. Without height the column mean at u, whatever v is. [0, 1].
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
    int y = h - 1 - fromBottom; // row 0 is the top: v -> 1; in [0, h - 1]
    return FetchTexel(PatternAtlas, PatternAtlasSampler, int2(p.rect.xy) + int2(x, y), PatternAtlasSize.xy).r;
}

float2 PatternMaskToWorld(float2 mask)
{
    return float2(dot(PatternMaskToWorldX.xyz, float3(mask, 1.0)), dot(PatternMaskToWorldY.xyz, float3(mask, 1.0)));
}

float2 PatternToLocal(GpuPatternObstacle o, float2 world)
{
    float2 d = world - o.transform.xy;
    return float2(o.transform.z * d.x + o.transform.w * d.y, -o.transform.w * d.x + o.transform.z * d.y);
}

float2 PatternCorner(uint first, uint i)
{
    return asfloat(uint2(PatternData[first + 2u * i], PatternData[first + 2u * i + 1u]));
}

// EntryPolygon: a simple polygon of n corners (PatternCorner(first, i)) in either winding, n <= 64.
PatternEntry PatternEntryPolygon(uint first, uint n, float2 a, float2 b, float za, float zb)
{
    if (n < 2u) {
        return PatternFallback(0.0, -1, zb, 0.0);
    }
    float area2 = 0.0; // twice the signed area: > 0 counter-clockwise
    [loop] for (uint i = 0u; i < n; ++i) {
        area2 += PatternCross(PatternCorner(first, i), PatternCorner(first, i + 1u < n ? i + 1u : 0u));
    }
    float2 d = b - a;
    float bestT = 2.0;
    int bestEdge = -1;
    float2 bestE = float2(0.0, 0.0);
    [loop] for (uint j = 0u; j < n; ++j) {
        float2 v0 = PatternCorner(first, j);
        float2 e = PatternCorner(first, j + 1u < n ? j + 1u : 0u) - v0;
        float denom = PatternCross(d, e); // = dot(d, outward normal) for counter-clockwise winding
        if (abs(denom) < kPatternTiny || denom * area2 >= 0.0) {
            continue; // parallel, or leaving the polygon through this edge
        }
        float2 w = v0 - a;
        float t = PatternCross(w, e) / denom;
        float s = PatternCross(w, d) / denom;
        if (t >= 0.0 && t <= 1.0 && s >= 0.0 && s <= 1.0 && t < bestT) {
            bestT = t;
            bestEdge = int(j);
            bestE = e;
        }
    }
    if (bestEdge >= 0) {
        return PatternHit(bestT, PatternEdgeU(a + bestT * d, bestE), bestEdge, za, zb);
    }
    float bestDist = -1.0;
    float bestU = 0.0;
    [loop] for (uint k = 0u; k < n; ++k) {
        float2 v0 = PatternCorner(first, k);
        float2 e = PatternCorner(first, k + 1u < n ? k + 1u : 0u) - v0;
        float ee = e.x * e.x + e.y * e.y;
        float s = ee < kPatternTiny ? 0.0 : clamp(((b.x - v0.x) * e.x + (b.y - v0.y) * e.y) / ee, 0.0, 1.0);
        float2 p = v0 + s * e;
        float2 off = b - p;
        float dist = off.x * off.x + off.y * off.y;
        if (bestDist < 0.0 || dist < bestDist) {
            bestDist = dist;
            bestEdge = int(k);
            bestU = PatternEdgeU(p, e);
        }
    }
    return PatternFallback(bestU, bestEdge, zb, bestDist);
}

// Most cells the grid search reaches out from the cell under b on each side.
static const int kPatternGridReach = 4;

// The cell factor of a tile grid, as ObstacleStore::MaskOpacity's `factor`: the obstacle's opacity times the raw
// cell_opacity byte of the cell word (not the clamped mask value).
float PatternCellFactor(GpuPatternObstacle grid, uint word)
{
    return grid.heights.w * (float((word >> 16u) & 0xFFu) / 255.0);
}

// The entered cell of a tile grid: EntryTileCell for each solid cell within `reach` cells of the cell under b (the
// hit sample lies within about a mask texel of the entry, so reach covers a texel: ceil(texel / cell size), at
// least 1 and at most kPatternGridReach; with cells smaller than a quarter texel the entered cell can lie beyond
// and the nearest cell found stands in for it), the found entry with the smallest t, else the fallback nearest to
// b. False if no solid cell lies there.
bool PatternEntryGrid(GpuPatternObstacle grid, float2 a, float2 b, float za, float zb, int reach,
                      out PatternEntry best, out uint bestCell)
{
    best = PatternFallback(0.0, -1, zb, 0.0);
    bestCell = 0u;
    bool seen = false;
    int2 home = int2(floor(b / grid.shape.xy));
    [loop] for (int dy = -reach; dy <= reach; ++dy) {
        [loop] for (int dx = -reach; dx <= reach; ++dx) {
            int2 c = home + int2(dx, dy);
            if (any(c < 0) || c.x >= int(grid.data.x) || c.y >= int(grid.data.y)) {
                continue;
            }
            uint cell = uint(c.y) * grid.data.x + uint(c.x);
            uint word = PatternData[grid.data.z + cell];
            if ((word & 0xFFu) == 0u || float((word >> 16u) & 0xFFu) * grid.heights.w < 0.5) {
                continue; // empty, or a cell opacity that rounds to 0 in the cell texture (tile.frag discards both)
            }
            PatternEntry e =
                PatternEntryTileCell(grid.shape.xy, uint(c.x), uint(c.y), word >> 24u, grid.shape.z, a, b, za, zb);
            bool better = !seen || (e.found && (!best.found || e.t < best.t)) ||
                          (!e.found && !best.found && e.dist2 < best.dist2);
            if (better) {
                best = e;
                bestCell = cell;
                seen = true;
            }
        }
    }
    return seen;
}

// Opacity of a crossing that enters pattern obstacle `id` (1 + index into PatternObstacles) on the ray piece from the
// last sample outside, aMask, to the hit sample bMask (mask texels), at ray heights za and zb (world units);
// rayHeight false without ray heights (flat mode: the column mean). alpha * opacity at the entry point; maskOpacity
// (the mask's value there) where nothing patterns the crossing.
float PatternCrossingOpacity(uint id, float2 aMask, float2 bMask, float za, float zb, bool rayHeight,
                             float maskOpacity)
{
    GpuPatternObstacle o = PatternObstacles[id - 1u];
    // The piece reaches one mask texel past the hit sample: the mask widens thin obstacles to at least a texel
    // (WidenCircle, tile strips), so b can lie in the widened part before the true outline, which then follows
    // within that texel along the ray.
    float2 dMask = bMask - aMask;
    float lenMask = length(dMask);
    float reachOver = lenMask > kPatternTiny ? (lenMask + 1.0) / lenMask : 1.0;
    bMask = aMask + dMask * reachOver;
    zb = za + (zb - za) * reachOver;
    float2 a = PatternToLocal(o, PatternMaskToWorld(aMask));
    float2 b = PatternToLocal(o, PatternMaskToWorld(bMask));
    uint pattern = o.kind.z;
    float opacity = o.heights.w;
    float base = o.heights.x;
    float top = o.heights.y;
    bool finite = (o.kind.y & kPatternFiniteHeight) != 0u;
    PatternEntry e;
    if (o.kind.x == kPatternBox) {
        e = PatternEntryBox(o.shape.x, o.shape.y, a, b, za, zb);
    } else if (o.kind.x == kPatternPolygon) {
        e = PatternEntryPolygon(o.data.x, o.data.y, a, b, za, zb);
    } else if (o.kind.x == kPatternCircle) {
        e = PatternEntryCircle(o.shape.x, a, b, za, zb);
    } else {
        // World length of one mask texel along the longer mask axis.
        float texel = max(length(float2(PatternMaskToWorldX.x, PatternMaskToWorldY.x)),
                          length(float2(PatternMaskToWorldX.y, PatternMaskToWorldY.y)));
        int reach = clamp(int(ceil(texel / min(o.shape.x, o.shape.y))), 1, kPatternGridReach);
        uint cell;
        if (!PatternEntryGrid(o, a, b, za, zb, reach, e, cell)) {
            return maskOpacity;
        }
        uint word = PatternData[o.data.z + cell];
        pattern = PatternOfCell(PatternData, o, cell);
        opacity = PatternCellFactor(o, word);
        if (o.heights.z > 0.0) { // the cell spans bottom .. value levels
            base = float((word >> 8u) & 0xFFu) * o.heights.z;
            top = float(word & 0xFFu) * o.heights.z;
            finite = true;
        }
    }
    if (pattern == 0u) {
        return maskOpacity; // a cell without a pattern (or a destroyed one): the scalar opacity, as without patterns
    }
    GpuPattern p = Patterns[pattern - 1u];
    bool hasHeight = rayHeight && finite && top > base;
    float v = 0.0;
    if (hasHeight) {
        // A ray that enters through the top (or bottom) reads the pattern at the edge it passes.
        float z = clamp(e.worldZ, base, top) - base;
        v = p.repeat.y > 0.0 ? z / p.repeat.y : min(z / (top - base), kPatternBelowOne);
    }
    return SamplePatternAtlas(p, e.u / p.repeat.x, v, hasHeight) * opacity;
}

#endif // CHI_PATTERN_SLOT

#endif
