// Origin: chiaro @ 9bc5b9b, shaders/src/tile.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Occlusion mask: one tile grid as a single transformed quad. Draw with 6
// vertices and no vertex buffers; tile.frag discards the empty cells.
// Cell (0, 0) has its corner at the local origin; the grid is rotated about
// and moved to Placement.xy, like AppendWorldTriangles in src/shadows/ObstacleStore.cpp.

cbuffer DomainParams : register(b0, space1)
{
    float4 OriginXAxis;          // xy projection origin, zw x axis (scene pixels)
    float4 YAxisMargin;          // xy y axis, zw margin in scene pixels
    float4 TexelScaleDomainSize; // xy mask texels per scene pixel, zw mask size in texels
    float4 RemainderPad;         // xy remainder of the mask anchor in texels (OcclusionDomain.hpp), zw unused
    float4 Placement;            // xy world position, z rotation in radians (counter-clockwise), w unused
    float4 GridSize;             // xy cell size in world units, zw columns, rows
};

struct VSOutput
{
    float2 uv : TEXCOORD0; // 0..1 across the grid, (0, 0) at cell (0, 0)
    float4 position : SV_Position;
};

static const float2 kCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

VSOutput main(uint vertexId : SV_VertexID)
{
    float2 corner = kCorners[vertexId];
    float2 local = corner * GridSize.xy * GridSize.zw;
    float c = cos(Placement.z);
    float s = sin(Placement.z);
    float2 world = float2(c * local.x - s * local.y, s * local.x + c * local.y) + Placement.xy;
    float2 scene = OriginXAxis.xy + world.x * OriginXAxis.zw + world.y * YAxisMargin.xy;
    float2 texel = (scene + YAxisMargin.zw) * TexelScaleDomainSize.xy - RemainderPad.xy;

    VSOutput o;
    o.uv = corner;
    o.position = float4(texel / TexelScaleDomainSize.zw * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
