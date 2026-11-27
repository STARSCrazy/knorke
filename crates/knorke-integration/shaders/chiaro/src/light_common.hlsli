// Origin: chiaro @ 9bc5b9b, shaders/src/light_common.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Shared parts of the light pass programs (light, light_sdf):
// the per-light data, the varyings of light.vert and the light model.
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
    float4 shadow;         // x source radius R in mask texels (-1 - R with CHI_LIGHT_INSIDE_OBSTACLE, E147), zw
                           // ground position of the light in mask texels (shadowed lights), y the specular factor
                           // of every light (E47). The varying shadow.y is the light's height instead.
    float4 profile;        // x first profile atlas row or -1 without a profile, yzw C0 unit vector (profile.hlsli)
    float4 area;           // line and rect lights: xyz half extent U along the length, w half width (rect)
    float4 areaMask;       // shadowed line and rect lights: xy U, zw V (the rect's half extent across) in mask texels
};

// Varyings from light.vert to the light fragment shaders. light.frag does not read mask, shadow and areaMask; D3D12 needs
// the inputs a fragment shader reads to be a prefix of the outputs, so they come last. light.vert appends
// SV_Position after them; the fragment shaders do not read it (D3D12 rejects
// the linkage when the pixel shader declares SV_Position with this layout).
// CHI_POINT_LIGHTS (light_point.vert, light_surface_point.vert and the shadow programs of point and spot lights): no
// area and areaMask, and every area light branch compiles away. Shadowed line and rect lights are drawn by the *_area
// shadow programs instead (E33); their varyings and branches cost the shadowed point lights ~8 % otherwise.
#if defined(CHI_LIGHT_MASK) && defined(CHI_POINT_LIGHTS)
// The `_mask` programs (masked point and spot lights, always with CHI_POINT_LIGHTS): the three mask vectors
// (light_mask.hlsli) before mask and shadow, which the unshadowed program does not read.
#define CHI_LIGHT_VARYINGS                                                       \
    float3 world : TEXCOORD0;                                                    \
    nointerpolation float4 positionRadius : TEXCOORD1;                           \
    nointerpolation float4 colorFalloff : TEXCOORD2;                             \
    nointerpolation float4 directionType : TEXCOORD3;                            \
    nointerpolation float4 spotCos : TEXCOORD4;                                  \
    nointerpolation float4 profile : TEXCOORD5;                                  \
    nointerpolation float4 maskN : TEXCOORD6;                                    \
    nointerpolation float4 maskU : TEXCOORD7;                                    \
    nointerpolation float4 maskV : TEXCOORD8;                                    \
    float2 mask : TEXCOORD9; /* the pixel's position in occlusion mask texels */ \
    nointerpolation float4 shadow : TEXCOORD10;
#elif defined(CHI_POINT_LIGHTS)
#define CHI_LIGHT_VARYINGS                                                       \
    float3 world : TEXCOORD0;                                                    \
    nointerpolation float4 positionRadius : TEXCOORD1;                           \
    nointerpolation float4 colorFalloff : TEXCOORD2;                             \
    nointerpolation float4 directionType : TEXCOORD3;                            \
    nointerpolation float4 spotCos : TEXCOORD4;                                  \
    nointerpolation float4 profile : TEXCOORD5;                                  \
    float2 mask : TEXCOORD6; /* the pixel's position in occlusion mask texels */ \
    nointerpolation float4 shadow : TEXCOORD7;
#else
#define CHI_LIGHT_VARYINGS                                                       \
    float3 world : TEXCOORD0;                                                    \
    nointerpolation float4 positionRadius : TEXCOORD1;                           \
    nointerpolation float4 colorFalloff : TEXCOORD2;                             \
    nointerpolation float4 directionType : TEXCOORD3;                            \
    nointerpolation float4 spotCos : TEXCOORD4;                                  \
    nointerpolation float4 profile : TEXCOORD5;                                  \
    nointerpolation float4 area : TEXCOORD6;                                     \
    float2 mask : TEXCOORD7; /* the pixel's position in occlusion mask texels */ \
    nointerpolation float4 shadow : TEXCOORD8;                                   \
    nointerpolation float4 areaMask : TEXCOORD9;
#endif

struct LightVaryings
{
    CHI_LIGHT_VARYINGS
};

// The per-light values the light model reads, from either set of varyings (LightVaryings, SurfaceLightVaryings).
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
#define CHI_AREA_MASK(i) float4(0.0, 0.0, 0.0, 0.0)
#else
#define CHI_AREA(i) i.area
#define CHI_AREA_MASK(i) i.areaMask
#endif

#ifdef CHI_LIGHT_MASK
// The mask factor of varyings i toward toPixel (light_mask.hlsli); t the share of the way to a window.
#define CHI_LIGHT_MASK_TINT(i, toPixel, t) LightMaskTint(toPixel, i.maskN, i.maskU, i.maskV, t)

// Window masks (t > 0): the shadow ray ends at the ray's point in the window, t of the way from the light to the
// receiver at `start` (mask texels, height startZ). target: xy mask texels, z height of the light.
float3 MaskShadowTarget(float3 target, float2 start, float startZ, float t)
{
    return t > 0.0 ? float3(lerp(target.xy, start, t), lerp(target.z, startZ, t)) : target;
}

// A window's shadow ray stops this many mask texels before the window (kAreaShadowStop, E33): the window sits flush
// in an obstacle's face. 0 for a projector.
float MaskShadowStop(float t)
{
    return t > 0.0 ? 1.5 : 0.0;
}
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

