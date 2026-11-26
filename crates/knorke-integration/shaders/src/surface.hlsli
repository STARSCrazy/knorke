// Origin: chiaro @ 9bc5b9b, shaders/src/surface.hlsli, copied unchanged below this notice for the light row
// programs (lightrow_*, ADR 0019), so they light exactly as Chiaro's light programs do.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Surface buffer (E23): per scene pixel height and normal in 8-bit RGBA
// (CHI_RenderInfo::surface). Read unfiltered.
//   R    height in steps of SurfaceHeightStep
//   G, B normal in world space, shared codec (normal_codec.hlsli)
//   A    >= 128: the pixel has an entry; otherwise it is ground (height 0, normal up)
// Also the per-pixel light model shared by the surface light programs.
#ifndef CHI_SURFACE_HLSLI
#define CHI_SURFACE_HLSLI

#include "normal_codec.hlsli"
#include "profile.hlsli"

// Varyings of light_surface.vert, read by the surface light fragment shaders. CHI_POINT_LIGHTS: without the area light
// fields (light_common.hlsli).
#if defined(CHI_LIGHT_MASK) && defined(CHI_POINT_LIGHTS)
// The `_mask` programs: the three mask vectors (light_mask.hlsli) before shadow.
#define CHI_SURFACE_LIGHT_VARYINGS                         \
    float2 scene : TEXCOORD0;                              \
    nointerpolation float4 positionRadius : TEXCOORD1;     \
    nointerpolation float4 colorFalloff : TEXCOORD2;       \
    nointerpolation float4 directionType : TEXCOORD3;      \
    nointerpolation float4 spotCos : TEXCOORD4;            \
    nointerpolation float4 profile : TEXCOORD5;            \
    nointerpolation float4 maskN : TEXCOORD6;              \
    nointerpolation float4 maskU : TEXCOORD7;              \
    nointerpolation float4 maskV : TEXCOORD8;              \
    nointerpolation float4 shadow : TEXCOORD9; /* last: the unshadowed programs do not read it (D3D12 wants a prefix) */
#elif defined(CHI_POINT_LIGHTS)
#define CHI_SURFACE_LIGHT_VARYINGS                         \
    float2 scene : TEXCOORD0;                              \
    nointerpolation float4 positionRadius : TEXCOORD1;     \
    nointerpolation float4 colorFalloff : TEXCOORD2;       \
    nointerpolation float4 directionType : TEXCOORD3;      \
    nointerpolation float4 spotCos : TEXCOORD4;            \
    nointerpolation float4 profile : TEXCOORD5;            \
    nointerpolation float4 shadow : TEXCOORD6; /* last: the unshadowed programs do not read it (D3D12 wants a prefix) */
#else
#define CHI_SURFACE_LIGHT_VARYINGS                         \
    float2 scene : TEXCOORD0;                              \
    nointerpolation float4 positionRadius : TEXCOORD1;     \
    nointerpolation float4 colorFalloff : TEXCOORD2;       \
    nointerpolation float4 directionType : TEXCOORD3;      \
    nointerpolation float4 spotCos : TEXCOORD4;            \
    nointerpolation float4 profile : TEXCOORD5;           \
    nointerpolation float4 area : TEXCOORD6;               \
    nointerpolation float4 shadow : TEXCOORD7; /* last two: the unshadowed programs do not read them (D3D12 wants a prefix) */ \
    nointerpolation float4 areaMask : TEXCOORD8;
#endif

struct SurfaceLightVaryings
{
    CHI_SURFACE_LIGHT_VARYINGS
};

struct Surface
{
    float z;         // height in world units
    float3 n;        // unit normal in world space
    bool present;    // the pixel has an entry
};

Surface DecodeSurface(float4 c, float step)
{
    uint4 b = uint4(round(saturate(c) * 255.0));
    Surface s;
    s.z = 0.0;
    s.n = float3(0.0, 0.0, 1.0);
    s.present = false;
    if (b.a < 128u) {
        return s;
    }
    s.present = true;
    s.z = float(b.r) * step;
    s.n = DecodeNormal(c.gb);
    return s;
}

