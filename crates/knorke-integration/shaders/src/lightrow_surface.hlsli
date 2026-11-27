// Light rows (ADR 0019): the light of one row lamp at a surface pixel, into the targets of Chiaro's light pass.
// The light itself is Chiaro's surface light body (surface_light.hlsli: falloff, cone, Lambert, near field, and
// with CHI_MATERIAL_EXACT the highlight to SV_Target1, with CHI_MATERIAL_DIRECTION the direction sample); only the
// visibility differs. The including program defines `Sight` after this file.
// Origin of the receiver rule: chiaro @ 9bc5b9b, shaders/src/shadow_trace.hlsli (ReceiverOnObstacle) and
// light_sdf_surface.frag.hlsl (Visibility); zlib license, see light_common.hlsli.
#ifndef KNORKE_LIGHTROW_SURFACE_HLSLI
#define KNORKE_LIGHTROW_SURFACE_HLSLI

#include "lightrow.hlsli"

Texture2D<float4> SurfaceTex : register(t0, space2);
Texture2D<float4> Mask : register(t1, space2);
Texture2D<float4> Heights : register(t2, space2); // RGBA16F (-base, top); free texels hold -65504
SamplerState SurfaceSampler : register(s0, space2); // nearest
SamplerState MaskSampler : register(s1, space2);    // nearest
SamplerState HeightSampler : register(s2, space2);  // nearest
#ifdef CHI_MATERIAL_EXACT // after the row and edge atlases of lightrow_light.hlsli
Texture2D<float4> MaterialTex : register(t7, space2);
SamplerState MaterialSampler : register(s7, space2); // nearest
#endif

cbuffer SurfaceParams : register(b0, space3)
{
    CHI_SURFACE_PARAMS
};

// `RowLookupParams` of knorke-light2d.
cbuffer LookupParams : register(b1, space3)
{
    float4 RowSize;       // x angles, y rows of the row atlas, z texels before the receiver that do not count
                          // (Chiaro: 1), w tolerance below a top (Chiaro's HeightParams.y)
    float4 Domain;        // xy mask size in texels, w lightrow_truth.frag: lamp points per axis
    float4 WorldFromMask; // world = (dot(xy, m), dot(zw, m)) for a vector m in mask texels
    float4 HeightCode;    // how the rows store heights (PackRun in lightrow.hlsli)
};

#ifdef CHI_MEDIUM // the *_medium programs: the including program defines the map's slots
#include "medium_transmit.hlsli"
#endif

// A point of a line or rect light's emitter at emitter coordinates ts (t along U, s along V): its ground point in
// mask texels and its height. Chiaro's ray ends at the nearest such point (ShadowTarget in light_common.hlsli).
#ifndef CHI_POINT_LIGHTS
void EmitterPoint(SurfaceLightVaryings l, float2 ts, out float2 ground, out float zl)
{
    LightParams p = CHI_LIGHT_PARAMS(l);
    float3 v = IsRectLight(p) ? RectAcross(p) : float3(0.0, 0.0, 0.0);
    ground = ROW_GROUND(l) + ts.x * l.areaMask.xy + ts.y * l.areaMask.zw;
    zl = ROW_HEIGHT(l) + ts.x * l.area.z + ts.y * v.z;
}
#endif

// Share of the lamp the place `start` (mask texels) at height z sees, r > 0 texels from the lamp's ground point;
// nothing nearer the lamp than `from` texels counts.
float Sight(SurfaceLightVaryings i, float2 start, float z, float r, float from);

// maskT: the share of the way from the lamp at which the ray passes a window mask, 0 without one (a projector
// mask, no mask). Chiaro's ray then ends there and stops 1.5 texels before it (MaskShadowStop in
// light_common.hlsli); the receiver rule looks toward the window.
float Visibility(SurfaceLightVaryings i, Surface s, float maskT)
{
    if (ROW_SLOT(i) < 0.0) {
        return 1.0; // the lamp casts no shadow; mask and heights are stand-ins then
    }
    float2 start;
    float z;
    SurfaceRayStart(i.scene, s, OriginZAxis, MaskMap, MaskRemainder, XYAxes, SceneStep.z, start, z);
    // Receiver rule: a ground pixel on an obstacle that stands on the ground is lit as if nothing occluded it.
    if (!s.present && ReceiverInObstacle(Mask, MaskSampler, start, Domain.xy)
        && 0.0 >= -FetchTexel(Heights, HeightSampler, int2(floor(start)), 1.0 / Domain.xy).r) {
        return ExemptVisibility(Mask, MaskSampler, start, lerp(ROW_GROUND(i), start, maskT), Domain.xy);
    }
    float r = length(start - ROW_GROUND(i));
    float from = maskT > 0.0 ? maskT * r + 1.5 : 0.0;
    return r < 1e-3 ? 1.0 : Sight(i, start, z, r, from);
}

#ifdef CHI_LIGHT_MASK // maskT: a local of the bodies in surface_light.hlsli
#define CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest) Visibility(i, s, maskT)
#else
#define CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest) Visibility(i, s, 0.0)
#endif
#ifdef KNORKE_FLAT
// A frame without surface and material buffer: Chiaro's flat light (light_sdf.frag, light_sdf_height.frag main).
// Every pixel is ground (SurfaceParams without a surface buffer: SurfaceAt reads no texture), and the light has no
// Lambert term. With CHI_MEDIUM the light is dimmed toward the lamp's nearest point (the window with a window mask)
// where it is visible at all, as in light_sdf.frag.
#include "light_common.hlsli"
#include "surface.hlsli"

float4 main(SurfaceLightVaryings i) : SV_Target0
{
    float3 world;
    Surface s = SurfaceAt(SurfaceTex, SurfaceSampler, i.scene, SceneStep, world, InvAxes, OriginZAxis);
    LightParams l = CHI_LIGHT_PARAMS(i);
    float3 toLight, nearest;
    float2 ts;
    float3 light = i.colorFalloff.rgb * LightReachAt(world, l, toLight, nearest, ts);
#ifdef CHI_LIGHT_MASK
    float maskT;
    light *= CHI_LIGHT_MASK_TINT(i, world - i.positionRadius.xyz, maskT);
    nearest = lerp(nearest, world, maskT);
#endif
    if (all(light <= 0.0)) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    float visibility = CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest);
#ifdef CHI_MEDIUM
    if (visibility > 0.0) {
        light *= MediumTransmit(world, nearest, kLightMediumSamples);
    }
#endif
    return float4(light * visibility, 0.0);
}
#else
#include "surface_light.hlsli"
#endif

#endif
