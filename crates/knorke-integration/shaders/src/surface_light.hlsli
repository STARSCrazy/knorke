// Origin: chiaro @ 9bc5b9b, shaders/src/surface_light.hlsli, copied below this notice for the light row programs
// (lightrow_*, ADR 0019), so they light exactly as Chiaro's light programs do. Altered: the *_medium variants march
// the medium only where the light is visible (MediumVisibility), with the same result.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// The fragment shader body shared by the surface light programs (light_surface, light_sdf_surface and their exact
// and direction variants): light one point/spot light at the surface point under the pixel. The including program
// declares SurfaceTex (and MaterialTex in exact mode), their samplers and cbuffer SurfaceParams, and defines
// CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest), the visibility of the light from the pixel (1.0 without shadows; the
// shadow programs trace toward ShadowTarget). Visibility is only evaluated where the light reaches the pixel at all.
// Line and rect lights (E33) average the Lambert term over their nearest point and their ends or corners,
// weighted by 1 / d^2 (AreaLambert), and take the highlight from the representative point (AreaSpecularPoint).
//   default:                  Lambert with the base normal
//   CHI_MATERIAL_EXACT:       diffuse with the detail normal of the material buffer, the specular highlight to
//                             SV_Target1 (ONE/ONE), both times the visibility
//   CHI_MATERIAL_DIRECTION:   diffuse with the base normal, the direction sample (lum * L, lum) of the light times the
//                             visibility to SV_Target1 (ONE/ONE) for the composite (DirectionSample)
// CHI_MEDIUM (the *_medium variants, medium_transmit.hlsli included by the program): every output also times the
// transmittance of the medium world map from the surface point to the light's (nearest) point.
#ifndef CHI_SURFACE_LIGHT_HLSLI
#define CHI_SURFACE_LIGHT_HLSLI

#include "light_common.hlsli"
#include "surface.hlsli"

// The light's colour at the pixel: times the mask in the `_mask` programs (a local of the bodies).
#ifdef CHI_LIGHT_MASK
#define CHI_LIGHT_COLOR(i) lightColor
#else
#define CHI_LIGHT_COLOR(i) i.colorFalloff.rgb
#endif

// Below this squared distance a Lambert sample counts as touching the pixel (kMinSampleDistance2 in AreaLight.cpp).
static const float kMinSampleDistance2 = 1e-10;

// Adds the Lambert term toward sample `q`, weighted by 1 / d^2, to sum.x and the weight to sum.y (AddLambert in
// src/lighting/AreaLight.cpp).
void AddLambert(float3 n, float3 world, float3 q, inout float2 sum)
{
    float3 l = q - world;
    float dd = dot(l, l);
    float w = 1.0 / max(dd, kMinSampleDistance2);
    sum += float2(w * (dd > kMinSampleDistance2 ? max(0.0, dot(n, l) * rsqrt(dd)) : 1.0), w);
}

// Lambert term of a line or rect light: the mean over its nearest point and its ends or corners, each weighted by its
// share of the irradiance 1 / d^2 (AreaLambert in src/lighting/AreaLight.cpp).
float AreaLambert(float3 n, float3 world, LightParams l, float3 nearest)
{
    float3 c = l.positionRadius.xyz;
    float3 u = l.area.xyz;
    float2 sum = float2(0.0, 0.0);
    AddLambert(n, world, nearest, sum);
    if (!IsRectLight(l)) {
        AddLambert(n, world, c + u, sum);
        AddLambert(n, world, c - u, sum);
        return sum.x / sum.y;
    }
    float3 v = RectAcross(l);
    AddLambert(n, world, c - u - v, sum);
    AddLambert(n, world, c - u + v, sum);
    AddLambert(n, world, c + u - v, sum);
    AddLambert(n, world, c + u + v, sum);
    return sum.x / sum.y;
}

// Lambert term of a point light seen as a sphere of radius r (soft near field, NearFieldLambert in
// src/lighting/LightMath.cpp): n.v / sqrt(|v|^2 + r^2) for the vector v from the pixel to the light. Far from the light
// it is the point Lambert term; closer than about r it no longer reaches 1 under the light but levels off smoothly,
// so a light hugging a wall lights it with a soft, dim spot instead of a hard bright disc.
float NearFieldLambert(float3 n, float3 v, float r)
{
    return max(0.0, dot(n, v)) * rsqrt(dot(v, v) + r * r);
}

