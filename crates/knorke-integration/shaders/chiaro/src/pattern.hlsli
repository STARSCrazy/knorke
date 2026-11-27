// Origin: chiaro @ 9bc5b9b, shaders/src/pattern.hlsli, copied below this notice. Altered: only the records and
// PatternOfCell are kept, which tile_pattern_id.frag reads; the entry geometry and the crossing went with Chiaro's
// shadow programs. The blobs in ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Pattern obstacles (Muster-Hindernisse, docs/superpowers/specs/2026-11-14-muster-hindernisse-design.md): an obstacle
// with a pass-through pattern standing upright on its faces. The records (src/rendering/GpuPatternObstacle.hpp).
#ifndef CHI_PATTERN_HLSLI
#define CHI_PATTERN_HLSLI

#include "common.hlsli"

// GpuPatternObstacle.kind.x (PatternKind in src/rendering/GpuPatternObstacle.hpp).
static const uint kPatternBox = 0u;
static const uint kPatternPolygon = 1u;
static const uint kPatternCircle = 2u;
static const uint kPatternTileGrid = 3u;
// GpuPatternObstacle.data.w of a tile grid without cell indices (every cell takes the grid's pattern).
static const uint kPatternNoIndices = 0xFFFFFFFFu;
// Unused (EntryHit::side in Chiaro): without them DXC orders the SPIR-V constants of tile_pattern_id.frag
// differently, and the check of the frozen blobs compiles these sources byte for byte.
static const int kPatternSideXLow = 0;
static const int kPatternSideXHigh = 1;

// Matches struct GpuPatternObstacle in src/rendering/GpuPatternObstacle.hpp: one pattern obstacle of the frame.
struct GpuPatternObstacle
{
    uint4 kind;       // x kPattern* kind, y bit 0: heights.xy is a finite span, z pattern: 1 + index into Patterns,
                      // 0 none (a tile grid: the pattern of the cells whose index is 0), w unused
    float4 transform; // xy world origin (box and circle centre, polygon local origin, tile grid corner of cell
                      // (0, 0)), zw cos and sin of the rotation (counter-clockwise): local = R^T (world - origin)
    float4 shape;     // box: xy half extents (local x, y); circle: x radius; tile grid: xy cell size, z edge_width
    float4 heights;   // x base, y top (world units, only with kind.y bit 0); z tile grid: level_height (0:
                      // every cell spans x .. y); w the obstacle's opacity (a tile grid: times each cell's byte)
    uint4 data;       // polygon: x first word of the corners in PatternData (2 words per corner, asfloat x, y), y
                      // corner count (<= 64); tile grid: x columns, y rows, z first cell word (one per cell,
                      // row-major: value | bottom << 8 | raw cell_opacity << 16 | edges << 24, see
                      // GpuPatternObstacle.hpp), w first word of the cell indices (4 per word, cell i in byte i % 4)
                      // or kPatternNoIndices
    uint4 table;      // tile grid: x first word of the pattern table (1 + index into Patterns, 0 none), y entries;
                      // zw unused
};

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

#endif
