// Origin: chiaro @ 9bc5b9b, shaders/src/occlusion_mask.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Occlusion mask: the application's sprite mask (CHI_RenderInfo.occlusion),
// drawn with fullscreen.vert over the whole mask and sampled nearest. The scene
// rectangle of the mask is passed in mask texels rather than set as a viewport:
// GPUs clip a fractional viewport to whole pixels, which loses the last column
// and row when the anchor's remainder is below 0.5. A texel belongs to the
// scene rectangle if its centre lies in [origin, origin + size). Alpha >= 0.5
// occludes.
// CHI_HEIGHT (occlusion_mask_height.frag): sprite mask texels are infinitely tall
// ground obstacles (0, kInfiniteTop); they cast no directional shadows (E24).
#ifdef CHI_HEIGHT
#include "occlusion_height.hlsli"
#endif

Texture2D<float4> Occlusion : register(t0, space2);
SamplerState OcclusionSampler : register(s0, space2);

cbuffer SpriteParams : register(b0, space3)
{
    float4 SceneRect; // xy origin, zw size, in mask texels
    float4 MaskSize;  // xy mask size in texels, zw unused
};

#ifdef CHI_HEIGHT
MaskHeightOutput main(float2 uv : TEXCOORD0)
#else
float4 main(float2 uv : TEXCOORD0) : SV_Target0
#endif
{
    float2 d = uv * MaskSize.xy - SceneRect.xy; // uv lies on texel centres
    if (any(d < 0.0) || any(d >= SceneRect.zw) ||
        Occlusion.SampleLevel(OcclusionSampler, d / SceneRect.zw, 0.0).a < 0.5) {
        discard;
    }
#ifdef CHI_HEIGHT
    return Occupied(float4(0.0, kInfiniteTop, -kNoHeight, -kNoHeight));
#else
    return float4(1.0, 0.0, 0.0, 0.0); // opacity 1, no footprint
#endif
}