// Receiver Lambert term of any light: toward the light for point and spot lights (SurfaceLambert), AreaLambert for
// line and rect lights.
float LightLambert(float3 n, float3 world, LightParams l, float3 toLight, float3 nearest)
{
    if (IsAreaLight(l)) {
        return AreaLambert(n, world, l, nearest);
    }
    float r = NearFieldRadius(l);
    if (r > 0.0) {
        return NearFieldLambert(n, l.positionRadius.xyz - world, r);
    }
    return SurfaceLambert(n, toLight);
}

// Representative point of a line or rect light's highlight (Karis 2013; AreaSpecularPoint in
// src/lighting/AreaLight.cpp): the segment point nearest to the reflected ray, or the ray's hit on the rect's plane
// clamped into it; `nearest` without a hit.
float3 AreaSpecularPoint(float3 world, float3 n, float3 v, LightParams l, float3 nearest)
{
    float3 r = 2.0 * dot(n, v) * n - v;
    float3 c = l.positionRadius.xyz;
    float3 u = l.area.xyz;
    if (!IsRectLight(l)) {
        float3 l0 = c - u - world;
        float3 ld = 2.0 * u;
        float rd = dot(r, ld);
        float denom = dot(ld, ld) - rd * rd;
        if (denom <= 1e-6 * dot(ld, ld)) {
            return nearest;
        }
        float t = saturate((dot(r, l0) * rd - dot(l0, ld)) / denom);
        return world + l0 + t * ld;
    }
    float3 normal = l.directionType.xyz;
    float rn = dot(r, normal);
    if (abs(rn) <= 1e-4) {
        return nearest;
    }
    float x = dot(c - world, normal) / rn;
    if (x <= 0.0) {
        return nearest;
    }
    float3 rel = world + x * r - c;
    float3 across = RectAcross(l);
    return c + AreaCoordinate(rel, u) * u + AreaCoordinate(rel, across) * across;
}

// Unit direction of the highlight: toward the light for point and spot lights, toward the representative point for
// line and rect lights (0 on the emitter).
float3 LightSpecularDirection(float3 world, float3 n, float3 v, LightParams l, float3 toLight, float3 nearest)
{
    if (!IsAreaLight(l)) {
        return toLight;
    }
    float3 q = AreaSpecularPoint(world, n, v, l, nearest) - world;
    float d = length(q);
    return d > 1e-6 ? q / d : float3(0.0, 0.0, 0.0);
}

#ifdef CHI_MEDIUM
// The visibility times the medium's transmittance, which is only marched where the light is visible (altered for the
// row programs: Chiaro marches it in the shadow too; its light_sdf.frag skips it there as here). The same result.
float3 MediumVisibility(float visibility, float3 world, float3 nearest)
{
    if (visibility <= 0.0) {
        return float3(0.0, 0.0, 0.0);
    }
    return visibility * MediumTransmit(world, nearest, kLightMediumSamples);
}
#endif

#if defined(CHI_MATERIAL_EXACT)
struct LightOutput
{
    float4 light : SV_Target0;
    float4 specular : SV_Target1;
};

