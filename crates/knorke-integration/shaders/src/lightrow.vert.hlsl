// Light rows (ADR 0019): one instance per lamp for every row program.
//   AtlasRows > 0  row build and edge pass: the lamp's row of the atlas, scene.x the angle as a fraction of the turn
//   AtlasRows = 0  light: the scene rectangle the lamp can reach, scene the scene pixel
// CHI_LIGHT_MASK (lightrow_mask.vert, with CHI_POINT_LIGHTS as Chiaro's `_mask` programs): the mask instead of the
// area light fields.
// Origin of the light rectangle and of the varyings: chiaro @ 9bc5b9b, shaders/src/light_surface.vert.hlsl.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as such
// (this file is altered: row lamps, the atlas mode) and this notice may not be removed or altered from any source
// distribution. Full text: LICENSE-CHIARO-ZLIB.txt.
#include "lightrow.hlsli"

StructuredBuffer<RowLamp> Lamps : register(t0, space0);

// `LightSurfacePassParams` of knorke-light2d; the row programs read its `mask_base` as AtlasRows.
cbuffer LightRowPass : register(b0, space1)
{
    float4 OriginXAxis;    // xy projection origin, zw x axis (scene pixels)
    float4 YAxisSceneSize; // xy y axis, zw scene size in pixels
    float4 ZAxisMaxHeight; // xy z axis, z highest height a lit pixel can have
    uint InstanceOffset;   // index of this draw's first lamp in Lamps
    uint AtlasRows;        // height of the row atlas in texels; 0 draws the light
    uint2 Pad;
};

struct VSOutput
{
    CHI_SURFACE_LIGHT_VARYINGS
    float4 position : SV_Position;
};

static const float2 kCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

VSOutput main(uint vertexId : SV_VertexID, uint instanceId : SV_InstanceID)
{
    RowLamp lamp = Lamps[instanceId + InstanceOffset];
    GpuLight light = lamp.light;
    float2 corner = kCorners[vertexId];
    VSOutput o;
    o.positionRadius = light.positionRadius;
    o.colorFalloff = light.colorFalloff;
    o.directionType = light.directionType;
    // As light_surface.vert: the specular factor as spotCos.z. shadow.xy carry the row (lightrow.hlsli).
    o.spotCos = float4(light.spotCos.xy, light.shadow.y, light.spotCos.w);
    o.shadow = float4(lamp.row.xy, light.shadow.zw);
    o.profile = light.profile;
#ifdef CHI_LIGHT_MASK
    o.maskN = lamp.mask.n;
    o.maskU = lamp.mask.p;
    o.maskV = lamp.mask.q;
#else
    o.area = light.area;
    o.areaMask = light.areaMask;
#endif
    if (AtlasRows > 0u) {
#ifndef CHI_LIGHT_MASK
        // The atlas programs read no area light fields (a line or rect light's rows are built from its support
        // points): they carry the window the row begins at (lightrow_build.frag).
        o.area = lamp.window;
        o.areaMask = lamp.windowV;
#endif
        o.scene = corner;
        o.position = float4(corner.x * 2.0 - 1.0, 1.0 - 2.0 * (lamp.row.x + corner.y) / float(AtlasRows), 0.0, 1.0);
        return o;
    }
    float g = light.spotCos.z;
    float2 lo = float2(1e30, 1e30);
    float2 hi = float2(-1e30, -1e30);
    [unroll] for (int k = 0; k < 4; ++k) {
        float2 world = light.positionRadius.xy + float2((k & 1) ? g : -g, (k & 2) ? g : -g);
        float2 ground = OriginXAxis.xy + world.x * OriginXAxis.zw + world.y * YAxisSceneSize.xy;
        float2 raised = ground + ZAxisMaxHeight.z * ZAxisMaxHeight.xy;
        lo = min(lo, min(ground, raised));
        hi = max(hi, max(ground, raised));
    }
    float2 scene = lerp(lo, hi, corner);
    o.scene = scene;
    o.position = float4(scene / YAxisSceneSize.zw * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
