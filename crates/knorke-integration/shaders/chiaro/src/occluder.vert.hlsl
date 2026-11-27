// Origin: chiaro @ 9bc5b9b, shaders/src/occluder.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Occlusion mask: world-space obstacle triangles (static or dynamic) rasterized
// into the R8 mask. Draw with 3 vertices per triangle and no vertex buffers;
// the vertices come from a storage buffer of float2 world positions, followed
// in the same buffer by the opacity of each triangle's obstacle (E25).
// The mask covers the scene plus a margin band (see src/shadows/OcclusionDomain.hpp).
// CHI_HEIGHT (occluder_height.vert, E22): also passes on the height intervals of
// each triangle's obstacle. Intervals holds one (-base, top, finite -base, finite
// top) per triangle, in world units; infinitely tall obstacles have
// (0, 65504, -65504, -65504).

StructuredBuffer<float2> Vertices : register(t0, space0);
#ifdef CHI_HEIGHT
StructuredBuffer<float4> Intervals : register(t1, space0);
#endif

cbuffer DomainParams : register(b0, space1)
{
    float4 OriginXAxis;          // xy projection origin, zw x axis (scene pixels)
    float4 YAxisMargin;          // xy y axis, zw margin in scene pixels
    float4 TexelScaleDomainSize; // xy mask texels per scene pixel, zw mask size in texels
    float2 Remainder;            // remainder of the mask anchor in texels (OcclusionDomain.hpp)
    uint OpacityStart;           // index in Vertices where the opacities begin, two per entry
    float Pad;
};

// Opacity of the obstacle of triangle `index` (E25), negated for a footprint obstacle (shadow_footprint).
float TriangleOpacity(uint index)
{
    float2 pair = Vertices[OpacityStart + index / 2];
    return (index & 1u) != 0u ? pair.y : pair.x;
}

struct VSOutput
{
#ifdef CHI_HEIGHT
    nointerpolation float4 interval : TEXCOORD0;
    nointerpolation float2 opacity : TEXCOORD1; // x opacity, y 1 for a footprint obstacle
#else
    nointerpolation float2 opacity : TEXCOORD0;
#endif
    float4 position : SV_Position;
};

VSOutput main(uint vertexId : SV_VertexID)
{
    float2 world = Vertices[vertexId];
    float2 scene = OriginXAxis.xy + world.x * OriginXAxis.zw + world.y * YAxisMargin.xy;
    float2 texel = (scene + YAxisMargin.zw) * TexelScaleDomainSize.xy - Remainder;
    VSOutput o;
#ifdef CHI_HEIGHT
    o.interval = Intervals[vertexId / 3];
#endif
    float opacity = TriangleOpacity(vertexId / 3); // negative: a footprint obstacle (shadow_footprint)
    o.opacity = float2(abs(opacity), opacity < 0.0 ? 1.0 : 0.0);
    o.position = float4(texel / TexelScaleDomainSize.zw * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
