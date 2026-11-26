// Light row probe (ADR 0019, throwaway): lights one lamp's rectangle, additive into the light buffer.
// Falloff, cone and Lambert as Chiaro computes them; the including program brings the shadow as `Sight`.
//
// Origin of the surface decoding, the ray start, the falloff, the cone and the Lambert term: chiaro @ 9bc5b9b,
// shaders/src/normal_codec.hlsli (DecodeNormal), surface.hlsli (DecodeSurface, SurfaceAt, GroundMask,
// NormalStepInMask, SurfaceRayStart, SurfaceLambert, CHI_SURFACE_PARAMS), light_common.hlsli (PointReach,
// ReceiverInObstacle), shadow_trace.hlsli (ReceiverOnObstacle) and surface_light.hlsli (the body's order).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as such
// (this file is altered: one lamp kind, no profile, mask, material or area lights) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.
#ifndef KNORKE_LIGHTROW_LIGHT_HLSLI
#define KNORKE_LIGHTROW_LIGHT_HLSLI

#include "lightrow.hlsli"

Texture2D<float4> SurfaceTex : register(t0, space2);
Texture2D<float4> Mask : register(t1, space2);
Texture2D<float4> Heights : register(t2, space2); // RGBA16F (-base, top); free texels hold -65504
SamplerState SurfaceSampler : register(s0, space2); // nearest
SamplerState MaskSampler : register(s1, space2);    // nearest
SamplerState HeightSampler : register(s2, space2);  // nearest

// `SurfaceParams` of knorke-light2d (CHI_SURFACE_PARAMS).
cbuffer SurfaceParams : register(b0, space3)
{
    float4 InvAxes;       // ground = (dot(InvAxes.xy, v), dot(InvAxes.zw, v)), v = scene - origin - z * z_axis
    float4 OriginZAxis;   // xy projection origin, zw z_axis (scene pixels per unit of height)
    float4 SceneStep;     // xy scene size in pixels, z surface height step, w 1 = surface buffer present
    float4 MaskMap;       // xy mask texels per scene pixel, z mask margin in scene pixels
    float4 MaskRemainder; // xy mask anchor remainder
    float4 XYAxes;        // xy x_axis, zw y_axis (scene pixels per world unit)
    float4 ViewDir;
    float4 ScreenRight;
    float4 ZAxisSpec;
    float4 Wet;
};

cbuffer LookupParams : register(b1, space3)
{
    float4 RowSize; // x angles, y lamps (rows of the row atlas), z texels before the receiver that do not count
                    // (Chiaro: 1), w tolerance below a top (Chiaro's heightParams.y)
    float4 Domain;  // xy mask size in texels, z 1 = blend the two neighbouring angles, w the reference's lamp
                    // points per axis
    float4 WorldFromMask; // world = (dot(xy, m), dot(zw, m)) for a vector m in mask texels
    float4 Soft;          // x angles the edge pass walked to each side
};

static const float kMinObstacleOpacity = 0.5 / 255.0;

float3 DecodeNormal(float2 xy01)
{
    float2 bytes = round(saturate(xy01) * 255.0);
    float2 q = clamp((bytes - 128.0) / 127.0, -1.0, 1.0);
    float2 p = float2(q.x + q.y, q.x - q.y) * 0.5;
    return normalize(float3(p.x, p.y, 1.0 - abs(p.x) - abs(p.y)));
}

struct Surface
{
    float z;      // height in world units
    float3 n;     // unit normal in world space
    bool present; // the pixel has an entry
};

// The surface of the scene pixel under `scene` and its world position.
Surface SurfaceAt(float2 scene, out float3 world)
{
    Surface s;
    s.z = 0.0;
    s.n = float3(0.0, 0.0, 1.0);
    s.present = false;
    if (SceneStep.w != 0.0) {
        int2 texel = clamp(int2(floor(scene)), int2(0, 0), int2(SceneStep.xy) - 1);
        float4 c = SurfaceTex.SampleLevel(SurfaceSampler, (float2(texel) + 0.5) / SceneStep.xy, 0.0);
        uint4 b = uint4(round(saturate(c) * 255.0));
        if (b.a >= 128u) {
            s.present = true;
            s.z = float(b.r) * SceneStep.z;
            s.n = DecodeNormal(c.gb);
        }
    }
    float2 v = scene - OriginZAxis.xy - s.z * OriginZAxis.zw;
    world = float3(dot(InvAxes.xy, v), dot(InvAxes.zw, v), s.z);
    return s;
}

