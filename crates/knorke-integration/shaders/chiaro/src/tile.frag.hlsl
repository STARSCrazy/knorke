// Origin: chiaro @ 9bc5b9b, shaders/src/tile.frag.hlsl, copied unchanged below this notice; only its variant
// tile_height.frag is compiled; its blobs in ../compiled are Chiaro's
// shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Occlusion mask: one tile grid. Cells holds one RGBA8 texel per cell, sampled nearest: the raw cell value in r
// (non-zero = solid), the bottom in g, the cell opacity times the obstacle's in b and the edge strip bits in a (E29).
// Empty cells, cells of opacity 0 and texels outside a cell's edge strips are discarded; the rest write the cell
// opacity (E25).
// CHI_HEIGHT (tile_height.frag): with Heights.x > 0 a solid cell spans bottom * Heights.x .. value * Heights.x,
// otherwise every solid cell has the obstacle's interval.
#include "common.hlsli"
#include "tile_strip.hlsli"
#ifdef CHI_HEIGHT
#include "occlusion_height.hlsli"
#endif

Texture2D<float4> Cells : register(t0, space2);
SamplerState CellSampler : register(s0, space2);

cbuffer TileParams : register(b0, space3)
{
    float4 Heights; // x cell height (0: use yz), y top, z -base, w 1 if the obstacle has a finite height
    float4 Strip;   // tile_strip.hlsli
};

#ifdef CHI_HEIGHT
MaskHeightOutput main(float2 uv : TEXCOORD0)
#else
float4 main(float2 uv : TEXCOORD0) : SV_Target0
#endif
{
    float4 texel = Cells.SampleLevel(CellSampler, uv, 0.0);
    bool outside = OutsideStrips(uv, texel.a, Strip);
    float2 cell = round(texel.rg * 255.0);
    if (cell.x < 0.5 || texel.b < kMinObstacleOpacity || outside) {
        discard;
    }
#ifdef CHI_HEIGHT
    float2 interval = Heights.x > 0.0 ? float2(-cell.y * Heights.x, cell.x * Heights.x) : Heights.zy;
    float2 finite = Heights.w > 0.5 ? interval : float2(-kNoHeight, -kNoHeight);
    return Occupied(float4(interval, finite), texel.b);
#else
    return float4(texel.b, 0.0, 0.0, 0.0); // tile grids are never footprints
#endif
}