// Matches struct SurfaceParams in src/rendering/LightingStage.cpp.
#define CHI_SURFACE_PARAMS                                                                              \
    float4 InvAxes;       /* ground = (dot(InvAxes.xy, v), dot(InvAxes.zw, v)), v = scene - origin - z * z_axis */ \
    float4 OriginZAxis;   /* xy projection origin, zw z_axis (scene pixels per unit of height) */      \
    float4 SceneStep;     /* xy scene size in pixels, z surface height step, w 1 = surface buffer present, 0 = every pixel is ground */                          \
    float4 MaskMap;       /* xy mask texels per scene pixel, z mask margin in scene pixels */           \
    float4 MaskRemainder; /* xy mask anchor remainder */                                            \
    float4 XYAxes;        /* xy x_axis, zw y_axis (scene pixels per world unit) */                       \
    float4 ViewDir;       /* xyz unit direction toward the viewer in world space (z > 0) */             \
    float4 ScreenRight;   /* xyz horizontal unit world direction whose image points screen-right */      \
    float4 ZAxisSpec;     /* xy z_axis, z specular_scale */                                         \
    float4 Wet;           /* x wet_stretch (1 off), y wet_breakup, z 1 / wet_scale (wet ground) */

// The texel of the scene pixel under `scene` in a buffer of scene size `size`.
int2 SceneTexel(float2 scene, float2 size)
{
    return clamp(int2(floor(scene)), int2(0, 0), int2(size) - 1);
}

// The surface of the scene pixel under `scene` and its world position.
Surface SurfaceAt(Texture2D<float4> surface, SamplerState nearestSampler, float2 scene, float4 sceneStep,
                  out float3 world, float4 invAxes, float4 originZAxis)
{
    Surface s;
    s.z = 0.0;
    s.n = float3(0.0, 0.0, 1.0);
    s.present = false;
    // Without a surface buffer (material buffer only) every pixel is ground; the slot then holds a stand-in that is
    // not read (uniform branch).
    if (sceneStep.w != 0.0) {
        int2 texel = SceneTexel(scene, sceneStep.xy);
        s = DecodeSurface(surface.SampleLevel(nearestSampler, (float2(texel) + 0.5) / sceneStep.xy, 0.0),
                          sceneStep.z);
    }
    float2 v = scene - originZAxis.xy - s.z * originZAxis.zw;
    world = float3(dot(invAxes.xy, v), dot(invAxes.zw, v), s.z);
    return s;
}

// The pixel's ground point in mask texels (the mask maps the ground plane, E20).
float2 GroundMask(float2 scene, Surface s, float4 originZAxis, float4 maskMap, float4 maskRemainder)
{
    return (scene - s.z * originZAxis.zw + maskMap.z) * maskMap.xy - maskRemainder.xy;
}

// Unit step in mask texels along the horizontal part of the normal, (0, 0) for a normal straight up.
float2 NormalStepInMask(Surface s, float4 xyAxes, float4 maskMap)
{
    float2 scene = s.n.x * xyAxes.xy + s.n.y * xyAxes.zw;
    float2 m = scene * maskMap.xy;
    float l = length(m);
    return l > 1e-6 ? m / l : float2(0.0, 0.0);
}

// Where the shadow ray of a surface pixel starts (E23). Heights are rounded to steps, so the decoded height can lie
// up to half a step below the true one, and the ground point of a vertical face can lie up to half a step times
// |z_axis| scene pixels inside its footprint. The ray therefore starts half a step higher along the vertical part
// of the normal, and one texel plus that slack out along its horizontal part. A pixel without entry starts at its
// ground point.
void SurfaceRayStart(float2 scene, Surface s, float4 originZAxis, float4 maskMap, float4 maskRemainder, float4 xyAxes,
                     float step, out float2 start, out float zStart)
{
    start = GroundMask(scene, s, originZAxis, maskMap, maskRemainder);
    zStart = s.z;
    if (s.present) {
        float slack = 0.5 * step * length(originZAxis.zw * maskMap.xy);
        start += NormalStepInMask(s, xyAxes, maskMap) * (1.0 + slack);
        zStart += 0.5 * step * s.n.z;
    }
}

