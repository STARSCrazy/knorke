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
};

// Share of the lamp the place `start` (mask texels) at height z sees, r > 0 texels from the lamp's ground point.
float Sight(SurfaceLightVaryings i, float2 start, float z, float r);

float Visibility(SurfaceLightVaryings i, Surface s)
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
        return ExemptVisibility(Mask, MaskSampler, start, ROW_GROUND(i), Domain.xy);
    }
    float r = length(start - ROW_GROUND(i));
    return r < 1e-3 ? 1.0 : Sight(i, start, z, r);
}

#define CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest) Visibility(i, s)
#include "surface_light.hlsli"

#endif
