// Origin: chiaro @ 9bc5b9b, shaders/src/light_common.hlsli, copied below this notice; altered: code that no program
// here reaches is removed. Shared by Chiaro's programs (../chiaro/src, whose blobs are Chiaro's shadercross output of
// that revision) and the light row programs (../src, ADR 0019), so these light exactly as Chiaro's did.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Shared parts of the light programs: the per-light data and the light model.
// Same formulas as src/lighting/LightMath.cpp (Attenuation, SpotFactor).
#ifndef CHI_LIGHT_COMMON_HLSLI
#define CHI_LIGHT_COMMON_HLSLI

#include "common.hlsli"
#include "profile.hlsli"
#ifdef CHI_LIGHT_MASK
#include "light_mask.hlsli"
#endif

// Matches struct GpuLight in src/rendering/GpuLight.hpp.
struct GpuLight
{
    float4 positionRadius; // xyz world position, w radius
    float4 colorFalloff;   // rgb color * intensity * flicker, a falloff exponent
    float4 directionType;  // xyz normalized direction (a rect's normal), w type (0 point, 1 spot, 2 line, 3 rect,
                           // 4 two-sided rect)
    float4 spotCos;        // x cos(inner), y cos(outer), z quad half extent = ground radius sqrt(r^2 - z^2),
                           // w 1 / profile atlas height (profile.hlsli). The varying spotCos.z is the specular
                           // factor instead (E47, set by the vertex shaders from shadow.y).
    float4 shadow;         // x the medium's polar row + 1 (medium_light.vert), 0 for the ground lights; zw ground
                           // position of the light in mask texels (shadowed lights), y the specular factor of
                           // every light (E47). The varying shadow.y is the light's height instead.
    float4 profile;        // x first profile atlas row or -1 without a profile, yzw C0 unit vector (profile.hlsli)
    float4 area;           // line and rect lights: xyz half extent U along the length, w half width (rect)
    float4 areaMask;       // shadowed line and rect lights: xy U, zw V (the rect's half extent across) in mask texels
};

// The per-light values the light model reads, from the varyings (SurfaceLightVaryings) or a GpuLight.
struct LightParams
{
    float4 positionRadius;
    float4 colorFalloff;
    float4 directionType;
    float4 spotCos;
    float4 profile;
    float4 area;
};

// The area light fields of varyings i; zero under CHI_POINT_LIGHTS.
#ifdef CHI_POINT_LIGHTS
#define CHI_AREA(i) float4(0.0, 0.0, 0.0, 0.0)
#else
#define CHI_AREA(i) i.area
#endif

#ifdef CHI_LIGHT_MASK
// The mask factor of varyings i toward toPixel (light_mask.hlsli); t the share of the way to a window.
#define CHI_LIGHT_MASK_TINT(i, toPixel, t) LightMaskTint(toPixel, i.maskN, i.maskU, i.maskV, t)

#endif

#define CHI_LIGHT_PARAMS(i) \
    MakeLightParams(i.positionRadius, i.colorFalloff, i.directionType, i.spotCos, i.profile, CHI_AREA(i))

LightParams MakeLightParams(float4 positionRadius, float4 colorFalloff, float4 directionType, float4 spotCos,
                            float4 profile, float4 area)
{
    LightParams l;
    l.positionRadius = positionRadius;
    l.colorFalloff = colorFalloff;
    l.directionType = directionType;
    l.spotCos = spotCos;
    l.profile = profile;
    l.area = area;
    return l;
}

// Line and rect lights (E33): directionType.w 2 line, 3 rect, 4 two-sided rect.
bool IsAreaLight(LightParams l)
{
#ifdef CHI_POINT_LIGHTS
    return false;
#else
    return l.directionType.w > 1.5;
#endif
}

bool IsRectLight(LightParams l)
{
    return l.directionType.w > 2.5;
}

// The point light window saturate(1 - (d / r)^2)^falloff (Attenuation in src/lighting/LightMath.cpp).
float LightWindow(float d, LightParams l)
{
    float x = d / l.positionRadius.w;
    float window = saturate(1.0 - x * x);
    return window > 0.0 ? pow(window, l.colorFalloff.w) : 0.0;
}

// A rect light's half extent across: half_width * normalize(normal x U) (RectAcross in src/lighting/AreaLight.cpp).
float3 RectAcross(LightParams l)
{
    float3 c = cross(l.directionType.xyz, l.area.xyz);
    float len = length(c);
    return len > 0.0 ? c * (l.area.w / len) : float3(0.0, 0.0, 0.0);
}