// Lambert term toward a point light; 1 for a pixel at the light position.
float SurfaceLambert(float3 n, float3 toLight)
{
    return any(toLight != 0.0) ? max(0.0, dot(n, toLight)) : 1.0;
}

// ---- Material buffer (phase 4b) ----------------------------------------------------------------------------------
// Per scene pixel, 8-bit RGBA, read unfiltered (CHI_SceneBuffers::material):
//   R, G detail normal in the tangent space of the base surface, shared codec (normal_codec.hlsli)
//   B    bits 7..5 specular strength (0..7), bit 4 metal, bits 3..0 shininess g: exponent 2^(1 + g * 7/15)
//   A    >= 128: the pixel has an entry; otherwise no material (base normal, no specular highlight). 192..254: the
//        entry is wet (CHI_MATERIAL_WET, E48), only such pixels get the wet streak.
// Mirrors DecodeMaterialBits, MakeTangentFrame, DetailToWorld, ToksvigExponent and SpecularTerm in
// src/lighting/MaterialMath.cpp, the CPU oracle of the tests.

struct Material
{
    float3 detail;  // tangent space: x along T (screen-right), y along B (screen-up), z along the base normal
    float strength; // s / 7
    float metal;    // 0 or 1
    float exponent; // Blinn-Phong exponent
    bool present;
    bool wet;       // A in [192, 254]
};

Material DecodeMaterial(float4 c)
{
    uint4 b = uint4(round(saturate(c) * 255.0));
    Material m;
    m.present = b.a >= 128u;
    m.wet = b.a >= 192u && b.a < 255u;
    m.detail = float3(0.0, 0.0, 1.0);
    m.strength = 0.0;
    m.metal = 0.0;
    m.exponent = 2.0;
    if (!m.present) {
        return m;
    }
    m.detail = DecodeNormal(c.rg);
    m.strength = float(b.b >> 5) / 7.0;
    m.metal = float((b.b >> 4) & 1u);
    m.exponent = exp2(1.0 + float(b.b & 15u) * 7.0 / 15.0);
    return m;
}

// Blinn-Phong exponent e widened for a direction spread whose average unit vector has length len (Toksvig):
// len / (len + e (1 - len)) * e, at least 1. len = 1 keeps e. Mirrors ToksvigExponent in MaterialMath.cpp.
float ToksvigExponent(float e, float len)
{
    return max(1.0, len / (len + e * (1.0 - len)) * e);
}

// Tangent T (its image points screen-right) and bitangent B (screen-up) of the base normal n.
// T = normalize(R - (R.n) n); for n parallel to R (a facade seen edge-on) T = normalize(up x n), negated if its
// image points left. B = n x T, negated if its image points down (positive scene y).
void TangentFrameOf(float3 n, float3 right, float4 xyAxes, float2 zAxis, out float3 t, out float3 b)
{
    float3 proj = right - dot(right, n) * n;
    if (length(proj) < 1e-4) {
        t = normalize(cross(float3(0.0, 0.0, 1.0), n));
        if (xyAxes.x * t.x + xyAxes.z * t.y + zAxis.x * t.z < 0.0) {
            t = -t;
        }
    } else {
        t = normalize(proj);
    }
    b = cross(n, t);
    if (xyAxes.y * b.x + xyAxes.w * b.y + zAxis.y * b.z > 0.0) {
        b = -b;
    }
}