// Where the pixel is looked up (SurfaceRayStart): its ground point in mask texels; a pixel with an entry one texel
// plus the rounding slack out along the horizontal part of its normal and half a height step up along the vertical
// part, so facades and roofs do not shadow themselves.
void LookupPoint(float2 scene, Surface s, out float2 start, out float zStart)
{
    start = (scene - s.z * OriginZAxis.zw + MaskMap.z) * MaskMap.xy - MaskRemainder.xy;
    zStart = s.z;
    if (s.present) {
        float slack = 0.5 * SceneStep.z * length(OriginZAxis.zw * MaskMap.xy);
        float2 m = (s.n.x * XYAxes.xy + s.n.y * XYAxes.zw) * MaskMap.xy;
        float l = length(m);
        start += (l > 1e-6 ? m / l : float2(0.0, 0.0)) * (1.0 + slack);
        zStart += 0.5 * SceneStep.z * s.n.z;
    }
}

// Falloff times cone at `world`, and the unit vector toward the lamp (PointReach).
float Reach(float3 world, LightRowVaryings l, out float3 toLight)
{
    float3 toPixel = world - l.positionRadius.xyz;
    float d = length(toPixel);
    float x = d / l.positionRadius.w;
    float window = saturate(1.0 - x * x);
    float att = window > 0.0 ? pow(window, l.colorFalloff.w) : 0.0;
    float spot = 1.0;
    float c = d > 1e-6 ? dot(toPixel / d, l.directionType.xyz) : 1.0;
    if (l.directionType.w > 0.5) {
        float range = l.cone.x - l.cone.y;
        if (range > 1e-6) {
            spot = smoothstep(l.cone.y, l.cone.x, c);
        } else {
            spot = c >= l.cone.x ? 1.0 : 0.0;
        }
    }
    toLight = d > 1e-6 ? -toPixel / d : float3(0.0, 0.0, 0.0);
    return att * spot;
}

// Receiver rule (ReceiverOnObstacle): a ground pixel on an obstacle texel that stands on the ground is lit as if
// nothing occluded it.
bool OnObstacle(float2 p)
{
    int2 texel = int2(floor(p));
    if (any(texel < 0) || any(texel >= int2(Domain.xy))) {
        return false;
    }
    float2 uv = (float2(texel) + 0.5) / Domain.xy;
    if (Mask.SampleLevel(MaskSampler, uv, 0.0).r < kMinObstacleOpacity) {
        return false;
    }
    return 0.0 >= -Heights.SampleLevel(HeightSampler, uv, 0.0).r;
}

// 0 if an obstacle stands between the lamp and the place `start` (mask texels) at height z, r > 0 texels from the
// lamp's ground point; else 1.
float Sight(LightRowVaryings l, float2 start, float z, float r);

float Visibility(LightRowVaryings l, Surface s, float2 scene)
{
    float2 start;
    float z;
    LookupPoint(scene, s, start, z);
    if (!s.present && OnObstacle(start)) {
        return 1.0;
    }
    float r = length(start - l.mask.xy);
    return r < 1e-3 ? 1.0 : Sight(l, start, z, r);
}

float4 main(LightRowVaryings i) : SV_Target0
{
    float3 world;
    Surface s = SurfaceAt(i.coord, world);
    float3 toLight;
    float reach = Reach(world, i, toLight);
    float lambert = any(toLight != 0.0) ? max(0.0, dot(s.n, toLight)) : 1.0;
    float3 light = i.colorFalloff.rgb * (reach * lambert);
    if (all(light <= 0.0)) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    return float4(light * Visibility(i, s, i.coord), 0.0);
}

#endif
