// Light rows (ADR 0019, plan 17.13 L3): a sun's light at a surface pixel, one fullscreen draw into the targets
// of Chiaro's light pass. The light is Chiaro's directional_surface.frag without the pattern variants
// (CHI_MATERIAL_EXACT: Lambert with the detail normal, the highlight to SV_Target1; CHI_MATERIAL_DIRECTION: the
// direction sample to SV_Target1), KNORKE_FLAT Chiaro's directional.frag (no surface and no material buffer: every
// pixel is ground, no Lambert term), each with its medium variant (CHI_MEDIUM); only the visibility differs. The
// including program defines SunSight and, with CHI_MEDIUM, the medium map's slots.
// Origin of the light body below: chiaro @ 9bc5b9b, shaders/src/directional_surface.frag.hlsl and
// directional.frag.hlsl. Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without
// any express or implied warranty; the origin must not be misrepresented; altered source versions must be plainly
// marked as such (this file is altered: the shadow from SunSight, no pattern variants, one file for the
// material variants and the flat light) and this notice may not be removed or altered from any source distribution.
// Full text: LICENSE-CHIARO-ZLIB.txt.
#ifndef KNORKE_LIGHTROW_SUN_SURFACE_HLSLI
#define KNORKE_LIGHTROW_SUN_SURFACE_HLSLI

#include "lightrow_sun.hlsli"
#include "light_common.hlsli"
#include "surface.hlsli"

Texture2D<float4> SurfaceTex : register(t0, space2);
Texture2D<float4> Mask : register(t1, space2);
Texture2D<float4> Heights : register(t2, space2); // RGBA16F, .ba the finite obstacles (-base, top)
SamplerState SurfaceSampler : register(s0, space2); // nearest
SamplerState MaskSampler : register(s1, space2);    // nearest
SamplerState HeightSampler : register(s2, space2);  // nearest
#ifdef CHI_MATERIAL_EXACT // after the map, the runs and the pattern atlas of lightrow_sun_light.hlsli
Texture2D<float4> MaterialTex : register(t8, space2);
SamplerState MaterialSampler : register(s8, space2); // nearest
#endif

cbuffer SurfaceParams : register(b0, space3)
{
    CHI_SURFACE_PARAMS
};

cbuffer SunParams : register(b1, space3)
{
    LIGHTROW_SUN_PARAMS
};

#ifdef CHI_MEDIUM
#include "medium_transmit.hlsli"
#endif

// Share of the sun the place `start` (mask texels) at height z sees.
float SunSight(float2 start, float z);

// Visibility of the sun from surface pixel `scene` (base surface s). A sun without shadows (Color.w 0) or straight
// overhead sees everything, as in Chiaro's directional programs; it binds no map, and stand-ins for the textures.
float Visibility(float2 scene, Surface s)
{
    if (Color.w < 0.5 || all(DirSlope.xy == 0.0)) {
        return 1.0;
    }
    float2 start;
    float z;
    SurfaceRayStart(scene, s, OriginZAxis, MaskMap, MaskRemainder, XYAxes, SceneStep.z, start, z);
    // Receiver rule (Chiaro's ReceiverOnObstacle): a ground pixel on an obstacle that stands on the ground is lit as
    // if nothing occluded it.
    if (!s.present && ReceiverInObstacle(Mask, MaskSampler, start, Domain.xy)
        && 0.0 >= -FetchTexel(Heights, HeightSampler, int2(floor(start)), 1.0 / Domain.xy).r) {
        return ExemptVisibility(Mask, MaskSampler, start, start + DirSlope.xy, Domain.xy);
    }
    return SunSight(start, z);
}

#if defined(CHI_MATERIAL_EXACT)
struct LightOutput
{
    float4 light : SV_Target0;
    float4 specular : SV_Target1;
};

