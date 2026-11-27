// Origin: chiaro @ 9bc5b9b, shaders/src/composite.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Composite stage: scene -> linear HDR buffer, multiplied by the light.
// With a light pass the light comes from the light buffer; without one
// (no visible point/spot lights) from the constant Ambient (ambient plus
// directional lights). With a glow buffer (E26) its glow is added on top
// as unlit HDR light: colour * (A / 255) * GlowScale, the colour from the
// buffer or, with GlowFromScene, from the scene pixel.
// With the specular buffer of the exact material mode (UseSpecular) its
// highlights are added as specular * lerp(1, scene, metal), the metal bit read
// from the material buffer: a metal highlight takes the scene colour, any
// other stays the light colour.
// In direction mode (material buffer, light buffer smaller than the scene,
// UseDirection) t4 holds the direction sums of the light pass instead, and the
// detail normal of each scene pixel corrects the direct light and adds the
// highlight (DirectionModeLight in surface.hlsli); the base normal comes from the
// surface buffer in t5, or is up when ScreenRight.w = 0.
// Unused slots hold the scene texture and are not read.
// With a medium (variant composite_medium, CHI_MEDIUM) the pre-filtered scatter buffer of the medium stage is upsampled
// edge-aware (MediumUpsample in medium_common.hlsli: t6 filtered scatter buffer, t8 its guides, the pixel's guide from
// the surface buffer in t5 and the medium layer in t7; composite_medium_nearest, CHI_MEDIUM_NEAREST, takes one whole
// texel, MediumUpsampleNearest) and the colour becomes colour * T + scattered light. Glow and highlights sit in the medium and are
// dimmed by T; the overlay (output stage) is not.
// With ambient zones in direction mode (variants *_zones, CHI_AMBIENT_ZONES) the pure ambient of each pixel comes from
// the ambient zone pass (t6 without, t9 with CHI_MEDIUM; linear like the light buffer) instead of Ambient.rgb.
#include "common.hlsli"
#include "surface.hlsli"

Texture2D<float4> Scene : register(t0, space2);
Texture2D<float4> LightBuffer : register(t1, space2);
Texture2D<float4> GlowTex : register(t2, space2);
Texture2D<float4> MaterialTex : register(t3, space2);
Texture2D<float4> DetailTex : register(t4, space2);  // specular (exact mode) or direction sums (direction mode)
Texture2D<float4> SurfaceTex : register(t5, space2); // direction mode: base normal
SamplerState SceneSampler : register(s0, space2);
SamplerState LightSampler : register(s1, space2);
SamplerState GlowSampler : register(s2, space2);     // nearest
SamplerState MaterialSampler : register(s3, space2); // nearest
SamplerState DetailSampler : register(s4, space2);  // linear, like the light buffer
SamplerState SurfaceSampler : register(s5, space2); // nearest

cbuffer CompositeParams : register(b0, space3)
{
    uint DecodeSrgb;
    uint UseLightBuffer;
    uint UseGlow;
    uint DecodeGlow;    // glow colour is sRGB-encoded 8-bit UNORM
    float4 Ambient;     // rgb: light used when UseLightBuffer == 0
    uint GlowFromScene; // CHI_GLOW_COLOR_SCENE
    float GlowScale;
    uint UseSpecular;   // specular buffer of the exact material mode
    uint UseDirection;  // direction sums of the direction material mode; Ambient is then the pure ambient
    float4 ViewDirSpec;  // direction mode: xyz unit direction toward the viewer, w specular_scale
    float4 ScreenRight; // direction mode: xyz screen-right world direction, w 1 = surface buffer in t5
    float4 XYAxes;       // direction mode: xy x_axis, zw y_axis
    float4 ZAxisScene;   // direction mode: xy z_axis, zw scene size in pixels
    float4 InvAxes;      // direction mode, wet ground: rows of the inverse of [x_axis y_axis]
    float4 OriginStep;   // direction mode, wet ground: xy projection origin, z surface height step
    float4 Wet;          // direction mode: wet ground (surface.hlsli), x stretch (1 off), y breakup, z 1 / scale
};

#ifdef CHI_MEDIUM
#define CHI_MEDIUM_PARAMS_SLOT b1
#include "medium_common.hlsli"
Texture2D<float4> ScatterTex : register(t6, space2);
Texture2D<float4> MediumTex : register(t7, space2);
Texture2D<float4> GuideTex : register(t8, space2);
SamplerState ScatterSampler : register(s6, space2); // nearest
SamplerState MediumSampler : register(s7, space2);  // nearest
SamplerState GuideSampler : register(s8, space2);   // nearest
#endif

