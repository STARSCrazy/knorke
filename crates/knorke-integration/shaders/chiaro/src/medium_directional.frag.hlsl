// Origin: chiaro @ 9bc5b9b, shaders/src/medium_directional.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium stage, directional lights (spec 2026-11-13-medien-volumetrik, "Was am Pixel ankommt"): the light one
// directional light scatters toward the viewer along the view ray of a scatter buffer texel, one fullscreen draw per
// light, added to RGB of the scatter buffer (A = T is not written):
//   E * integral over the column of V(s) * T_sun(s) * T(s -> top) * sigma_s(s) * p(s) ds
// E the light's colour * intensity * haze factor (no Lambert), p the Henyey-Greenstein phase of each component (times
// 4 pi, 1 for g = 0; medium_common.hlsli). The sun's direction and the view direction are the same for every point of
// the column, so cos theta and with it each component's phase is constant along it.
//
// Closed form (Hillaire 2015, energy-conserving): along the column dT(z -> top)/dz = sigma(z) T / w.z, so where the
// sun reaches every point (V = 1, T_sun = 1) the light scattered from [z1, z2] is
//   albedo * p * (T(z2 -> top) - T(z1 -> top)),
// exact for the haze alone and for the layer alone (albedo * p constant per component); inside the layer's slab with
// haze albedo * p is the sigma-weighted mix of both at the piece's middle (MediumScatteringAt). The column is cut at the
// slab's bottom and top.
// Where the sun is shaded or dimmed the column is cut into N pieces (MediumLightPass.x), in two parts:
//   [lo, highest finite obstacle top] (with shadows): V and T_sun;
//   [highest obstacle top, highest medium top] (with a world map): T_sun only.
// When both exist each gets pieces by its share of the light scattered below the highest top (its step of T), at least
// one each; with N = 1 one piece covers both. A high cloud anywhere in the frame thus takes pieces from the shadow part
// only in proportion to the light scattered above the obstacles' top in this column. In each part the
// piece boundaries lie at equal steps of the column's transmittance T(z -> top), so every piece carries an equal share
// of the light scattered in that part: the pieces gather where the medium is (near the ground in haze), not at equal
// heights. The boundaries come from inverting T (MediumColumnHeightAt); their precision does not affect correctness,
// every piece's integral stays in closed form. Per piece:
//   V: the part above the sun shadow height z* (medium_sun.hlsli) of the point's ground point; s - z*(ground(s)) is
//      taken linear between the piece's ends, so the lit part [l1, l2] ends where it crosses 0 (exact top-down, where
//      the ground point does not move along the ray);
//   T_sun: the medium world map toward the sun (K = MediumLightPass.y, medium_transmit.hlsli) at the height where
//      T(z -> top) = (T(l1) + T(l2)) / 2, the middle of the lit part's scattered light.
// Above, the closed form again up to the top of the column (T = 1).
// t0 surface buffer, t1 medium layer, t2 medium world map, t3 sun shadow height atlas; a missing buffer's slot holds
// the scene and is not read. b0 MediumParams, b1 MediumMapParams, b2 SunParams.
#include "common.hlsli"
#include "surface.hlsli"
#include "medium_common.hlsli"
#define CHI_MEDIUM_MAP_SLOT 2
#define CHI_MEDIUM_MAP_PARAMS_SLOT 1
#include "medium_transmit.hlsli"
#define CHI_SUN_HEIGHT_SLOT 3
#define CHI_SUN_PARAMS_SLOT 2
#include "medium_sun.hlsli"

Texture2D<float4> SurfaceTex : register(t0, space2);
Texture2D<float4> MediumTex : register(t1, space2);
SamplerState SurfaceSampler : register(s0, space2); // nearest
SamplerState MediumSampler : register(s1, space2);  // nearest

static const float kColumnTop = 3.4e38; // above every medium: T = 1 (taken as such, exp() of huge arguments is
                                        // unspecified on some drivers)

// Light scattered toward the viewer from [za, zb] of the column with the sun unshaded and undimmed, for a piece that
// does not cross the slab's bottom or top: albedo * p * (T(zb -> top) - T(za -> top)).
float3 ScatterPiece(float za, float zb, MediumSlab m)
{
    if (zb <= za) {
        return float3(0.0, 0.0, 0.0);
    }
    float4 c = MediumScatteringAt(za + 0.5 * min(zb - za, MediumHaze.y), m, SunToLight.xyz); // sigma_s * p, sigma_t
    if (c.a <= 0.0) {
        return float3(0.0, 0.0, 0.0);
    }
    float ta = MediumTransmittance(MediumColumnAt(za, m));
    float tb = zb >= kColumnTop ? 1.0 : MediumTransmittance(MediumColumnAt(zb, m));
    return c.rgb / c.a * (tb - ta);
}

