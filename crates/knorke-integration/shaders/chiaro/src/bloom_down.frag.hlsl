// Origin: chiaro @ 9bc5b9b, shaders/src/bloom_down.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Bloom (E28), further downsamples: level n -> level n + 1 with the 13-tap filter of Jimenez 2014 (bilinear
// taps, weights 1/8 for the centre taps and (0, 0), 1/16 for the edge and 1/32 for the corner taps). Texel i of
// the destination is centred on the corner between source texels 2i and 2i + 1.
Texture2D<float4> Source : register(t0, space2);
SamplerState SourceSampler : register(s0, space2); // linear, clamp to edge

cbuffer DownParams : register(b0, space3)
{
    float2 UvScale;     // 2 * destination size / source size: destination uv -> source uv
    float2 SourceTexel; // 1 / source size
};

float3 Tap(float2 uv, float x, float y)
{
    return Source.SampleLevel(SourceSampler, uv + float2(x, y) * SourceTexel, 0.0).rgb;
}

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float2 c = uv * UvScale;
    float3 inner = Tap(c, -1, -1) + Tap(c, 1, -1) + Tap(c, -1, 1) + Tap(c, 1, 1);
    float3 corners = Tap(c, -2, -2) + Tap(c, 2, -2) + Tap(c, -2, 2) + Tap(c, 2, 2);
    float3 edges = Tap(c, 0, -2) + Tap(c, -2, 0) + Tap(c, 2, 0) + Tap(c, 0, 2);
    float3 centre = Tap(c, 0, 0);
    return float4(inner * 0.125 + corners * 0.03125 + edges * 0.0625 + centre * 0.125, 1.0);
}
