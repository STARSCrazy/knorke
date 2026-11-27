// Origin: chiaro @ 9bc5b9b, shaders/src/tile_pattern_id.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Pattern id pass for tile grids (Muster-Hindernisse, pattern.hlsli): tile.vert draws the grid's quad, and the cells
// tile.frag covers (the same cell texture and edge strips, tile_strip.hlsli) get the grid's id, 1 + index of its
// GpuPatternObstacle record, as id / 65535 in the id texture (kPatternIdFormat, R16_UNORM, cleared to 0, not
// blended: the last draw wins in the fixed draw order), but only where the cell's pattern (PatternOfCell) is not
// none: other cells keep the scalar opacity and narrow the penumbra as before.
#include "common.hlsli"
#include "pattern.hlsli"
#include "tile_strip.hlsli"

Texture2D<float4> Cells : register(t0, space2);
SamplerState CellSampler : register(s0, space2); // nearest
StructuredBuffer<GpuPatternObstacle> PatternObstacles : register(t1, space2);
StructuredBuffer<uint> PatternData : register(t2, space2);

// Matches struct GpuTilePatternIdParams in src/rendering/GpuPatternObstacle.hpp.
cbuffer TilePatternIdParams : register(b0, space3)
{
    float4 Strip;  // tile_strip.hlsli, as in tile.frag's TileParams
    uint4 Record;  // x the id: 1 + index of the grid's record in PatternObstacles; yzw unused
};

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float4 texel = Cells.SampleLevel(CellSampler, uv, 0.0);
    bool outside = OutsideStrips(uv, texel.a, Strip);
    float2 cell = round(texel.rg * 255.0);
    if (cell.x < 0.5 || texel.b < kMinObstacleOpacity || outside) {
        discard;
    }
    GpuPatternObstacle grid = PatternObstacles[Record.x - 1u];
    uint2 c = min(uint2(max(uv, 0.0) * float2(grid.data.xy)), grid.data.xy - 1u); // the texel the sampler read
    if (PatternOfCell(PatternData, grid, c.y * grid.data.x + c.x) == 0u) {
        discard;
    }
    return float4(float(Record.x) / 65535.0, 0.0, 0.0, 0.0);
}