// Coordinate of p along the half extent `axis`, clamped to [-1, 1].
float AreaCoordinate(float3 p, float3 axis)
{
    float aa = dot(axis, axis);
    return aa > 0.0 ? clamp(dot(p, axis) / aa, -1.0, 1.0) : 0.0;
}

// The point of a line or rect light nearest to `world`, ts its emitter coordinates (t along U, s along V)
// (NearestEmitterPoint in src/lighting/AreaLight.cpp).
float3 NearestEmitterPoint(float3 world, LightParams l, out float2 ts)
{
    float3 rel = world - l.positionRadius.xyz;
    float3 v = IsRectLight(l) ? RectAcross(l) : float3(0.0, 0.0, 0.0);
    ts = float2(AreaCoordinate(rel, l.area.xyz), IsRectLight(l) ? AreaCoordinate(rel, v) : 0.0);
    return l.positionRadius.xyz + ts.x * l.area.xyz + ts.y * v;
}

// Reach of a line or rect light (AreaReach in src/lighting/AreaLight.cpp): the window of the distance to the nearest
// point times, for a rect, the emitter cosine (one-sided max(0, cos), two-sided |cos|).
float AreaReach(float3 world, LightParams l, out float3 toLight, out float3 nearest, out float2 ts)
{
    nearest = NearestEmitterPoint(world, l, ts);
    float3 fromEmitter = world - nearest;
    float d = length(fromEmitter);
    toLight = d > 1e-6 ? -fromEmitter / d : float3(0.0, 0.0, 0.0);
    float emit = 1.0;
    if (IsRectLight(l) && d > 1e-6) {
        float c = dot(l.directionType.xyz, fromEmitter) / d;
        emit = l.directionType.w > 3.5 ? abs(c) : max(0.0, c);
    }
    return LightWindow(d, l) * emit;
}

// Falloff times cone (and profile) of a point or spot light at `world` (E18), and the unit vector toward the light;
// a pixel at the light position gets toLight = (0, 0, 0). Same formulas as Attenuation and SpotFactor in
// src/lighting/LightMath.cpp.
float PointReach(float3 world, LightParams l, out float3 toLight)
{
    float3 toPixel = world - l.positionRadius.xyz;
    float d = length(toPixel);
    float x = d / l.positionRadius.w;
    float window = saturate(1.0 - x * x);
    float att = window > 0.0 ? pow(window, l.colorFalloff.w) : 0.0;
    float spot = 1.0;
    float c = d > 1e-6 ? dot(toPixel / d, l.directionType.xyz) : 1.0;
    if (l.directionType.w > 0.5) {
        float range = l.spotCos.x - l.spotCos.y;
        if (range > 1e-6) {
            spot = smoothstep(l.spotCos.y, l.spotCos.x, c);
        } else {
            spot = c >= l.spotCos.x ? 1.0 : 0.0;
        }
    }
    spot *= CHI_PROFILE_FACTOR(l.profile, l.directionType.xyz, l.spotCos.w, toPixel); // exactly 1 without a profile
    toLight = d > 1e-6 ? -toPixel / d : float3(0.0, 0.0, 0.0);
    return att * spot;
}

// Reach of any light at `world` without the receiver's Lambert term, the unit vector toward its nearest point, that
// point and its emitter coordinates ((0, 0) and the light position for point and spot lights).
float LightReachAt(float3 world, LightParams l, out float3 toLight, out float3 nearest, out float2 ts)
{
    if (IsAreaLight(l)) {
        return AreaReach(world, l, toLight, nearest, ts);
    }
    nearest = l.positionRadius.xyz;
    ts = float2(0.0, 0.0);
    return PointReach(world, l, toLight);
}

// Line and rect lights stop their shadow rays this many mask texels before the nearest point: an emitter usually sits
// on an obstacle's face (window, billboard, tube on a wall), and the texels right at it would shadow the light itself.
// The face can lie up to a texel beyond the emitter after rasterization, and a sample closer than half a texel to an
// obstacle is a hit, hence 1.5. Unused since Chiaro's shadow programs are gone; kept because without it DXC orders
// the SPIR-V constants of the programs that include this file differently (their blobs are checked byte for byte).
static const float kAreaShadowStop = 1.5;

