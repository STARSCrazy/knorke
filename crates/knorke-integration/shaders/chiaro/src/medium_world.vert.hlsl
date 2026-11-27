// Origin: chiaro @ 9bc5b9b, shaders/src/medium_world.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium world map (spec 2026-11-13-medien-volumetrik, "Abschirmen: Medium-Weltkarte"), splat. The map shares the
// occlusion mask's grid (E20) at a quarter of its resolution; that grid lies in scene space at ground level (z = 0).
// A layer pixel at scene position q belongs to the ground point of its slab's middle h = (bottom + top) / 2, whose
// scene position is q - h * z_axis.
//
// The layer is cut into blocks on the half-cell grid of the map (2 x 2 blocks per cell, in scene space), one point
// each. A block averages its footprint: n x n taps evenly spread over it, n = the block's width in layer pixels
// (rounded up), at most kMaxBlockTaps = 8 (MediumMapStage.cpp; below shadow_resolution 0.25 the taps thin out to 64 per
// block). Its sigma is the mean of the taps (empty ones count 0), its slab the union of theirs (lowest bottom, highest
// top), its height h the sigma-weighted mean of their middles. The point lands at the block's centre moved by
// -h * z_axis. Every block shifted by the same h
// is again a half-cell lattice, so exactly 4 points land per cell wherever the medium lies at one height: the
// resolve divides by 4 and gets the medium's coverage of the cell.
// Drawn as a point list into two RG32F targets at map size (medium_world.frag): the sums (sigma, 1), added, and the
// union of the slabs (top, -bottom), blended MAX; medium_world_resolve.frag turns them into the map. A block without
// medium is placed outside the target (clipped).
// t0 medium layer (nearest), b0 MediumParams (medium_common.hlsli), b1 MediumSplatParams.
#define CHI_MEDIUM_PARAMS_SPACE space1
#include "common.hlsli"
#include "surface.hlsli"
#include "medium_common.hlsli"

Texture2D<float4> MediumTex : register(t0, space0);
SamplerState MediumSampler : register(s0, space0); // nearest

// Matches struct MediumSplatParams in src/rendering/MediumMapStage.cpp.
cbuffer MediumSplatParams : register(b1, space1)
{
    float4 SceneFromMap; // scene position (z = 0) of map position m: m * xy + zw
    float4 MapFromScene; // xy: map units per scene pixel (the inverse scale)
    float4 MapBlocks;    // xy map size in texels, z blocks per row (2 per map texel), w taps per block side
};

struct VSOutput
{
    float4 value : TEXCOORD0; // (sigma, bottom, top, 0)
    float4 position : SV_Position;
    [[vk::builtin("PointSize")]] float size : PSIZE; // Vulkan needs it for point lists; D3D12 draws 1 pixel anyway
};

// The layer's slab at scene position `scene`; none outside the scene.
MediumSlab SlabAtScene(float2 scene)
{
    int2 texel = int2(floor(scene));
    MediumSlab m;
    m.density = 0.0;
    m.bottom = 0.0;
    m.top = 0.0;
    m.kind = 0u;
    if (any(texel < 0) || any(texel >= int2(MediumSizes.xy))) {
        return m;
    }
    return MediumLayerAt(MediumTex, MediumSampler, texel);
}

VSOutput main(uint id : SV_VertexID)
{
    uint blocksPerRow = uint(MapBlocks.z);
    float2 block = float2(id % blocksPerRow, id / blocksPerRow); // in half map texels
    // The block's footprint: n x n taps evenly spread over it (one per layer pixel up to the cap, MapBlocks.w).
    uint n = uint(MapBlocks.w);
    float sum = 0.0;    // sigma over all taps (empty ones count 0)
    float midSum = 0.0; // sigma-weighted middle heights
    float bottom = 3.4e38;
    float top = -3.4e38;
    [loop] for (uint j = 0u; j < n; ++j) {
        [loop] for (uint i = 0u; i < n; ++i) {
            float2 map = (block + (float2(i, j) + 0.5) / float(n)) * 0.5;
            MediumSlab m = SlabAtScene(map * SceneFromMap.xy + SceneFromMap.zw);
            float sigma = m.top > m.bottom ? m.density * MediumTable[m.kind].a : 0.0;
            if (sigma > 0.0) {
                sum += sigma;
                midSum += sigma * 0.5 * (m.bottom + m.top);
                bottom = min(bottom, m.bottom);
                top = max(top, m.top);
            }
        }
    }
    VSOutput o;
    o.size = 1.0;
    if (sum <= 0.0) {
        o.value = float4(0.0, 0.0, 0.0, 0.0);
        o.position = float4(-2.0, -2.0, 0.0, 1.0); // outside the map
        return o;
    }
    o.value = float4(sum / float(n * n), bottom, top, 0.0);
    float2 centre = (block + 0.5) * 0.5;
    float2 map = centre - (midSum / sum) * MediumOriginZAxis.zw * MapFromScene.xy;
    o.position = float4(map / MapBlocks.xy * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
