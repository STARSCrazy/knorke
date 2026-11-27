// Origin: chiaro @ 9bc5b9b, shaders/src/bloom_prefilter.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Bloom (E28), first downsample: exposed HDR scene -> level 0 at half resolution.
// The 13-tap filter of Jimenez 2014 ("Next Generation Post Processing in Call of Duty: Advanced Warfare") read as
// its five overlapping 4x4 texel boxes (centre 1/2, corners 1/8 each). Every scene pixel passes the soft threshold
// on its own before any averaging, so a single bright pixel glows like a large one. A Karis average weights
// every box by 1 / (1 + luma) against flickering of isolated bright pixels. Level texel i covers scene pixels
// 2i and 2i + 1 exactly; the scene is read nearest with clamp to edge.
#include "common.hlsli"

Texture2D<float4> Hdr : register(t0, space2);
SamplerState HdrSampler : register(s0, space2); // nearest, clamp to edge

cbuffer PrefilterParams : register(b0, space3)
{
    float2 SourceInvSize; // 1 / scene size
    float2 DestSize;      // size of level 0
    float Exposure;       // linear multiplier: the threshold applies after the exposure
    float Threshold;      // brightest channel from which a pixel glows
    float Knee;           // absolute: threshold * knee share
    float Pad;
};

// Soft threshold with a quadratic knee on the brightest channel; the colour is scaled along.
float3 Prefilter(float3 c)
{
    c = max(c * Exposure, 0.0);
    float b = max(c.r, max(c.g, c.b));
    float soft = clamp(b - Threshold + Knee, 0.0, 2.0 * Knee);
    soft = soft * soft / (4.0 * Knee + 1e-5);
    return c * (max(soft, b - Threshold) / max(b, 1e-5));
}

float Luma(float3 c)
{
    return dot(c, float3(0.2126, 0.7152, 0.0722));
}

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    int2 base = int2(uv * DestSize) * 2 - 2;
    // The thresholded 6x6 scene pixels around the two-by-two pixels of this texel.
    float3 t[6][6];
    [unroll] for (int y = 0; y < 6; ++y) {
        [unroll] for (int x = 0; x < 6; ++x) {
            t[y][x] = Prefilter(FetchTexel(Hdr, HdrSampler, base + int2(x, y), SourceInvSize).rgb);
        }
    }
    // Box (ox, oy) averages t[oy..oy+3][ox..ox+3].
    static const int2 kBox[5] = {int2(1, 1), int2(0, 0), int2(2, 0), int2(0, 2), int2(2, 2)};
    static const float kWeight[5] = {0.5, 0.125, 0.125, 0.125, 0.125};
    float3 sum = 0.0;
    float weights = 0.0;
    [unroll] for (int i = 0; i < 5; ++i) {
        float3 box = 0.0;
        [unroll] for (int by = 0; by < 4; ++by) {
            [unroll] for (int bx = 0; bx < 4; ++bx) {
                box += t[kBox[i].y + by][kBox[i].x + bx];
            }
        }
        box *= 1.0 / 16.0;
        float w = kWeight[i] / (1.0 + Luma(box));
        sum += box * w;
        weights += w;
    }
    return float4(sum / weights, 1.0);
}
