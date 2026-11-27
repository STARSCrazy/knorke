// Origin: chiaro @ 9bc5b9b, shaders/src/clear.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Occlusion mask: clears the rectangles of a local update (rect.vert) to free (0)
// before the obstacles are drawn again. CHI_HEIGHT (clear_height.frag): also the
// height target, which is written without blending here.

#ifdef CHI_HEIGHT
#include "occlusion_height.hlsli"

MaskHeightOutput main(float2 uv : TEXCOORD0)
{
    MaskHeightOutput o;
    o.mask = float4(0.0, 0.0, 0.0, 0.0);
    o.height = float4(-kNoHeight, -kNoHeight, -kNoHeight, -kNoHeight);
    return o;
}
#else
float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    return float4(0.0, 0.0, 0.0, 0.0);
}
#endif
