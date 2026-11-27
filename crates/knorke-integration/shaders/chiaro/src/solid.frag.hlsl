// Origin: chiaro @ 9bc5b9b, shaders/src/solid.frag.hlsl, copied unchanged below this notice; only its variant
// solid_height.frag is compiled; its blobs in ../compiled are Chiaro's
// shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Occlusion mask: every covered texel is an obstacle with the triangle's opacity (E25) in R and, in G, 1 for a
// footprint obstacle (CHI_ObstacleCommon::shadow_footprint).
// CHI_HEIGHT (solid_height.frag): also writes the triangle's interval to the height target.
#include "common.hlsli"

#ifdef CHI_HEIGHT
#include "occlusion_height.hlsli"

MaskHeightOutput main(nointerpolation float4 interval : TEXCOORD0, nointerpolation float2 opacity : TEXCOORD1)
{
    if (opacity.x < kMinObstacleOpacity) {
        discard;
    }
    return OccupiedFootprint(interval, opacity.x, opacity.y);
}
#else
float4 main(nointerpolation float2 opacity : TEXCOORD0) : SV_Target0
{
    if (opacity.x < kMinObstacleOpacity) {
        discard;
    }
    return float4(opacity.x, opacity.y, 0.0, 0.0);
}
#endif
