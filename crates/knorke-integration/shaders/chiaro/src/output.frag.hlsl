// Origin: chiaro @ 9bc5b9b, shaders/src/output.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Output stage: HDR buffer -> target.
// One pass over the whole target. Pixels inside SceneRect sample the scene,
// the rest gets the letterbox color. The optional overlay (premultiplied,
// covering the whole target) is blended on top, in sRGB-encoded space for
// 8-bit UNORM overlays and in linear space for *_SRGB/float overlays. Targets
// that expect linear values get a final decode. Tonemapping (E27) acts on the
// exposed HDR scene only, before the sRGB encode: the letterbox and the overlay
// are never tonemapped. NONE is the plain clamp. Bloom (E28) is added to the
// exposed scene before tonemapping: a tent filter over bloom level 0, read per
// scene pixel when the scene is upscaled nearest. Colour grading and the vignette
// (E110) follow the tonemap: tint and saturation on linear values, contrast and
// the vignette on sRGB-encoded display values. Each runs only with its flag, so
// neutral settings execute exactly the code of before (bit-identical). The dither (E114) adds an ordered 8x8 Bayer
// offset of +-0.5 LSB to the sRGB-encoded scene value, after the vignette and before the overlay, only with its flag
// (the CPU sets it for 8-bit targets and CHI_OUTPUT_DITHER only). CRT (E137): the bend warps the scene coordinate
// before anything reads the scene (outside the rounded screen the letterbox, one pixel of antialiasing), scanlines and
// the aperture grille multiply linear values with the brightness after the vignette and before the dither; with
// CHI_CRT_OVERLAY the overlay is read at the warped coordinate. Each step only with its flag (bit-identical when off).
// Shockwaves (E142): after the CRT bend, the scene position is moved back by the radial offset of every wave before
// the scene and bloom are read; exposure to dither and the overlay keep the undistorted position. With nearest
// upscaling the offset is taken at the scene pixel centre so whole scene pixels move. Only with its flag.
#include "bloom.hlsli"
#include "common.hlsli"

Texture2D<float4> Hdr : register(t0, space2);
Texture2D<float4> Overlay : register(t1, space2);
Texture2D<float4> Bloom : register(t2, space2);
SamplerState HdrSampler : register(s0, space2);
SamplerState OverlaySampler : register(s1, space2);
SamplerState BloomSampler : register(s2, space2); // linear

cbuffer OutputParams : register(b0, space3)
{
    float4 SceneRect;    // x, y, w, h in target pixels
    float2 TargetSize;   // target width, height in pixels
    float ExposureScale; // linear multiplier
    uint Flags;          // bit 0: encode sRGB, bit 1: overlay present, bit 2: overlay samples linear,
                         // bit 3: bloom present, bit 4: bloom per scene pixel (nearest upscaling)
    float4 Letterbox;    // rgb: sRGB-encoded letterbox color
    uint TonemapMode;    // CHI_Tonemap
    float TonemapKnee;   // CHI_TONEMAP_SOFT
    float BloomScale;    // intensity / sum of the level weights
    float Pad;
    float4 BloomUv;      // xy: scene size / (2 * level 0 size), zw: 1 / level 0 size
    float2 SceneSize;    // pixels
    float2 Pad2;
    float4 TintSaturation; // rgb tint (linear), a saturation
    float4 Vignette;       // x strength, y radius, z softness (half shorter scene sides), w contrast
    float4 VignetteColor;  // rgb sRGB-encoded vignette colour, a 1 / (half the shorter scene side) in pixels
    float4 CrtWarp;        // x k, y corner radius, zw aspect of the scene rect (sides / shorter side)
    float4 CrtBeam;        // x scanline strength (faded), y scanline period, z mask strength, w mask pitch
    float4 CrtExtra;       // x brightness, y target pixels per half shorter rect side
    float4 ShockAxes;      // x_axis.xy, y_axis.xy of the projection (world ground -> scene pixels)
    float4 ShockInvAxes;   // rows of the inverse of [x_axis y_axis]
    float2 ShockOrigin;    // projection origin, scene pixels
    uint ShockCount;       // waves in ShockRing / ShockStrength
    float ShockPad;
    float4 ShockRing[16];     // centre x, y (world), radius, 2 / width; GpuShockwave in src/core/Shockwave.hpp
    float4 ShockStrength[16]; // x strength (world units)
};

