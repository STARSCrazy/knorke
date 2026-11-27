// Origin: chiaro @ 9bc5b9b, shaders/src/medium_base.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium stage, base pass (spec 2026-11-13-medien-volumetrik, "Medium-Stufe und Composite"): fullscreen over the
// scatter buffer at medium resolution. A: transmittance T of the view ray from the visible point up through the
// medium layer's slab and the haze (analytic, medium_common.hlsli). RGB: ambient light scattered toward the viewer,
// ambient * albedo * (1 - T). Later passes add the scattered light of each light into RGB only.
// t0 surface buffer, t1 medium layer; a missing buffer's slot holds the scene and is not read.
// With ambient zones (medium_base_zones, CHI_AMBIENT_ZONES) the ambient is the pure ambient of the zone pass (t2,
// linear, at the light buffer's resolution) minus the flat directional sum in ZoneParams (b1), not MediumAmbient.
#include "common.hlsli"
#include "surface.hlsli"
#include "medium_common.hlsli"

Texture2D<float4> SurfaceTex : register(t0, space2);
Texture2D<float4> MediumTex : register(t1, space2);
SamplerState SurfaceSampler : register(s0, space2); // nearest
SamplerState MediumSampler : register(s1, space2);  // nearest
#ifdef CHI_AMBIENT_ZONES
Texture2D<float4> PureAmbientTex : register(t2, space2);
SamplerState PureAmbientSampler : register(s2, space2); // linear

// Matches struct ZoneParams in src/rendering/MediumStage.cpp.
cbuffer ZoneParams : register(b1, space3)
{
    float4 Directional; // rgb the flat directional sum in the zone pass's start value, subtracted again
};

// MediumAmbientScatter (medium_common.hlsli) with the ambient of the pixel.
float3 ZoneAmbientScatter(float3 ambient, MediumColumn c, float t)
{
    float tau = c.tauLayer + c.tauHaze;
    if (tau <= 0.0) {
        return float3(0.0, 0.0, 0.0);
    }
    float3 albedo = (c.albedoLayer * c.tauLayer + MediumHazeColor.rgb * c.tauHaze) / tau;
    return ambient * albedo * (1.0 - t);
}
#endif

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    int2 texel = MediumSourceTexel(int2(floor(uv * MediumSizes.zw)));
    float z0 = MediumSurfaceZ(SurfaceTex, SurfaceSampler, texel);
    MediumColumn column = MediumColumnAt(z0, MediumLayerAt(MediumTex, MediumSampler, texel));
    float t = MediumTransmittance(column);
#ifdef CHI_AMBIENT_ZONES
    float3 ambient = max(PureAmbientTex.SampleLevel(PureAmbientSampler, uv, 0.0).rgb - Directional.rgb, 0.0);
    return float4(ZoneAmbientScatter(ambient, column, t), t);
#else
    return float4(MediumAmbientScatter(column, t), t);
#endif
}
