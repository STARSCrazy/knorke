// Origin: chiaro @ 9bc5b9b, shaders/src/ambient_zone.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Ambient zone pass (Lichtquellen 6): the zone's weight w at the pixel's world point (SurfaceAt, as the light
// programs) and its colour, blended SRC_ALPHA / ONE_MINUS_SRC_ALPHA into RGB of the light buffer, which starts at
// the global ambient plus the flat directional sum. The hardware keeps the primitive order per pixel, so the
// instances fold A_i = A_(i-1) + w_i (C_i - A_(i-1)) in fold order. w = 0 discards: the pixel keeps its bits.
// ambient_zone_dual (CHI_AMBIENT_DUAL) writes the same into a second target, the pure ambient for the composite in
// direction mode and the medium base pass, while the light pass adds the lights into the first.
// t0 surface buffer (nearest; a stand-in that is not read without one), t1 the polygon vertex texture (nearest,
// R32G32_FLOAT, one row per polygon zone; a stand-in without polygon zones). Texels are read with SampleLevel at the
// texel centre and a nearest sampler: Load would make the texture a storage texture in the reflection.

#include "surface.hlsli"
#include "ambient_zone.hlsli"

Texture2D<float4> SurfaceTex : register(t0, space2);
Texture2D<float4> VertexTex : register(t1, space2);
SamplerState SurfaceSampler : register(s0, space2); // nearest
SamplerState VertexSampler : register(s1, space2);  // nearest

// Matches struct ZoneFragParams in src/rendering/AmbientZonePass.cpp.
cbuffer ZoneFragParams : register(b0, space3)
{
    float4 InvAxes;     // rows of the inverse of [x_axis y_axis]
    float4 OriginZAxis; // xy projection origin, zw z_axis (scene pixels per unit of height)
    float4 SceneStep;   // xy scene size in pixels, z surface height step, w 1 = surface buffer present
    float4 VertexSize;  // xy 1 / (width, rows) of the vertex texture
};

struct PSInput
{
    CHI_AMBIENT_ZONE_VARYINGS
};

struct PSOutput
{
    float4 light : SV_Target0;
#ifdef CHI_AMBIENT_DUAL
    float4 ambient : SV_Target1;
#endif
};

PSOutput main(PSInput i)
{
    float3 world;
    SurfaceAt(SurfaceTex, SurfaceSampler, i.scene, SceneStep, world, InvAxes, OriginZAxis);
    float w = ZoneWeight(i.color, i.shape, i.span, i.geom, world, VertexTex, VertexSampler, VertexSize.xy);
    if (w <= 0.0) {
        discard;
    }
    PSOutput o;
    o.light = float4(i.color.rgb, w);
#ifdef CHI_AMBIENT_DUAL
    o.ambient = o.light;
#endif
    return o;
}