// The same for any [za, zb]: cut at the slab's bottom and top.
float3 ColumnScatter(float za, float zb, MediumSlab m)
{
    if (zb <= za) {
        return float3(0.0, 0.0, 0.0);
    }
    float e0 = clamp(m.bottom, za, zb);
    float e1 = max(clamp(m.top, za, zb), e0);
    return ScatterPiece(za, e0, m) + ScatterPiece(e0, e1, m) + ScatterPiece(e1, zb, m);
}

// T(z -> top) of the column at height z.
float ColumnT(float z, MediumSlab m)
{
    return MediumTransmittance(MediumColumnAt(z, m));
}

// The height in [za, zb] where the column's optical depth above it is tau (tau(za) >= tau >= tau(zb)). The optical
// depth is the layer's slab (k * (top - z) inside it, constant below, 0 above) plus the haze c * exp(-z / H). Below
// and above the slab only the haze varies and is inverted exactly; inside it Newton's method from the slab's bottom
// approaches the root from below and converges monotonically (the function is convex and falls there).
float ColumnHeightAtTau(float tau, float za, float zb, MediumSlab m)
{
    float k = m.density * MediumTable[m.kind].a * MediumView.w; // the layer's optical depth per unit of height
    float c = MediumHaze.x * MediumHaze.y * MediumView.w;       // the haze's optical depth above height 0
    float H = MediumHaze.y;
    bool slab = k > 0.0 && m.top > m.bottom;
    float e0 = slab ? clamp(m.bottom, za, zb) : zb;
    float e1 = slab ? max(clamp(m.top, za, zb), e0) : zb;
    float z;
    if (tau >= MediumColumnAt(e0, m).tauLayer + c * exp(-e0 / H)) {
        // [za, e0]: below the slab its part is constant.
        float h = tau - (slab ? k * max(m.top - max(m.bottom, za), 0.0) : 0.0);
        z = (c > 0.0 && h > 0.0) ? -H * log(h / c) : za;
        z = clamp(z, za, e0);
    } else if (tau >= MediumColumnAt(e1, m).tauLayer + c * exp(-e1 / H)) {
        // [e0, e1]: f(z) = k (top - z) + c exp(-z / H) - tau, convex and falling, f(e0) >= 0. Newton starts at the
        // larger of the roots of the linear term alone and of the haze term alone (each lies at or below the root,
        // the haze's only while inside the slab), so a few steps suffice whichever term dominates.
        float zLin = m.top - tau / k;
        float zHaze = (c > 0.0 && tau > 0.0) ? -H * log(tau / c) : e0;
        z = max(e0, max(zLin, zHaze <= e1 ? zHaze : e0));
        [unroll] for (int i = 0; i < 4; ++i) {
            float hz = c * exp(-z / H);
            float f = k * (m.top - z) + hz - tau;
            z += f / (k + hz / H);
        }
        z = clamp(z, e0, e1);
    } else {
        // [e1, zb]: above the slab only the haze.
        z = (c > 0.0 && tau > 0.0) ? -H * log(tau / c) : zb;
        z = clamp(z, e1, zb);
    }
    return z;
}

// The height in [za, zb] where T(z -> top) = t.
float ColumnHeightAt(float t, float za, float zb, MediumSlab m)
{
    return ColumnHeightAtTau(-log(max(t, 1e-30)), za, zb, m);
}

// Pieces whose lit part's step of T is below this share of their part's do not fetch T_sun (it is taken as 1).
static const float kMinSunShare = 1e-3;

