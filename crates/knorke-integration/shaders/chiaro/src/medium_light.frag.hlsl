// Origin: chiaro @ 9bc5b9b, shaders/src/medium_light.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium stage, light pass (spec 2026-11-13-medien-volumetrik, "Was am Pixel ankommt"): the light one light scatters
// toward the viewer along the view ray of a scatter buffer texel,
//   integral of T(s -> top) * sigma_s(s) * p(theta) * E(s) ds,
// E the light's irradiance without Lambert (LightIrradiance: window, cone, profile, nearest emitter point; haze factor,
// animation and flicker already in the colour), p the Henyey-Greenstein phase of each component, 1 at g = 0
// (MediumScatteringAt). N steps (MediumLightPass.x) over the part of the ray that holds medium (haze from z0 to
// z0 + 5 H, the layer's slab) and lies in the light's reach (its heights and its bounding sphere), one sample per step
// at a static, world-anchored offset (MediumWorldNoise), each step integrated energy-conserving against the analytic
// column above it (MediumColumnAt).
// Added to RGB of the scatter buffer (ONE + ONE, A = T is not written). V_i, the visibility from the light past
// obstacles, from the light's row of the polar atlas (medium_polar.hlsli) for point and spot lights with shadows;
// V = 1 for the others (shaftRow 0). T_i, the light
// path from the sample to the light's (nearest) point through the medium world map, with K = MediumLightPass.y
// samples (medium_transmit.hlsli); 1 without a world map (MediumMapSize.w = 0). The haze does not dim the light path.
// t0 surface buffer, t1 medium layer, t2 profile atlas, t3 medium world map, t4 polar atlas; a missing buffer's slot
// holds the scene and is not read. b0 MediumParams, b1 MediumMapParams, b2 PolarQueryParams.
// CHI_LIGHT_MASK (medium_light_mask.frag, masked point and spot lights): each sample times the mask (light_mask.hlsli),
// t5 the mask atlas; a window mask's light path starts at the window (its lights have no shaft row, E115).
#define CHI_PROFILE_SLOT 2
#ifdef CHI_LIGHT_MASK
#define CHI_MASK_SLOT 5
#endif
#include "common.hlsli"
#include "surface.hlsli"
#include "light_common.hlsli"
#include "medium_common.hlsli"
#define CHI_MEDIUM_MAP_SLOT 3
#define CHI_MEDIUM_MAP_PARAMS_SLOT 1
#include "medium_transmit.hlsli"
#include "medium_polar.hlsli"

Texture2D<float4> SurfaceTex : register(t0, space2);
Texture2D<float4> MediumTex : register(t1, space2);
SamplerState SurfaceSampler : register(s0, space2); // nearest
SamplerState MediumSampler : register(s1, space2);  // nearest

struct PSInput
{
    float2 scatter : TEXCOORD0;
    nointerpolation float4 positionRadius : TEXCOORD1;
    nointerpolation float4 colorFalloff : TEXCOORD2;
    nointerpolation float4 directionType : TEXCOORD3;
    nointerpolation float4 spotCos : TEXCOORD4;
    nointerpolation float4 profile : TEXCOORD5;
    nointerpolation float4 area : TEXCOORD6;
    nointerpolation float2 reach : TEXCOORD7;
    nointerpolation float shaftRow : TEXCOORD8;
#ifdef CHI_LIGHT_MASK
    nointerpolation float4 maskN : TEXCOORD9;
    nointerpolation float4 maskU : TEXCOORD10;
    nointerpolation float4 maskV : TEXCOORD11;
#endif
};

float4 main(PSInput i) : SV_Target0
{
    float2 scatterTexel = floor(i.scatter);
    int2 texel = MediumSourceTexel(int2(scatterTexel));
    float2 scene = float2(texel) + 0.5;
    float z0 = MediumSurfaceZ(SurfaceTex, SurfaceSampler, texel);
    MediumSlab slab = MediumLayerAt(MediumTex, MediumSampler, texel);

    // The part of the ray with medium.
    bool haze = MediumHaze.x > 0.0;
    bool layer = slab.density > 0.0 && slab.top > max(slab.bottom, z0);
    float lo = haze ? z0 : max(slab.bottom, z0);
    float hi = max(haze ? z0 + 5.0 * MediumHaze.y : z0, layer ? slab.top : z0);
    // ... inside the light's reach: its heights and its bounding sphere (radius plus the emitter's half extents).
    lo = max(lo, i.reach.x);
    hi = min(hi, i.reach.y);
    float3 p0 = float3(MediumGroundAt(scene, 0.0), 0.0);
    float3 d = float3(MediumGroundAt(scene, 1.0) - p0.xy, 1.0); // the ray per unit of height; |d| = 1 / w.z
    float3 oc = p0 - i.positionRadius.xyz;
    float bound = i.positionRadius.w + length(i.area.xyz) + i.area.w;
    float a = dot(d, d);
    float b = dot(d, oc);
    float disc = b * b - a * (dot(oc, oc) - bound * bound);
    if (disc <= 0.0) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    float root = sqrt(disc);
    lo = max(lo, (-b - root) / a);
    hi = min(hi, (-b + root) / a);
    if (hi <= lo) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }

    // The varying spotCos.z (the quad's half extent) is not read by the irradiance.
    LightParams l = MakeLightParams(i.positionRadius, i.colorFalloff, i.directionType, i.spotCos, i.profile, i.area);
    uint transmitSamples = uint(MediumLightPass.y);
    bool shafts = i.shaftRow > 0.5;
    uint row = shafts ? uint(i.shaftRow + 0.5) - 1u : 0u;

    int n = max(int(MediumLightPass.x), 1);
    float dh = (hi - lo) / float(n);
    float ds = dh * MediumView.w; // path length of one step: dh / w.z
    float shift = MediumWorldNoise(float3(MediumGroundAt(scene, z0), z0));
    float3 sum = float3(0.0, 0.0, 0.0);
    [loop] for (int k = 0; k < n; ++k) {
        float h = lo + (float(k) + shift) * dh;
        float3 p = p0 + h * d;
        float3 toLight, nearest;
        float2 ts;
        float3 e = l.colorFalloff.rgb * LightReachAt(p, l, toLight, nearest, ts); // LightIrradianceOf
#ifdef CHI_LIGHT_MASK
        if (any(e > 0.0)) {
            float maskT;
            e *= CHI_LIGHT_MASK_TINT(i, p - i.positionRadius.xyz, maskT);
            nearest = lerp(nearest, p, maskT);
        }
#endif
        if (shafts && any(e > 0.0)) {
            e *= PolarVisibility(row, l.positionRadius.xyz, p); // V_i(s): obstacles between light and sample
        }
        if (any(e > 0.0)) {
            e *= MediumTransmit(p, nearest, transmitSamples); // T_i(s): the medium between light and sample
        }
        float4 c = MediumScatteringAt(h, slab, toLight); // sigma_s * phase, sigma_t
        // Energy-conserving step (Hillaire 2015): the source at the sample, integrated over the step against the
        // view transmittance from the step's top, T(top) * (1 - exp(-sigma_t ds)) / sigma_t; exact for a homogeneous
        // step, ds in the limit sigma_t -> 0 (series below 1e-3, where the quotient loses precision).
        float t = MediumTransmittance(MediumColumnAt(lo + float(k + 1) * dh, slab)); // T(step top -> top)
        float x = c.a * ds;
        float w = x > 1e-3 ? (1.0 - exp(-x)) / c.a : ds * (1.0 - x * (0.5 - x / 6.0));
        sum += (t * w) * c.rgb * e;
    }
    return float4(sum, 0.0);
}
