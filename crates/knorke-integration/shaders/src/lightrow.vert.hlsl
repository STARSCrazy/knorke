// Light row probe (ADR 0019, throwaway): one instance per lamp for every pass.
//   mode 0  row build: the lamp's row of the row atlas, coord.x the angle as a fraction of the turn
//   mode 2  light:     the scene rectangle the lamp can reach, coord the scene pixel
// The light rectangle is that of chiaro @ 9bc5b9b, shaders/src/light_surface.vert.hlsl (zlib license, see
// lightrow_light.frag.hlsl): the ground square and the same square raised by the highest height.
#include "lightrow.hlsli"

StructuredBuffer<Lamp> Lamps : register(t0, space0);

cbuffer LightRowPass : register(b0, space1)
{
    float4 OriginXAxis;    // xy projection origin, zw x axis (scene pixels)
    float4 YAxisSceneSize; // xy y axis, zw scene size in pixels
    float4 ZAxisMaxHeight; // xy z axis, z highest height a lit pixel can have
    float4 Atlas;          // x mode, y atlas height in texels, z rows per lamp (1)
};

struct VSOutput
{
    LIGHTROW_VARYINGS
    float4 position : SV_Position;
};

static const float2 kCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

VSOutput main(uint vertexId : SV_VertexID, uint instanceId : SV_InstanceID)
{
    Lamp lamp = Lamps[instanceId];
    float2 corner = kCorners[vertexId];
    VSOutput o;
    o.positionRadius = lamp.positionRadius;
    o.colorFalloff = lamp.colorFalloff;
    o.directionType = lamp.directionType;
    o.cone = lamp.cone;
    o.mask = lamp.mask;
    if (Atlas.x < 1.5) {
        float y = (lamp.mask.z + corner.y) * Atlas.z / Atlas.y;
        o.coord = corner;
        o.position = float4(corner.x * 2.0 - 1.0, 1.0 - 2.0 * y, 0.0, 1.0);
        return o;
    }
    float g = lamp.cone.z;
    float2 lo = float2(1e30, 1e30);
    float2 hi = float2(-1e30, -1e30);
    [unroll] for (int k = 0; k < 4; ++k) {
        float2 world = lamp.positionRadius.xy + float2((k & 1) ? g : -g, (k & 2) ? g : -g);
        float2 ground = OriginXAxis.xy + world.x * OriginXAxis.zw + world.y * YAxisSceneSize.xy;
        float2 raised = ground + ZAxisMaxHeight.z * ZAxisMaxHeight.xy;
        lo = min(lo, min(ground, raised));
        hi = max(hi, max(ground, raised));
    }
    float2 scene = lerp(lo, hi, corner);
    o.coord = scene;
    o.position = float4(scene / YAxisSceneSize.zw * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