float4 MaterialTexel(Texture2D<float4> material, SamplerState nearestSampler, int2 texel, float2 size)
{
    return material.SampleLevel(nearestSampler, (float2(texel) + 0.5) / size, 0.0);
}

// Blinn-Phong exponent of a pixel with a material entry m decoded from `texel`, after specular antialiasing
// (Toksvig): the detail normals of the pixel and of its right and lower neighbours with an entry are averaged, and the
// shortening of the average lowers m.exponent (the normalization N(e) then uses the lowered exponent). The
// exponent only matters where the pixel has a highlight (m.strength * specularScale > 0, SurfaceSpecular), so only
// such pixels read the neighbours; all others get m.exponent unchanged.
float MaterialExponent(Texture2D<float4> material, SamplerState nearestSampler, int2 texel, float2 sizeF, Material m,
                       float specularScale)
{
    if (!(m.strength * specularScale > 0.0)) {
        return m.exponent;
    }
    int2 size = int2(sizeF);
    float3 sum = m.detail;
    float count = 1.0;
    if (texel.x + 1 < size.x) {
        Material r = DecodeMaterial(MaterialTexel(material, nearestSampler, texel + int2(1, 0), sizeF));
        if (r.present) {
            sum += r.detail;
            count += 1.0;
        }
    }
    if (texel.y + 1 < size.y) {
        Material d = DecodeMaterial(MaterialTexel(material, nearestSampler, texel + int2(0, 1), sizeF));
        if (d.present) {
            sum += d.detail;
            count += 1.0;
        }
    }
    float len = length(sum) / count;
    return ToksvigExponent(m.exponent, len);
}

// Shading normal in world space of a pixel with a material entry m: the detail normal turned into the tangent frame
// of the base normal n.
float3 MaterialShadingNormal(float3 n, float3 right, float4 xyAxes, float2 zAxis, Material m)
{
    float3 t, b;
    TangentFrameOf(n, right, xyAxes, zAxis, t, b);
    return normalize(m.detail.x * t + m.detail.y * b + m.detail.z * n);
}

// The material of the scene pixel under `scene` (exponent from MaterialExponent) and its shading normal
// (MaterialShadingNormal), or n itself without an entry.
float3 MaterialNormalAt(Texture2D<float4> material, SamplerState nearestSampler, float2 scene, float4 sceneStep,
                        float3 n, float3 right, float4 xyAxes, float2 zAxis, float specularScale, out Material m)
{
    int2 texel = SceneTexel(scene, sceneStep.xy);
    m = DecodeMaterial(MaterialTexel(material, nearestSampler, texel, sceneStep.xy));
    if (!m.present) {
        return n;
    }
    m.exponent = MaterialExponent(material, nearestSampler, texel, sceneStep.xy, m, specularScale);
    return MaterialShadingNormal(n, right, xyAxes, zAxis, m);
}

// Energy normalization of the lobe (E49, SpecularNormalization in src/lighting/MaterialMath.cpp): N(e) (n.h)^e seen
// along the normal sums to pi over all light directions for every e, as the Lambert term does.
float SpecularNormalization(float e)
{
    return (e + 2.0) / (8.0 * (1.0 - exp2(-0.5 * (e + 2.0))));
}

// Normalized Blinn-Phong specular N(e) * (n.H)^e for the unit light direction l; 0 where n.l <= 0, so back
// sides have no highlight. l = 0 (light at the pixel, no direction) gives n.l = 0 and so no highlight, as
// SpecularTerm.
float SpecularFactor(float3 n, float3 l, float3 v, float e)
{
    if (dot(n, l) <= 0.0) {
        return 0.0;
    }
    float3 h = l + v;
    float hl = length(h);
    float nh = hl > 0.0 ? max(0.0, dot(n, h / hl)) : 0.0;
    return SpecularNormalization(e) * pow(nh, e);
}

