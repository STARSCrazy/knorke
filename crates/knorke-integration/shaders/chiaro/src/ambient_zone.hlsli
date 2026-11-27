// Origin: chiaro @ 9bc5b9b, shaders/src/ambient_zone.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Ambient zones (Lichtquellen 6, spec 2026-11-14-lokales-ambient): the weight of one zone at a world point. Mirror of
// src/lighting/AmbientZone.cpp (ZoneSignedDistance, ZoneEdgeWeight, ZoneHeightContains), which CHI_SampleAmbient
// uses; keep both in step.
#ifndef CHI_AMBIENT_ZONE_HLSLI
#define CHI_AMBIENT_ZONE_HLSLI

// Matches struct GpuZone in src/rendering/GpuZone.hpp (80 bytes).
struct GpuZone
{
    float4 sceneRect; // x0, y0, x1, y1 in scene pixels
    float4 color;     // rgb the zone's ambient (relative resolved) plus the flat directional sum, a kind (CHI_ShapeType)
    float4 shape;     // centre x, y, cos and sin of the rotation
    float4 span;      // soft_edge, base_height, top (FLT_MAX: none), 1 = check the height
    float4 geom;      // circle: x radius; box: xy half extents; polygon: x row of the vertex texture, y vertex count
};

// Varyings of ambient_zone.vert, read by ambient_zone.frag (same order in both, D3D12 wants a prefix, E31).
#define CHI_AMBIENT_ZONE_VARYINGS                 \
    float2 scene : TEXCOORD0;                     \
    nointerpolation float4 color : TEXCOORD1;     \
    nointerpolation float4 shape : TEXCOORD2;     \
    nointerpolation float4 span : TEXCOORD3;      \
    nointerpolation float4 geom : TEXCOORD4;

static const float kZoneCircle = 1.0; // CHI_SHAPE_CIRCLE
static const float kZoneBox = 2.0;    // CHI_SHAPE_BOX

// Signed distance to the outline in world units, positive inside. Polygons read their world vertices (x, y per texel)
// from row geom.x of `vertices`, `invSize` = (1 / width, 1 / rows).
float ZoneSignedDistance(float kind, float4 shape, float4 geom, float2 p, Texture2D<float4> vertices,
                         SamplerState vertexSampler, float2 invSize)
{
    if (kind == kZoneCircle) {
        return geom.x - length(p - shape.xy);
    }
    if (kind == kZoneBox) {
        float2 d = p - shape.xy;
        float2 q = float2(shape.z * d.x + shape.w * d.y, -shape.w * d.x + shape.z * d.y); // R(-rotation) (p - c)
        float2 a = abs(q) - geom.xy;
        return -(length(max(a, 0.0)) + min(max(a.x, a.y), 0.0));
    }
    // Polygon: the nearest edge, the sign by the crossing rule (same loop as ZoneSignedDistance on the CPU).
    uint n = uint(geom.y);
    float v = (geom.x + 0.5) * invSize.y;
    float best = 3.0e38;
    bool inside = false;
    float2 a = vertices.SampleLevel(vertexSampler, float2((float(n - 1u) + 0.5) * invSize.x, v), 0.0).xy;
    for (uint i = 0u; i < n; ++i) {
        float2 b = vertices.SampleLevel(vertexSampler, float2((float(i) + 0.5) * invSize.x, v), 0.0).xy;
        float2 e = b - a;
        float2 w = p - a;
        float ee = dot(e, e);
        float t = ee > 0.0 ? clamp(dot(w, e) / ee, 0.0, 1.0) : 0.0;
        float2 c = w - e * t;
        best = min(best, dot(c, c));
        if ((a.y > p.y) != (b.y > p.y) && p.x < e.x * (p.y - a.y) / e.y + a.x) {
            inside = !inside;
        }
        a = b;
    }
    float dist = sqrt(best);
    return inside ? dist : -dist;
}

// Weight in the soft edge: 0 outside, smoothstep over soft_edge inside, 1 without a soft edge.
float ZoneEdgeWeight(float d, float softEdge)
{
    if (d < 0.0) {
        return 0.0;
    }
    if (softEdge <= 0.0) {
        return 1.0;
    }
    float t = clamp(d / softEdge, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// w of the zone at `world` (xy ground, z height): edge times the half-open height span [base, top).
float ZoneWeight(float4 color, float4 shape, float4 span, float4 geom, float3 world, Texture2D<float4> vertices,
                 SamplerState vertexSampler, float2 invSize)
{
    if (span.w != 0.0 && (world.z < span.y || world.z >= span.z)) {
        return 0.0;
    }
    float d = ZoneSignedDistance(color.a, shape, geom, world.xy, vertices, vertexSampler, invSize);
    return ZoneEdgeWeight(d, span.x);
}

#endif