static const uint FLAG_ENCODE_SRGB = 1u;
static const uint FLAG_HAS_OVERLAY = 2u;
static const uint FLAG_OVERLAY_LINEAR = 4u;
static const uint FLAG_BLOOM = 8u;
static const uint FLAG_BLOOM_SNAP = 16u;
static const uint FLAG_GRADE = 32u;        // tint and saturation
static const uint FLAG_CONTRAST = 64u;
static const uint FLAG_VIGNETTE = 128u;
static const uint FLAG_SCENE_SNAP = 256u;  // vignette per scene pixel (nearest upscaling)
static const uint FLAG_DITHER = 512u;      // ordered dither of the scene, +-0.5 LSB of an 8-bit target
static const uint FLAG_CRT_WARP = 1024u;   // bend and rounded corners
static const uint FLAG_CRT_SCANLINES = 2048u;
static const uint FLAG_CRT_MASK = 4096u;   // aperture grille
static const uint FLAG_CRT_BEAM = 8192u;   // linear multiply of scanlines, mask, brightness
static const uint FLAG_CRT_OVERLAY = 16384u;
static const uint FLAG_SHOCKWAVE = 32768u;      // shockwave displacement (E142)
static const uint FLAG_SHOCKWAVE_SNAP = 65536u; // per scene pixel (nearest upscaling)
static const float3 kLuma = float3(0.2126, 0.7152, 0.0722); // Rec. 709, as kLumaWeights in ColorMath.hpp
static const uint TONEMAP_SOFT = 1u;
static const uint TONEMAP_ACES = 2u;

// Identity up to the knee on the brightest channel, above it a soft shoulder towards 1; the colour is scaled
// along to keep its hue and, above 4, mixed towards grey by up to one half so very bright light turns white.
// Same as TonemapSoft in src/core/ColorMath.hpp.
float3 TonemapSoft(float3 rgb, float knee)
{
    float m = max(rgb.r, max(rgb.g, rgb.b));
    if (m <= knee) {
        return rgb;
    }
    float f = knee + (1.0 - knee) * (1.0 - exp(-(m - knee) / (1.0 - knee)));
    float3 scaled = rgb * (f / m);
    return lerp(scaled, float3(f, f, f), saturate((m - 4.0) / 12.0) * 0.5);
}

// Narkowicz 2015 ACES fit per channel. Same as TonemapAces in src/core/ColorMath.hpp.
float3 TonemapAces(float3 x)
{
    return saturate((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14));
}

// Ordered 8x8 Bayer threshold of target pixel p, in [-0.5, 0.5) with the mean 0 (E114). The index interleaves the
// bits of p.y and p.x ^ p.y, least significant coordinate bit first. Same as DitherThreshold in ColorMath.hpp.
float DitherThreshold(uint2 p)
{
    uint x = p.x & 7u;
    uint y = p.y & 7u;
    uint xy = x ^ y;
    uint index = ((y & 1u) << 5) | ((xy & 1u) << 4) | ((y & 2u) << 2) | ((xy & 2u) << 1) | ((y & 4u) >> 1) |
                 ((xy & 4u) >> 2);
    return (float(index) + 0.5) / 64.0 - 0.5;
}

// Barrel warp of the scene coordinate (E137). Same as CrtWarp in src/core/CrtMath.hpp.
float2 CrtWarpLocal(float2 local)
{
    float2 c = local * 2.0 - 1.0;
    float2 a = CrtWarp.zw;
    float r2 = dot(c * c, a * a) / dot(a, a);
    return c * (1.0 + CrtWarp.x * r2) * 0.5 + 0.5;
}

// Coverage of the rounded screen at the warped coordinate. Same as CrtCoverage in src/core/CrtMath.hpp.
float CrtCover(float2 warped)
{
    float2 a = CrtWarp.zw;
    float2 q = abs((warped * 2.0 - 1.0) * a) - (a - CrtWarp.y);
    float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - CrtWarp.y;
    return saturate(0.5 - d * CrtExtra.y);
}