// Specular of a light of colour `color` that reaches the pixel with `reach` (falloff, cone, visibility) from unit
// direction l. 0 for a pixel without entry (strength 0) and with specular_scale 0.
float3 SurfaceSpecular(float3 color, float reach, float3 n, float3 l, Material m, float3 v, float specularScale)
{
    float specular = m.strength * specularScale;
    if (specular <= 0.0 || reach <= 0.0) {
        return float3(0.0, 0.0, 0.0);
    }
    return color * (reach * specular * SpecularFactor(n, l, v, m.exponent));
}

// ---- Wet ground (CHI_BufferSettings::wet_stretch, wet_breakup, wet_scale) ---------------------------------------
// On flat surfaces (base normal within ~18 degrees of up) the highlight of a wet street is a long streak toward the
// viewer under every light instead of a round spot: the Blinn-Phong lobe (n.h)^e ~ exp(-e (x^2 + y^2) / 2), with x and
// y the tangents of h along screen-right and across it on the ground, is lengthened by `stretch` across (y / stretch)
// and narrowed by sqrt(stretch) along screen-right, and broken up by deterministic value noise in world space whose
// cells are long along screen-right and short along the streak (horizontal ripple bands). wet: x stretch
// (> 1 switches it on), y breakup [0, 1], z 1 / wet_scale (noise cells per world unit across the streak).
// WetSpecularFactor in src/lighting/MaterialMath.cpp is the CPU oracle of the lobe and the noise.

