// Origin: chiaro @ 9bc5b9b, shaders/src/ambient_zone.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Ambient zone pass (Lichtquellen 6): one instance per visible zone, in fold order (priority, then creation number).
// The quad is the zone's scene rectangle (AmbientZoneCulling: the outline's world rectangle over its height span,
// projected, plus one scene pixel); the fragments compute each pixel's world point and the zone's weight, so only
// the scene position is interpolated. Rasterised at light buffer resolution like the light quads.

#include "ambient_zone.hlsli"

StructuredBuffer<GpuZone> Zones : register(t0, space0);

// Matches struct ZonePassParams in src/rendering/AmbientZonePass.cpp.
cbuffer ZonePassParams : register(b0, space1)
{
    float2 SceneSize;    // scene size in pixels
    uint InstanceOffset; // index of this draw's first zone in Zones
    uint Pad;
};

struct VSOutput
{
    CHI_AMBIENT_ZONE_VARYINGS
    float4 position : SV_Position;
};

static const float2 kCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

VSOutput main(uint vertexId : SV_VertexID, uint instanceId : SV_InstanceID)
{
    GpuZone zone = Zones[instanceId + InstanceOffset];
    float2 scene = lerp(zone.sceneRect.xy, zone.sceneRect.zw, kCorners[vertexId]);
    VSOutput o;
    o.scene = scene;
    o.color = zone.color;
    o.shape = zone.shape;
    o.span = zone.span;
    o.geom = zone.geom;
    o.position = float4(scene / SceneSize * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
