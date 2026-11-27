// Origin: chiaro @ 9bc5b9b, shaders/src/normal_codec.hlsli, copied unchanged below this notice.
// Shared by Chiaro's programs (../chiaro/src, whose blobs are Chiaro's shadercross output of that revision) and
// the light row programs (../src, ADR 0019), so these light exactly as Chiaro's did.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Shared normal codec: a unit normal of the upper hemisphere (z >= 0) in two bytes.
// Mirrors chiaro::EncodeNormal / chiaro::DecodeNormal in include/Chiaro/Chiaro.hpp.
//   encode: n = normalize(n); n.z = max(n.z, 0); p = n.xy / (|n.x| + |n.y| + n.z)
//           q = (p.x + p.y, p.x - p.y); byte = clamp(round(q * 127) + 128, 1, 255)
//   decode: q = clamp((byte - 128) / 127, -1, 1); p = ((q.x + q.y) / 2, (q.x - q.y) / 2)
//           n = normalize(p.x, p.y, 1 - |p.x| - |p.y|)
// Byte (128, 128) is exactly (0, 0, 1).
#ifndef CHI_NORMAL_CODEC_HLSLI
#define CHI_NORMAL_CODEC_HLSLI

// xy01: the two bytes as sampled from a UNORM texture.
float3 DecodeNormal(float2 xy01)
{
    float2 bytes = round(saturate(xy01) * 255.0);
    float2 q = clamp((bytes - 128.0) / 127.0, -1.0, 1.0);
    float2 p = float2(q.x + q.y, q.x - q.y) * 0.5;
    return normalize(float3(p.x, p.y, 1.0 - abs(p.x) - abs(p.y)));
}

#endif