LightOutput main(float2 uv : TEXCOORD0)
{
    float2 scene = uv * SceneStep.xy;
    float3 world;
    Surface s = SurfaceAt(SurfaceTex, SurfaceSampler, scene, SceneStep, world, InvAxes, OriginZAxis);
    Material m;
    float3 n = MaterialNormalAt(MaterialTex, MaterialSampler, scene, SceneStep, s.n, ScreenRight.xyz, XYAxes,
                                ZAxisSpec.xy, ZAxisSpec.z, m);
    LightOutput o;
    o.light = float4(0.0, 0.0, 0.0, 0.0);
    o.specular = float4(0.0, 0.0, 0.0, 0.0);
    float lambert = max(0.0, dot(n, World.xyz));
    if (lambert <= 0.0) {
        return o;
    }
    float visibility = Visibility(scene, s);
#ifdef CHI_MEDIUM
    float3 color = Color.rgb * MediumTransmitToward(world, World.xyz, kLightMediumSamples);
    o.light = float4(color * (lambert * visibility), 0.0);
    o.specular = float4(SurfaceSpecularWet(color, visibility, n, World.xyz, m, ViewDir.xyz, ZAxisSpec.z * World.w, s.n,
                                           ScreenRight.xyz, world.xy, Wet), 0.0);
#else
    o.light = float4(Color.rgb * (lambert * visibility), 0.0);
    o.specular = float4(SurfaceSpecularWet(Color.rgb, visibility, n, World.xyz, m, ViewDir.xyz, ZAxisSpec.z * World.w, s.n,
                                           ScreenRight.xyz, world.xy, Wet), 0.0);
#endif
    return o;
}
#elif defined(CHI_MATERIAL_DIRECTION)
struct LightOutput
{
    float4 light : SV_Target0;
    float4 direction : SV_Target1;
};

LightOutput main(float2 uv : TEXCOORD0)
{
    float2 scene = uv * SceneStep.xy;
    float3 world;
    Surface s = SurfaceAt(SurfaceTex, SurfaceSampler, scene, SceneStep, world, InvAxes, OriginZAxis);
    LightOutput o;
    o.light = float4(0.0, 0.0, 0.0, 0.0);
    o.direction = float4(0.0, 0.0, 0.0, 0.0);
    float lambert = max(0.0, dot(s.n, World.xyz));
    if (lambert <= 0.0) {
        return o;
    }
#ifdef CHI_MEDIUM
    float3 visibility = Visibility(scene, s) * MediumTransmitToward(world, World.xyz, kLightMediumSamples);
#else
    float visibility = Visibility(scene, s);
#endif
    o.light = float4(Color.rgb * (lambert * visibility), 0.0);
    o.direction = DirectionSample(Color.rgb * visibility, World.xyz, lambert, World.w);
    return o;
}
#elif defined(KNORKE_FLAT)
float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float2 scene = uv * SceneStep.xy;
    float3 world;
    Surface s = SurfaceAt(SurfaceTex, SurfaceSampler, scene, SceneStep, world, InvAxes, OriginZAxis);
#ifdef CHI_MEDIUM
    float3 ground = float3(MediumMapGround(scene), 0.0);
    return float4(Color.rgb * Visibility(scene, s) * MediumTransmitToward(ground, World.xyz, kLightMediumSamples), 0.0);
#else
    return float4(Color.rgb * Visibility(scene, s), 0.0);
#endif
}
#else
float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float2 scene = uv * SceneStep.xy;
    float3 world;
    Surface s = SurfaceAt(SurfaceTex, SurfaceSampler, scene, SceneStep, world, InvAxes, OriginZAxis);
    float lambert = max(0.0, dot(s.n, World.xyz));
    if (lambert <= 0.0) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
#ifdef CHI_MEDIUM
    return float4(Color.rgb * lambert * Visibility(scene, s) * MediumTransmitToward(world, World.xyz, kLightMediumSamples),
                  0.0);
#else
    return float4(Color.rgb * (lambert * Visibility(scene, s)), 0.0);
#endif
}
#endif

#endif
