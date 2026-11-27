// Origin: chiaro @ 9bc5b9b, shaders/src/medium_light.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium stage, light pass (spec 2026-11-13-medien-volumetrik, "Medium-Stufe und Composite" step 4): one instance
// per light in the medium (CollectMediumLights). Each instance draws the scene-space bounding rectangle of the light's
// reach: the world box (x +- g, y +- g) x [bottom, top], g the horizontal half extent (spotCos.z), bottom and top
// the reach cut to the media (areaMask.xy), grown by one scatter texel so that every texel whose ray can meet the
// reach is shaded.
// CHI_LIGHT_MASK (medium_light_mask.vert): the masked lights, which follow the others in Lights from
// LightOffset.x on; Masks (light_mask.hlsli) holds only their masks, Masks[i] for Lights[LightOffset.x + i].
#include "light_common.hlsli"

StructuredBuffer<GpuLight> Lights : register(t0, space0);
#ifdef CHI_LIGHT_MASK
StructuredBuffer<GpuMask> Masks : register(t1, space0);
#endif

cbuffer MediumLightPassParams : register(b0, space1)
{
    float4 OriginXAxis; // xy projection origin, zw x axis (scene pixels)
    float4 YAxisZAxis;  // xy y axis, zw z axis
    float4 Sizes;       // xy scene size, zw scatter buffer size
#ifdef CHI_LIGHT_MASK
    uint4 LightOffset;  // x index of the first masked light in Lights (its mask is Masks[0])
#endif
};

struct VSOutput
{
    float2 scatter : TEXCOORD0; // position in scatter buffer texels
    nointerpolation float4 positionRadius : TEXCOORD1;
    nointerpolation float4 colorFalloff : TEXCOORD2;
    nointerpolation float4 directionType : TEXCOORD3;
    nointerpolation float4 spotCos : TEXCOORD4;
    nointerpolation float4 profile : TEXCOORD5;
    nointerpolation float4 area : TEXCOORD6;
    nointerpolation float2 reach : TEXCOORD7; // heights [bottom, top] of the reach
    nointerpolation float shaftRow : TEXCOORD8; // polar atlas row + 1 (GpuLight::shadow.x), 0 without shafts
#ifdef CHI_LIGHT_MASK
    nointerpolation float4 maskN : TEXCOORD9;
    nointerpolation float4 maskU : TEXCOORD10;
    nointerpolation float4 maskV : TEXCOORD11;
#endif
    float4 position : SV_Position;
};

static const float2 kCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

float2 SceneOf(float3 world)
{
    return OriginXAxis.xy + world.x * OriginXAxis.zw + world.y * YAxisZAxis.xy + world.z * YAxisZAxis.zw;
}

VSOutput main(uint vertexId : SV_VertexID, uint instanceId : SV_InstanceID)
{
#ifdef CHI_LIGHT_MASK
    const uint index = instanceId + LightOffset.x;
#else
    const uint index = instanceId;
#endif
    GpuLight light = Lights[index];
    float g = light.spotCos.z;
    float2 lo = float2(3.4e38, 3.4e38);
    float2 hi = -lo;
    [unroll] for (int k = 0; k < 8; ++k) {
        float3 c = float3(light.positionRadius.xy + float2((k & 1) ? g : -g, (k & 2) ? g : -g),
                          (k & 4) ? light.areaMask.y : light.areaMask.x);
        float2 s = SceneOf(c);
        lo = min(lo, s);
        hi = max(hi, s);
    }
    float2 texel = Sizes.xy / Sizes.zw; // one scatter texel in scene pixels
    lo -= texel;
    hi += texel;
    float2 scene = lerp(lo, hi, kCorners[vertexId]);
    float2 ndc = scene / Sizes.xy * float2(2.0, -2.0) + float2(-1.0, 1.0);

    VSOutput o;
    o.scatter = scene / Sizes.xy * Sizes.zw;
    o.positionRadius = light.positionRadius;
    o.colorFalloff = light.colorFalloff;
    o.directionType = light.directionType;
    o.spotCos = light.spotCos;
    o.profile = light.profile;
    o.area = light.area;
    o.reach = light.areaMask.xy;
    o.shaftRow = light.shadow.x;
#ifdef CHI_LIGHT_MASK
    GpuMask m = Masks[instanceId];
    o.maskN = m.n;
    o.maskU = m.p;
    o.maskV = m.q;
#endif
    o.position = float4(ndc, 0.0, 1.0);
    return o;
}
