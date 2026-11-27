// Origin: chiaro @ 9bc5b9b, shaders/src/medium_world_resolve.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium world map, resolve: the splat targets of a cell (medium_world.frag) into the map's (sigma, bottom, top,
// highest top). A holds the highest top of the whole medium this frame in every texel (medium_world_top.vert), read by
// MediumMapTop() (medium_transmit.hlsli).
// sigma is the sum over the larger of 4 (the points a fully covered cell gets, medium_world.vert) and the points that
// landed, so a cell the medium only partly covers gets its share (cloud edges keep their place at map resolution).
// The slab is the union of the points' slabs: lowest bottom to highest top. Clouds at different heights over one cell
// become one slab spanning both (a cell holds one slab; the gap between them then shields too). A cell without points
// holds no medium.
// A of texels 1 to 4 of row 0 holds the rectangle of cells with medium instead (min x, min y, max x, max y; empty, min
// above max, without medium), when the map is at least 5 texels wide (Resolve.w = 1, E108).
// t0 sums (sigma, count), t1 union (top, -bottom), t2 highest top and extent (2 x 1), all nearest; b0 ResolveParams.
#include "common.hlsli"

Texture2D<float4> Sums : register(t0, space2);
Texture2D<float4> Union : register(t1, space2);
SamplerState SumsSampler : register(s0, space2);  // nearest
Texture2D<float4> Top : register(t2, space2);
SamplerState UnionSampler : register(s1, space2); // nearest
SamplerState TopSampler : register(s2, space2);   // nearest

// Matches struct MediumResolveParams in src/rendering/MediumMapStage.cpp.
cbuffer ResolveParams : register(b0, space3)
{
    float4 Resolve; // x points per fully covered cell (4), yz map size in texels, w 1 = rectangle in row 0
};

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    int2 texel = int2(floor(uv * Resolve.yz));
    float2 invSize = 1.0 / Resolve.yz;
    float4 top0 = FetchTexel(Top, TopSampler, int2(0, 0), float2(0.5, 1.0));
    float top = top0.r;
    if (Resolve.w > 0.5 && texel.y == 0 && texel.x >= 1 && texel.x <= 4) {
        // The rectangle of cells with medium; without medium (all 0) min x = width > max x = -1.
        float top1 = FetchTexel(Top, TopSampler, int2(1, 0), float2(0.5, 1.0)).r;
        float4 rect = float4(Resolve.y - top0.a, Resolve.z - top1, top0.g - 1.0, top0.b - 1.0);
        top = rect[texel.x - 1];
    }
    float2 sums = FetchTexel(Sums, SumsSampler, texel, invSize).rg;
    if (sums.y < 0.5) {
        return float4(0.0, 0.0, 0.0, top);
    }
    float2 slab = FetchTexel(Union, UnionSampler, texel, invSize).rg;
    return float4(sums.x / max(Resolve.x, sums.y), -slab.y, slab.x, top);
}