float LightReach(float3 world, LightParams l, out float3 toLight)
{
    float3 nearest;
    float2 ts;
    return LightReachAt(world, l, toLight, nearest, ts);
}

// Irradiance of light l at world point p without occlusion and without the receiver's Lambert term: colour times
// window, cone and profile (point and spot lights) or window and emitter cosine of the nearest emitter point (line
// and rect lights). Animation and flicker are already in colorFalloff (CollectLights). The ground of the plain light
// program and the scattering in the medium (medium_light.frag) both take it.
float3 LightIrradianceOf(LightParams l, float3 p)
{
    float3 toLight;
    return l.colorFalloff.rgb * LightReach(p, l, toLight);
}

float3 LightIrradiance(GpuLight l, float3 p)
{
    return LightIrradianceOf(
        MakeLightParams(l.positionRadius, l.colorFalloff, l.directionType, l.spotCos, l.profile, l.area), p);
}

// Light arriving at the ground point (z = 0) under the pixel, without occlusion.
float3 LightContribution(LightVaryings i)
{
    return LightIrradianceOf(CHI_LIGHT_PARAMS(i), i.world);
}

// Line and rect lights stop their shadow rays this many mask texels before the nearest point: an emitter usually sits
// on an obstacle's face (window, billboard, tube on a wall), and the texels right at it would shadow the light itself.
// The face can lie up to a texel beyond the emitter after rasterization, and a sample closer than half a texel to an
// obstacle is a hit, hence 1.5.
static const float kAreaShadowStop = 1.5;

// Soft near field (CHI_LightDesc::soft_near_field, point and spot lights): the light is a sphere of source_radius.
// directionType.w carries it as a fraction kNearFieldShare * min(source_radius / radius, 1) on top of the type
// (LightCulling.cpp), so the type tests (> 0.5, > 1.5, ...) stay as they are. 0 without it.
static const float kNearFieldShare = 0.49;

float NearFieldRadius(LightParams l)
{
    return IsAreaLight(l) ? 0.0 : frac(l.directionType.w) / kNearFieldShare * l.positionRadius.w;
}

// How far before its target a shadow ray of light l stops, in mask texels (0 for point and spot lights).
float ShadowStop(LightParams l)
{
    return IsAreaLight(l) ? kAreaShadowStop : 0.0;
}

// Where the shadow ray of a pixel ends: xy ground position in mask texels, z height. Point and spot lights: the light
// (shadow.zw, shadow.y); line and rect lights: their nearest point, shadow.zw + t U + s V in the mask.
float3 ShadowTarget(float4 shadow, float4 areaMask, LightParams l, float2 ts, float3 nearest)
{
    if (IsAreaLight(l)) {
        return float3(shadow.zw + ts.x * areaMask.xy + ts.y * areaMask.zw, nearest.z);
    }
    return float3(shadow.zw, shadow.y);
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

// Highest opacity of the 2x2 mask texels whose centres surround maskPos: the texels a linear field sample at
// maskPos blends, so one of them is the obstacle texel behind a field value below half a texel.
float MaskOpacityAround(Texture2D<float4> mask, SamplerState nearestSampler, float2 maskPos, float2 maskSize)
{
    float2 corner = floor(maskPos - 0.5) + 0.5;
    float o = MaskOpacityAt(mask, nearestSampler, corner, maskSize);
    o = max(o, MaskOpacityAt(mask, nearestSampler, corner + float2(1.0, 0.0), maskSize));
    o = max(o, MaskOpacityAt(mask, nearestSampler, corner + float2(0.0, 1.0), maskSize));
    return max(o, MaskOpacityAt(mask, nearestSampler, corner + float2(1.0, 1.0), maskSize));
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

// CHI_LIGHT_INSIDE_OBSTACLE (E147): a marked light carries its source radius R in shadow.x as -1 - R, so the shadow
// programs need no variant and no new data; an unmarked light (R >= 0) takes one comparison.
bool InsideObstacleLight(float4 shadow)
{
    return shadow.x < 0.0;
}

// The source radius in mask texels, for both encodings.
float LightSourceRadius(float4 shadow)
{
    return shadow.x < 0.0 ? -1.0 - shadow.x : shadow.x;
}

// A marked light's rays ignore at most this many mask texels around it.
static const int kInsideSteps = 64;

// The piece of a marked light's shadow ray that lies in the obstacle around the light: from `light` (mask texels)
// toward the receiver at `start`, one texel per step, the distance of the first sample the tracer would not count as
// a hit (field >= 0.5, or outside the mask). 0 for a light that stands free (one field sample); the whole ray if it
// ends in the obstacle; -1 if the obstacle goes on for more than kInsideSteps texels (no light gets out). Flat mode:
// every obstacle texel blocks (light_sdf.frag; with heights InsideRunHeight in shadow_trace.hlsli).
float InsideRun(Texture2D<float4> field, SamplerState fieldSampler, float2 light, float2 start, float2 maskSize)
{
    float2 d = start - light;
    float len = length(d);
    float2 dir = len > 1e-3 ? d / len : float2(0.0, 0.0);
    float2 invSize = 1.0 / maskSize;
    [loop] for (int k = 0; k <= kInsideSteps; ++k) {
        float t = min(float(k), len);
        float2 p = light + dir * t;
        if (any(p < 0.0) || any(p > maskSize) || field.SampleLevel(fieldSampler, p * invSize, 0.0).r >= 0.5) {
            return t;
        }
        if (t >= len) {
            return len;
        }
    }
    return -1.0;
}

#endif