LightOutput main(SurfaceLightVaryings i)
{
    float3 world;
    Surface s = SurfaceAt(SurfaceTex, SurfaceSampler, i.scene, SceneStep, world, InvAxes, OriginZAxis);
    LightOutput o;
    o.light = float4(0.0, 0.0, 0.0, 0.0);
    o.specular = float4(0.0, 0.0, 0.0, 0.0);
    LightParams l = CHI_LIGHT_PARAMS(i);
    float3 toLight, nearest;
    float2 ts;
    float reach = LightReachAt(world, l, toLight, nearest, ts);
    if (reach <= 0.0) {
        return o; // outside radius or cone: diffuse and specular are 0, the material is not needed
    }
#ifdef CHI_LIGHT_MASK
    float maskT;
    float3 lightColor = i.colorFalloff.rgb * CHI_LIGHT_MASK_TINT(i, world - i.positionRadius.xyz, maskT);
    nearest = lerp(nearest, world, maskT); // a window mask: the light comes from the window
#endif
#ifdef CHI_LIGHT_MASK
    if (all(CHI_LIGHT_COLOR(i) <= 0.0)) {
        return o; // blocked by the mask
    }
#endif
    Material m;
    float3 n = MaterialNormalAt(MaterialTex, MaterialSampler, i.scene, SceneStep, s.n, ScreenRight.xyz, XYAxes,
                                ZAxisSpec.xy, ZAxisSpec.z, m);
    float3 diffuse = CHI_LIGHT_COLOR(i) * (reach * LightLambert(n, world, l, toLight, nearest));
    float3 specular = SurfaceSpecularWet(CHI_LIGHT_COLOR(i), reach, n,
                                         LightSpecularDirection(world, n, ViewDir.xyz, l, toLight, nearest), m,
                                         ViewDir.xyz, ZAxisSpec.z * i.spotCos.z, s.n, ScreenRight.xyz, world.xy, Wet);
    if (all(diffuse <= 0.0) && all(specular <= 0.0)) {
        return o;
    }
#ifdef CHI_MEDIUM
    float3 visibility = MediumVisibility(CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest), world, nearest);
#else
    float visibility = CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest);
#endif
    o.light = float4(diffuse * visibility, 0.0);
    o.specular = float4(specular * visibility, 0.0);
    return o;
}
#elif defined(CHI_MATERIAL_DIRECTION)
struct LightOutput
{
    float4 light : SV_Target0;
    float4 direction : SV_Target1;
};

LightOutput main(SurfaceLightVaryings i)
{
    float3 world;
    Surface s = SurfaceAt(SurfaceTex, SurfaceSampler, i.scene, SceneStep, world, InvAxes, OriginZAxis);
    LightParams l = CHI_LIGHT_PARAMS(i);
    float3 toLight, nearest;
    float2 ts;
    float reach = LightReachAt(world, l, toLight, nearest, ts);
#ifdef CHI_LIGHT_MASK
    float maskT;
    float3 lightColor = i.colorFalloff.rgb * CHI_LIGHT_MASK_TINT(i, world - i.positionRadius.xyz, maskT);
    nearest = lerp(nearest, world, maskT); // a window mask: the light comes from the window
#endif
    float lambert = LightLambert(s.n, world, l, toLight, nearest);
    float3 light = CHI_LIGHT_COLOR(i) * (reach * lambert);
    LightOutput o;
    o.light = float4(0.0, 0.0, 0.0, 0.0);
    o.direction = float4(0.0, 0.0, 0.0, 0.0);
    if (all(light <= 0.0)) {
        return o;
    }
#ifdef CHI_MEDIUM
    float3 visibility = MediumVisibility(CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest), world, nearest);
    o.light = float4(light * visibility, 0.0);
    o.direction = DirectionSample(CHI_LIGHT_COLOR(i) * reach * visibility, toLight, lambert, i.spotCos.z);
#else
    float visibility = CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest);
    o.light = float4(light * visibility, 0.0);
    o.direction = DirectionSample(CHI_LIGHT_COLOR(i) * (reach * visibility), toLight, lambert, i.spotCos.z);
#endif
    return o;
}
#else
float4 main(SurfaceLightVaryings i) : SV_Target0
{
    float3 world;
    Surface s = SurfaceAt(SurfaceTex, SurfaceSampler, i.scene, SceneStep, world, InvAxes, OriginZAxis);
    LightParams l = CHI_LIGHT_PARAMS(i);
    float3 toLight, nearest;
    float2 ts;
    float reach = LightReachAt(world, l, toLight, nearest, ts);
#ifdef CHI_LIGHT_MASK
    float maskT;
    float3 lightColor = i.colorFalloff.rgb * CHI_LIGHT_MASK_TINT(i, world - i.positionRadius.xyz, maskT);
    nearest = lerp(nearest, world, maskT); // a window mask: the light comes from the window
#endif
    float3 light = CHI_LIGHT_COLOR(i) * (reach * LightLambert(s.n, world, l, toLight, nearest));
    if (all(light <= 0.0)) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
#ifdef CHI_MEDIUM
    return float4(light * MediumVisibility(CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest), world, nearest),
                  0.0);
#else
    return float4(light * CHI_SURFACE_VISIBILITY(i, s, l, ts, nearest), 0.0);
#endif
}
#endif

#endif
