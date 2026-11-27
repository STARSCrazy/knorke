// Origin: chiaro @ 9bc5b9b, shaders/src/bloom.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Bloom helpers (E28), shared by the upsample and the output stage.
#ifndef CHI_BLOOM_HLSLI
#define CHI_BLOOM_HLSLI

// 3x3 tent (1 2 1 / 2 4 2 / 1 2 1, divided by 16) of bilinear taps one texel apart.
float3 BloomTent(Texture2D<float4> t, SamplerState s, float2 uv, float2 texel)
{
    float3 c = t.SampleLevel(s, uv, 0.0).rgb * 4.0;
    c += (t.SampleLevel(s, uv + float2(-texel.x, 0.0), 0.0).rgb + t.SampleLevel(s, uv + float2(texel.x, 0.0), 0.0).rgb +
          t.SampleLevel(s, uv + float2(0.0, -texel.y), 0.0).rgb + t.SampleLevel(s, uv + float2(0.0, texel.y), 0.0).rgb) *
         2.0;
    c += t.SampleLevel(s, uv - texel, 0.0).rgb + t.SampleLevel(s, uv + texel, 0.0).rgb +
         t.SampleLevel(s, uv + float2(texel.x, -texel.y), 0.0).rgb + t.SampleLevel(s, uv + float2(-texel.x, texel.y), 0.0).rgb;
    return c * (1.0 / 16.0);
}

#endif