float WetHash(float2 p)
{
    float3 q = frac(float3(p.xyx) * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return frac((q.x + q.y) * q.z);
}

float WetNoise(float2 p)
{
    float2 i = floor(p);
    float2 f = p - i;
    float2 u = f * f * (3.0 - 2.0 * f);
    float a = WetHash(i), b = WetHash(i + float2(1.0, 0.0));
    float c = WetHash(i + float2(0.0, 1.0)), d = WetHash(i + float2(1.0, 1.0));
    return lerp(lerp(a, b, u.x), lerp(c, d, u.x), u.y);
}

// True if the wet lobe applies: wet weather switched on, a wet material entry (E48) and a flat base surface.
bool IsWet(float4 wet, float3 baseN, Material m)
{
    return wet.x > 1.0 && m.wet && baseN.z > 0.95;
}

// The specular factor of SpecularFactor with the wet lobe; `right` the screen-right ground direction (ScreenRight),
// `ground` the pixel's world xy.
float WetSpecularFactor(float3 n, float3 l, float3 v, float e, float3 right, float2 ground, float4 wet)
{
    if (dot(n, l) <= 0.0) {
        return 0.0;
    }
    float3 h = l + v;
    float hl = length(h);
    if (hl <= 0.0) {
        return 0.0;
    }
    h /= hl;
    float hn = dot(n, h);
    if (hn <= 0.0) {
        return 0.0;
    }
    float3 across = float3(-right.y, right.x, 0.0); // horizontal, its image runs up and down the screen
    float x = dot(h, right) / hn * sqrt(wet.x); // narrower across the screen
    float y = dot(h, across) / hn / wet.x;       // longer up and down the screen
    // The stretch widens the lobe's area by sqrt(stretch): divided out, so the streak is brighter nowhere in sum (E49).
    float f = SpecularNormalization(e) * rsqrt(wet.x) * exp(-0.5 * e * (x * x + y * y));
    if (wet.y > 0.0) {
        float2 q = float2(dot(ground, right.xy), dot(ground, across.xy)) * wet.z;
        // Ripples: cells long along screen-right, short along the streak, so the breakup reads as horizontal bands.
        float noise = 0.6 * WetNoise(float2(q.x * 0.2, q.y)) + 0.4 * WetNoise(float2(q.x * 0.5 + 17.0, q.y * 2.3));
        f *= lerp(1.0, 2.0 * smoothstep(0.4, 0.65, noise), wet.y);
    }
    return f;
}

// SurfaceSpecular with the wet lobe where IsWet(wet, baseN), else exactly SurfaceSpecular.
float3 SurfaceSpecularWet(float3 color, float reach, float3 n, float3 l, Material m, float3 v, float specularScale,
                          float3 baseN, float3 right, float2 ground, float4 wet)
{
    if (!IsWet(wet, baseN, m)) {
        return SurfaceSpecular(color, reach, n, l, m, v, specularScale);
    }
    float specular = m.strength * specularScale;
    if (specular <= 0.0 || reach <= 0.0) {
        return float3(0.0, 0.0, 0.0);
    }
    return color * (reach * specular * WetSpecularFactor(n, l, v, m.exponent, right, ground, wet));
}

// ---- Direction mode (material buffer, light buffer smaller than the scene) ----------------------------------------
// The light programs light with the base normal N at light buffer resolution and write, per light, (lum * L, lum)
// into a second RGBA16F target (ONE/ONE): lum is the luminance of the light that reaches the pixel without the
// Lambert term. Composite then turns the sums into a mean direction d and a coherence c and applies the detail
// normal per scene pixel (DirectionModeLight).

static const float3 kLuminance = float3(0.2126, 0.7152, 0.0722);

// Direction sample of one light reaching the pixel with `light` (colour, falloff, cone, visibility; no Lambert) from
// unit direction l (0 for a light at the pixel: it counts in lum only). Lights without a Lambert term toward the base
// normal (lambert <= 0) add nothing to the light buffer, so they do not steer the direction either. `specular` is the
// light's specular factor (E47): it scales the light's share of the direction sum but not of lum, so a light with
// factor 0 lowers the coherence by its share of the light, and the composite's highlight (direct * coherence)
// leaves out about its light. Factor 1 is the 0.3 sample.
float4 DirectionSample(float3 light, float3 l, float lambert, float specular)
{
    if (lambert <= 0.0) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    float lum = dot(light, kLuminance);
    return float4((lum * specular) * l, lum);
}

// Light of a scene pixel in direction mode: `light` from the (interpolated) light buffer, `ambient` the pure ambient,
// `sums` the direction sums (xyz sum lum * L, w sum lum), N the base normal, n and m from MaterialNormalAt.
//   direct = max(light - ambient, 0); d = normalize(sums.xyz); c = |sums.xyz| / sums.w (0 for sums.w < 1e-5)
//   k = lerp(1, min(4, max(0, n.d) / max(0.05, N.d)), c), k = 1 without a material entry; light' = ambient + direct * k
//   specular = SurfaceSpecular with light colour direct / max(0.05, N.d) * c and H from d, the lobe widened by Toksvig
//   over c: e'' = ToksvigExponent(e', c), e' the exponent after the normal-variance Toksvig of MaterialNormalAt
//   (incoherent light, a broad lobe; c = 1 keeps e')
// Ambient is never corrected. c is clamped to 1 against fp16 rounding (it cannot exceed 1 exactly).
float3 DirectionModeLight(float3 light, float3 ambient, float4 sums, float3 baseN, float3 n, Material m, float3 v,
                          float specularScale, float3 right, float2 ground, float4 wet, out float3 specular)
{
    float3 direct = max(light - ambient, 0.0);
    float len = length(sums.xyz);
    float coherence = sums.w < 1e-5 ? 0.0 : saturate(len / sums.w);
    float3 d = len > 0.0 ? sums.xyz / len : baseN;
    float baseNd = max(0.05, dot(baseN, d));
    // A pixel without a material entry is the base surface: no correction (k = 1), also where N.d < 0.05.
    float k = m.present ? lerp(1.0, min(4.0, max(0.0, dot(n, d)) / baseNd), coherence) : 1.0;
    m.exponent = ToksvigExponent(m.exponent, coherence);
    specular = SurfaceSpecularWet(direct / baseNd * coherence, 1.0, n, d, m, v, specularScale, baseN, right, ground, wet);
    return ambient + direct * k;
}

#endif