#ifdef CHI_AMBIENT_ZONES
#ifdef CHI_MEDIUM
Texture2D<float4> PureAmbientTex : register(t9, space2);
SamplerState PureAmbientSampler : register(s9, space2); // linear
#else
Texture2D<float4> PureAmbientTex : register(t6, space2);
SamplerState PureAmbientSampler : register(s6, space2); // linear
#endif
#define CHI_DIRECTION_AMBIENT PureAmbientTex.SampleLevel(PureAmbientSampler, uv, 0.0).rgb
#else
#define CHI_DIRECTION_AMBIENT Ambient.rgb
#endif

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float4 c = Scene.SampleLevel(SceneSampler, uv, 0.0);
    if (DecodeSrgb != 0u) {
        c.rgb = SrgbToLinear(c.rgb);
    }
    float3 light = Ambient.rgb;
    if (UseLightBuffer != 0u) {
        light = LightBuffer.SampleLevel(LightSampler, uv, 0.0).rgb;
    }
    float3 sceneColor = c.rgb;
    if (UseDirection != 0u) {
        float2 size = ZAxisScene.zw;
        float2 scene = uv * size;
        int2 texel = SceneTexel(scene, size);
        Material m = DecodeMaterial(MaterialTexel(MaterialTex, MaterialSampler, texel, size));
        if (!m.present) {
            // DirectionModeLight without an entry: k = 1 and no highlight (strength 0), so neither the base normal
            // nor the direction sums are needed. Same expression as there, so the same value.
            c.rgb = sceneColor * (CHI_DIRECTION_AMBIENT + max(light - CHI_DIRECTION_AMBIENT, 0.0));
        } else {
            float3 baseN = float3(0.0, 0.0, 1.0);
            float z = 0.0;
            if (ScreenRight.w != 0.0) {
                Surface s = DecodeSurface(MaterialTexel(SurfaceTex, SurfaceSampler, texel, size), OriginStep.z);
                baseN = s.n;
                z = s.z;
            }
            float2 v = scene - OriginStep.xy - z * ZAxisScene.xy; // the pixel's world xy, as SurfaceAt
            float2 ground = float2(dot(InvAxes.xy, v), dot(InvAxes.zw, v));
            m.exponent = MaterialExponent(MaterialTex, MaterialSampler, texel, size, m, ViewDirSpec.w);
            float3 n = MaterialShadingNormal(baseN, ScreenRight.xyz, XYAxes, ZAxisScene.xy, m);
            float3 specular;
            light = DirectionModeLight(light, CHI_DIRECTION_AMBIENT, DetailTex.SampleLevel(DetailSampler, uv, 0.0), baseN, n,
                                       m, ViewDirSpec.xyz, ViewDirSpec.w, ScreenRight.xyz, ground, Wet, specular);
            c.rgb = sceneColor * light + specular * lerp(float3(1.0, 1.0, 1.0), sceneColor, m.metal);
        }
    } else {
        c.rgb *= light;
    }
    if (UseSpecular != 0u) {
        float3 specular = DetailTex.SampleLevel(DetailSampler, uv, 0.0).rgb;
        float metal = DecodeMaterial(MaterialTex.SampleLevel(MaterialSampler, uv, 0.0)).metal;
        c.rgb += specular * lerp(float3(1.0, 1.0, 1.0), sceneColor, metal);
    }
    if (UseGlow != 0u) {
        float4 e = GlowTex.SampleLevel(GlowSampler, uv, 0.0);
        float3 glow = e.rgb;
        if (GlowFromScene != 0u) {
            glow = sceneColor;
        } else if (DecodeGlow != 0u) {
            glow = SrgbToLinear(glow);
        }
        c.rgb += glow * (e.a * GlowScale);
    }
#ifdef CHI_MEDIUM
#ifdef CHI_MEDIUM_NEAREST
    float4 medium = MediumUpsampleNearest(ScatterTex, ScatterSampler, GuideTex, GuideSampler, SurfaceTex,
                                          SurfaceSampler, MediumTex, MediumSampler, uv);
#else
    float4 medium = MediumUpsample(ScatterTex, ScatterSampler, GuideTex, GuideSampler, SurfaceTex, SurfaceSampler,
                                   MediumTex, MediumSampler, uv);
#endif
    c.rgb = c.rgb * medium.a + medium.rgb;
#endif
    return c;
}
