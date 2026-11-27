// Origin: chiaro @ 9bc5b9b, shaders/src/medium_filter.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium stage, pre-filter (E106): fullscreen over the filtered scatter buffer at medium resolution, after the light
// passes. A [1 2 1] x [1 2 1] joint bilateral filter over the scatter buffer (rgb scattered light, a T) smooths the
// static sample pattern (spec addendum item 4); each tap is also weighted by exp(-guide distance) to the texel's own
// guide (MediumGuide: surface height and medium density at the scene texel the scatter texel was computed at,
// MediumSourceTexel), so it does not blur across a roof edge or a cloud's rim. The centre tap always counts (weight
// 4 / 16), so the sum never vanishes. Target 1 receives the texel's guide for the upsample in composite (bytes / 255).
// t0 scatter buffer, t1 surface buffer, t2 medium layer; a missing buffer's slot holds the scene and is not read.
#include "common.hlsli"
#include "surface.hlsli"
#include "medium_common.hlsli"

Texture2D<float4> ScatterTex : register(t0, space2);
Texture2D<float4> SurfaceTex : register(t1, space2);
Texture2D<float4> MediumTex : register(t2, space2);
SamplerState ScatterSampler : register(s0, space2); // nearest
SamplerState SurfaceSampler : register(s1, space2); // nearest
SamplerState MediumSampler : register(s2, space2);  // nearest

struct FilterOut
{
    float4 filtered : SV_Target0;
    float4 guide : SV_Target1;
};

FilterOut main(float2 uv : TEXCOORD0)
{
    float2 scatterSize = MediumSizes.zw;
    int2 texel = clamp(int2(floor(uv * scatterSize)), int2(0, 0), int2(scatterSize) - 1);
    float2 guide = MediumGuide(SurfaceTex, SurfaceSampler, MediumTex, MediumSampler, MediumSourceTexel(texel));
    int2 maxTexel = int2(scatterSize) - 1;
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    float weight = 0.0;
    [unroll] for (int k = 0; k < 9; ++k) {
        int2 o = int2(k % 3, k / 3) - 1;
        int2 tap = clamp(texel + o, int2(0, 0), maxTexel);
        float4 v = FetchTexel(ScatterTex, ScatterSampler, tap, 1.0 / scatterSize);
        float d = k == 4 ? 0.0
                         : MediumGuideDistance(guide, MediumGuide(SurfaceTex, SurfaceSampler, MediumTex, MediumSampler,
                                                                  MediumSourceTexel(tap)));
        float w = float((2 - abs(o.x)) * (2 - abs(o.y))) * exp(-d);
        sum += v * w;
        weight += w;
    }
    FilterOut r;
    r.filtered = sum / weight;
    r.guide = float4(guide / 255.0, 0.0, 0.0);
    return r;
}
