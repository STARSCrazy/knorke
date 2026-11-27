// Origin: chiaro @ 9bc5b9b, shaders/src/medium_world_top.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium world map, highest top and extent (Task 7, E100, E108): two points per map cell with medium into a 2 x 1 target
// blended MAX (empty cells are clipped, E105). Pixel 0 ends up holding (highest top, max x + 1, max y + 1,
// map width - min x) of the cells with medium this frame, pixel 1 (map height - min y, 0, 0, 0); all > 0 when there
// is medium, 0 (cleared) without. The resolve (medium_world_resolve.frag) writes the top into A of every map texel and
// the cell rectangle into A of texels 1 to 4 of row 0; directional lights march toward the sun only up to the top
// (medium_transmit.hlsli), not up to the highest top the layer can encode (255 steps), and a light path that misses the
// rectangle skips the map (E108).
// t0 slab union of the splat (top, -bottom), nearest; b0 TopParams. A cell without medium holds (-1e30, -1e30).
#include "common.hlsli"

Texture2D<float4> Union : register(t0, space0);
SamplerState UnionSampler : register(s0, space0); // nearest

// Matches struct MediumTopParams in src/rendering/MediumMapStage.cpp.
cbuffer TopParams : register(b0, space1)
{
    float4 TopMap; // xy map size in texels
};

struct VSOutput
{
    float4 value : TEXCOORD0;
    float4 position : SV_Position;
    [[vk::builtin("PointSize")]] float size : PSIZE; // Vulkan needs it for point lists
};

VSOutput main(uint id : SV_VertexID)
{
    uint width = uint(TopMap.x);
    uint pixel = id & 1u;
    int2 cell = int2((id >> 1) % width, (id >> 1) / width);
    VSOutput o;
    float top = max(FetchTexel(Union, UnionSampler, cell, 1.0 / TopMap.xy).r, 0.0);
    o.value = pixel == 0u ? float4(top, float(cell.x) + 1.0, float(cell.y) + 1.0, TopMap.x - float(cell.x))
                          : float4(TopMap.y - float(cell.y), 0.0, 0.0, 0.0);
    // The centre of pixel 0 or 1 of the 2 x 1 target. A cell without medium cannot raise the cleared 0, so its points
    // land outside the target and are clipped: every point that reaches the target blends into one of two pixels.
    o.position = top > 0.0 ? float4(pixel == 0u ? -0.5 : 0.5, 0.0, 0.0, 1.0) : float4(2.0, 2.0, 0.0, 1.0);
    o.size = 1.0;
    return o;
}