// Soft near field (CHI_LightDesc::soft_near_field, point and spot lights): the light is a sphere of source_radius.
// directionType.w carries it as a fraction kNearFieldShare * min(source_radius / radius, 1) on top of the type
// (LightCulling.cpp), so the type tests (> 0.5, > 1.5, ...) stay as they are. 0 without it.
static const float kNearFieldShare = 0.49;

float NearFieldRadius(LightParams l)
{
    return IsAreaLight(l) ? 0.0 : frac(l.directionType.w) / kNearFieldShare * l.positionRadius.w;
}

// Opacity of the mask texel under maskPos (E25), 0 outside the mask domain.
float MaskOpacityAt(Texture2D<float4> mask, SamplerState nearestSampler, float2 maskPos, float2 maskSize)
{
    int2 texel = int2(floor(maskPos));
    if (any(texel < 0) || any(texel >= int2(maskSize))) {
        return 0.0;
    }
    return FetchTexel(mask, nearestSampler, texel, 1.0 / maskSize).r;
}

// Receiver rule: obstacles cast shadows but do not receive them. A pixel whose
// mask texel is an obstacle (any opacity above 0, E25) is lit as if nothing
// occluded it. Outside the mask domain is free space.
bool ReceiverInObstacle(Texture2D<float4> mask, SamplerState nearestSampler, float2 maskPos, float2 maskSize)
{
    int2 texel = int2(floor(maskPos));
    if (any(texel < 0) || any(texel >= int2(maskSize))) {
        return false;
    }
    return FetchTexel(mask, nearestSampler, texel, 1.0 / maskSize).r >= kMinObstacleOpacity;
}

// Footprint rule (CHI_ObstacleCommon::shadow_footprint): true if the mask texel under maskPos belongs to a footprint
// obstacle (G). Only asked for receivers the receiver rule would exempt, so maskPos lies inside the mask.
bool ReceiverInFootprint(Texture2D<float4> mask, SamplerState nearestSampler, float2 maskPos, float2 maskSize)
{
    int2 texel = clamp(int2(floor(maskPos)), int2(0, 0), int2(maskSize) - 1);
    return FetchTexel(mask, nearestSampler, texel, 1.0 / maskSize).g >= 0.5;
}

// The footprint rule measures at most this many texels each way.
static const int kFootprintSteps = 64;

// Visibility of a ground receiver inside a footprint obstacle, `dir` the unit direction toward the light in mask
// texels: the run through the obstacle toward the light (a) and away from it (b) are measured texel by texel; the
// receiver lies in the obstacle's shadow where a > b, i.e. beyond the footprint's middle as seen from the light. The
// edge is one texel wide (anti-aliased); the shadow lets 1 - opacity through. A receiver right under the light
// (dir 0) or in a footprint wider than the steps on both sides stays lit.
float FootprintVisibility(Texture2D<float4> mask, SamplerState nearestSampler, float2 maskPos, float2 dir,
                          float2 maskSize)
{
    if (all(dir == 0.0)) {
        return 1.0;
    }
    float a = kFootprintSteps, b = kFootprintSteps;
    [loop] for (int k = 1; k < kFootprintSteps; ++k) {
        if (a >= kFootprintSteps && MaskOpacityAt(mask, nearestSampler, maskPos + dir * k, maskSize) < kMinObstacleOpacity) {
            a = k;
        }
        if (b >= kFootprintSteps && MaskOpacityAt(mask, nearestSampler, maskPos - dir * k, maskSize) < kMinObstacleOpacity) {
            b = k;
        }
        if (a < kFootprintSteps && b < kFootprintSteps) {
            break;
        }
    }
    float o = MaskOpacityAt(mask, nearestSampler, maskPos, maskSize);
    return lerp(1.0 - o, 1.0, saturate(0.5 + 0.5 * (b - a)));
}

// Visibility of a receiver the receiver rule exempts: 1, or the footprint rule toward `target` (mask texels).
float ExemptVisibility(Texture2D<float4> mask, SamplerState nearestSampler, float2 maskPos, float2 target,
                       float2 maskSize)
{
    if (!ReceiverInFootprint(mask, nearestSampler, maskPos, maskSize)) {
        return 1.0;
    }
    float2 d = target - maskPos;
    float len = length(d);
    return FootprintVisibility(mask, nearestSampler, maskPos, len > 1e-3 ? d / len : float2(0.0, 0.0), maskSize);
}

#endif