// The light scattered from [a, b] (b finite, ta = T(a -> top), tb = T(b -> top)), cut into n pieces at equal steps of
// T(z -> top) (see the top). With `shadows` only each piece's part above the sun shadow height (atlas block `row`),
// with `map` dimmed by T_sun.
float3 ScatterPieces(float2 scene, float a, float b, float ta, float tb, int n, bool shadows, uint row, bool map,
                     MediumSlab slab)
{
    uint transmitSamples = uint(MediumLightPass.y);
    float dt = (tb - ta) / float(n);
    float minSun = kMinSunShare * (tb - ta);
    // Where T does not change nothing scatters; equal heights keep the pieces defined.
    bool even = dt <= 1e-7;
    float dz = (b - a) / float(n);
    float3 sum = float3(0.0, 0.0, 0.0);
    float z1 = a;
    // f(s) = s - z*(ground(s)); lit where f >= 0.
    float f1 = shadows ? a - SunShadowHeight(row, MediumGroundAt(scene, a)) : 1.0;
    [loop] for (int k = 0; k < n; ++k) {
        float z2 = k == n - 1 ? b : even ? a + float(k + 1) * dz : ColumnHeightAt(ta + float(k + 1) * dt, z1, b, slab);
        float f2 = shadows ? z2 - SunShadowHeight(row, MediumGroundAt(scene, z2)) : 1.0;
        float l1 = z1, l2 = z2;
        if (f1 < 0.0 && f2 < 0.0) {
            l2 = z1; // shaded
        } else if (f1 < 0.0) {
            l1 = z1 + (z2 - z1) * (-f1) / (f2 - f1);
        } else if (f2 < 0.0) {
            l2 = z1 + (z2 - z1) * f1 / (f1 - f2);
        }
        z1 = z2;
        f1 = f2;
        if (l2 <= l1) {
            continue;
        }
        float3 scatter = ColumnScatter(l1, l2, slab);
        if (map) {
            // The lit part's step of T, not its scattered light: that carries the albedo, and in a dark medium every
            // piece would fall below the threshold.
            float t1 = ColumnT(l1, slab), t2 = ColumnT(l2, slab);
            if (t2 - t1 > minSun) {
                float zm = t2 - t1 > 1e-7 ? ColumnHeightAt(0.5 * (t1 + t2), l1, l2, slab) : 0.5 * (l1 + l2);
                scatter *= MediumTransmitToward(float3(MediumGroundAt(scene, zm), zm), SunToLight.xyz, transmitSamples);
            }
        }
        sum += scatter;
    }
    return sum;
}

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    int2 texel = MediumSourceTexel(int2(floor(uv * MediumSizes.zw)));
    float2 scene = float2(texel) + 0.5;
    float z0 = MediumSurfaceZ(SurfaceTex, SurfaceSampler, texel);
    MediumSlab slab = MediumLayerAt(MediumTex, MediumSampler, texel);
    bool haze = MediumHaze.x > 0.0;
    bool layer = slab.density > 0.0 && slab.top > max(slab.bottom, z0);
    if (!haze && !layer) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    // The part of the column with medium: from the visible point (or the slab's bottom) up.
    float lo = haze ? z0 : max(slab.bottom, z0);
    float mediumTop = haze ? kColumnTop : slab.top;
    // Above the highest finite obstacle top no obstacle shades the sun; above the highest medium top no medium dims
    // it.
    bool shadows = SunColor.w > 0.5;
    bool map = MediumMapSize.w > 0.0;
    float shadowTop = shadows ? SunToLight.w : 0.0;
    float hi = min(max(shadowTop, map ? MediumMapTop() : 0.0), mediumTop);
    float3 sum = float3(0.0, 0.0, 0.0);
    if (hi > lo) {
        float split = clamp(shadowTop, lo, hi); // V and T_sun below, T_sun only above
        uint row = shadows ? uint(SunColor.w + 0.5) - 1u : 0u;
        int n = max(int(MediumLightPass.x), 1);
        float tLo = ColumnT(lo, slab), tHi = ColumnT(hi, slab);
        bool lower = split > lo, upper = hi > split; // above the obstacles' top only the map dims the sun
        // One part, or one piece for both (the shadow height test is harmless above the obstacles' top): [lo, hi]
        // with n pieces. Both parts: the pieces go by each part's share of the scattered light (its step of T), at
        // least one each, so a cloud high above the obstacles takes pieces from the shadow part only by the light it
        // scatters. The parts run in one loop around a single ScatterPieces, so the compiler inlines it once.
        bool both = lower && upper && n > 1;
        float tSplit = both ? ColumnT(split, slab) : tHi;
        int nLower = both ? clamp(int(round(float(n) * (tSplit - tLo) / max(tHi - tLo, 1e-30))), 1, n - 1) : n;
        [loop] for (int part = 0; part < (both ? 2 : 1); ++part) {
            bool first = part == 0;
            sum += ScatterPieces(scene, first ? lo : split, first ? (both ? split : hi) : hi, first ? tLo : tSplit,
                                 first ? tSplit : tHi, first ? nLower : n - nLower, first && shadows && lower, row,
                                 map, slab);
        }
    }
    sum += ColumnScatter(max(lo, hi), kColumnTop, slab);
    return float4(SunColor.rgb * sum, 0.0);
}