// Scene pixel offset of all shockwaves at the scene position q: the ground point of q, then per wave the band profile
// (1 + cos(pi s)) / 2 times the strength along the world direction away from the centre, mapped by the projection.
// Same as ShockwaveSceneOffset in src/core/Shockwave.cpp.
float2 ShockwaveOffset(float2 q)
{
    float2 v0 = q - ShockOrigin;
    float2 g = float2(dot(ShockInvAxes.xy, v0), dot(ShockInvAxes.zw, v0));
    float2 total = float2(0.0, 0.0);
    for (uint i = 0u; i < ShockCount; ++i) {
        float2 v = g - ShockRing[i].xy;
        float dist = length(v);
        float s = (dist - ShockRing[i].z) * ShockRing[i].w;
        if (abs(s) < 1.0 && dist > 0.0) {
            float d = ShockStrength[i].x * (0.5 + 0.5 * cos(3.14159265 * s)) / dist;
            total += v * d;
        }
    }
    return total.x * ShockAxes.xy + total.y * ShockAxes.zw;
}

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float2 pixel = uv * TargetSize;
    float2 local = (pixel - SceneRect.xy) / SceneRect.zw;

    float3 rgb = Letterbox.rgb;
    bool inside = all(local >= 0.0) && all(local < 1.0);
    float cover = 1.0;
    if ((Flags & FLAG_CRT_WARP) != 0u) {
        local = CrtWarpLocal(local);
        cover = CrtCover(local);
        inside = cover > 0.0;
        local = clamp(local, 0.0, 0.99999);
    }
    if (inside) {
        float2 src = local; // where the scene is read: moved by the shockwaves
        if ((Flags & FLAG_SHOCKWAVE) != 0u) {
            float2 q = local * SceneSize;
            if ((Flags & FLAG_SHOCKWAVE_SNAP) != 0u) {
                q = floor(q) + 0.5;
            }
            float2 o = ShockwaveOffset(q);
            if (any(o != 0.0)) {
                src = clamp((q - o) / SceneSize, 0.0, 1.0);
            }
        }
        float3 hdr = Hdr.SampleLevel(HdrSampler, src, 0.0).rgb * ExposureScale;
        if ((Flags & FLAG_BLOOM) != 0u) {
            float2 at = src;
            if ((Flags & FLAG_BLOOM_SNAP) != 0u) {
                at = (floor(src * SceneSize) + 0.5) / SceneSize;
            }
            hdr += BloomTent(Bloom, BloomSampler, at * BloomUv.xy, BloomUv.zw) * BloomScale;
        }
        if (TonemapMode == TONEMAP_SOFT) {
            hdr = TonemapSoft(max(hdr, 0.0), TonemapKnee);
        } else if (TonemapMode == TONEMAP_ACES) {
            hdr = TonemapAces(max(hdr, 0.0));
        }
        float3 lin = saturate(hdr);
        if ((Flags & FLAG_GRADE) != 0u) {
            // Same as GradeLinear in src/core/ColorMath.hpp.
            lin = saturate(lin * TintSaturation.rgb);
            float y = dot(lin, kLuma);
            lin = saturate(y + (lin - y) * TintSaturation.a);
        }
        rgb = LinearToSrgb(lin);
        if ((Flags & FLAG_CONTRAST) != 0u) {
            rgb = saturate((rgb - 0.5) * Vignette.w + 0.5); // GradeContrast
        }
        if ((Flags & FLAG_VIGNETTE) != 0u) {
            // VignetteWeight: distance from the scene centre in half shorter scene sides.
            float2 at = local * SceneSize;
            if ((Flags & FLAG_SCENE_SNAP) != 0u) {
                at = floor(at) + 0.5;
            }
            float d = length(at - 0.5 * SceneSize) * VignetteColor.a;
            float t = Vignette.z > 0.0 ? saturate((d - Vignette.y) / Vignette.z) : (d >= Vignette.y ? 1.0 : 0.0);
            rgb = lerp(rgb, VignetteColor.rgb, Vignette.x * t * t * (3.0 - 2.0 * t));
        }
        if ((Flags & FLAG_CRT_BEAM) != 0u) {
            // Scanlines and mask on linear values (beam intensity). CrtScanlineWeight, CrtMaskWeight, CrtBeam.
            float3 w = float3(1.0, 1.0, 1.0);
            if ((Flags & FLAG_CRT_SCANLINES) != 0u) {
                float t = frac(local.y * SceneSize.y / CrtBeam.y);
                w *= 1.0 - CrtBeam.x * (0.5 - 0.5 * cos(6.28318530718 * (t - 0.25)));
            }
            if ((Flags & FLAG_CRT_MASK) != 0u) {
                float third = min(2.0, floor(frac((floor(pixel.x) + 0.5) / CrtBeam.w) * 3.0));
                float3 m = float3(third == 0.0 ? 1.0 : 0.0, third == 1.0 ? 1.0 : 0.0, third == 2.0 ? 1.0 : 0.0);
                w *= lerp(1.0 - CrtBeam.z, 1.0, m);
            }
            rgb = LinearToSrgb(saturate(SrgbToLinear(rgb) * w * CrtExtra.x));
        }
        if ((Flags & FLAG_CRT_WARP) != 0u) {
            rgb = lerp(Letterbox.rgb, rgb, cover);
        }
        if ((Flags & FLAG_DITHER) != 0u) {
            rgb = saturate(rgb + DitherThreshold(uint2(pixel)) / 255.0);
        }
    }

    if ((Flags & FLAG_HAS_OVERLAY) != 0u) {
        float2 overlayUv = uv;
        float overlayCover = 1.0;
        if ((Flags & FLAG_CRT_OVERLAY) != 0u) {
            overlayUv = (SceneRect.xy + local * SceneRect.zw) / TargetSize;
            overlayCover = inside ? cover : 0.0;
        }
        float4 overlay = Overlay.SampleLevel(OverlaySampler, overlayUv, 0.0) * overlayCover;
        if ((Flags & FLAG_OVERLAY_LINEAR) != 0u) {
            // *_SRGB or float overlay: its premultiplied values are linear, so blend in linear space.
            rgb = LinearToSrgb(saturate(overlay.rgb + SrgbToLinear(rgb) * (1.0 - overlay.a)));
        } else {
            rgb = overlay.rgb + rgb * (1.0 - overlay.a);
        }
    }

    if ((Flags & FLAG_ENCODE_SRGB) == 0u) {
        rgb = SrgbToLinear(rgb);
    }
    return float4(rgb, 1.0);
}
