// Origin: chiaro @ 9bc5b9b, shaders/src/bloom_up.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Bloom (E28), upsample: level n + 1 -> level n with a 3x3 tent filter, added onto level n (blend ONE + ONE).
// Weight fades the coarsest level in between radii that are powers of two.
#include "bloom.hlsli"

Texture2D<float4> Source : register(t0, space2);
SamplerState SourceSampler : register(s0, space2); // linear, clamp to edge

cbuffer UpParams : register(b0, space3)
{
    float2 UvScale;     // destination size / (2 * source size): destination uv -> source uv
    float2 SourceTexel; // 1 / source size
    float Weight;
    float3 Pad;
};

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    return float4(BloomTent(Source, SourceSampler, uv * UvScale, SourceTexel) * Weight, 0.0);
}
