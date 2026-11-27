// Origin: chiaro @ 9bc5b9b, shaders/src/debug_check.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Debug checks on the GPU (E83): one triangle per scene texel into a 1x1 target, drawn with ONE + ONE. A texel that
// fails a check emits a triangle covering the target with its findings as colour; every other texel emits a
// degenerate triangle (no fragment). So the target ends up non-zero exactly in the channels of the checks that fail
// somewhere: x half-alpha surface texel, y half-alpha material texel, z blown-out highlight, w medium pixel with
// density but its top edge not above its bottom edge (E91). Draw 3 * width * height
// vertices. Reads only; the frame's image does not depend on it.
#include "surface.hlsli"

Texture2D<float4> SurfaceBuffer : register(t0, space0);
SamplerState SurfaceSampler : register(s0, space0); // nearest
Texture2D<float4> MaterialBuffer : register(t1, space0);
SamplerState MaterialSampler : register(s1, space0); // nearest
Texture2D<float4> MediumBuffer : register(t2, space0);
SamplerState MediumSampler : register(s2, space0); // nearest

// Matches struct DebugCheckParams in src/rendering/DebugCheckStage.cpp.
cbuffer DebugCheckParams : register(b0, space1)
{
    float4 SizePresent; // xy scene size in texels, z 1 = surface buffer bound, w 1 = material buffer bound
    float4 Highlight;   // x specular_scale, y wet stretch (1 off), z wet breakup, w blow-out limit (peak / diffuse)
    float4 Medium;      // x 1 = medium buffer bound
};

struct VSOutput
{
    nointerpolation float4 findings : TEXCOORD0;
    float4 position : SV_Position;
};

// Peak of the normalized highlight relative to the diffuse light of the same light straight along the normal
// (both 1 at intensity 1): specular * specular_scale * N(e), with the wet streak's 1 / sqrt(stretch) and the
// ripple gain (1 + breakup) where the wet lobe can apply.
float HighlightPeak(int2 texel, Material m)
{
    float e = MaterialExponent(MaterialBuffer, MaterialSampler, texel, SizePresent.xy, m, Highlight.x);
    float peak = m.strength * Highlight.x * SpecularNormalization(e);
    if (m.wet && Highlight.y > 1.0) {
        peak *= rsqrt(Highlight.y) * (1.0 + Highlight.z);
    }
    return peak;
}

VSOutput main(uint id : SV_VertexID)
{
    uint w = uint(SizePresent.x);
    uint index = id / 3u;
    int2 texel = int2(index % w, index / w);
    float2 uv = (float2(texel) + 0.5) / SizePresent.xy;
    float4 findings = float4(0.0, 0.0, 0.0, 0.0);
    if (SizePresent.z > 0.0) {
        uint a = uint(round(saturate(SurfaceBuffer.SampleLevel(SurfaceSampler, uv, 0.0).a) * 255.0));
        findings.x = a > 0u && a < 255u ? 1.0 : 0.0; // an entry is 255, ground 0
    }
    if (SizePresent.w > 0.0) {
        float4 c = MaterialBuffer.SampleLevel(MaterialSampler, uv, 0.0);
        uint a = uint(round(saturate(c.a) * 255.0));
        findings.y = a > 0u && a < 192u ? 1.0 : 0.0; // dry entries 255, wet 192..254, none 0
        Material m = DecodeMaterial(c);
        findings.z = m.present && HighlightPeak(texel, m) > Highlight.w ? 1.0 : 0.0;
    }
    if (Medium.x > 0.0) {
        float4 c = MediumBuffer.SampleLevel(MediumSampler, uv, 0.0);
        uint density = uint(round(saturate(c.a) * 255.0));
        uint top = uint(round(saturate(c.r) * 255.0));
        uint bottom = 255u - uint(round(saturate(c.g) * 255.0)); // G holds 255 - the bottom edge
        findings.w = density > 0u && top <= bottom ? 1.0 : 0.0;
    }
    VSOutput o;
    o.findings = findings;
    uint corner = id % 3u;
    float2 p = float2((corner << 1) & 2, corner & 2) * 2.0 - 1.0; // the fullscreen triangle of the 1x1 target
    bool any = dot(findings, findings) > 0.0;
    o.position = float4(any ? p : float2(-2.0, -2.0), 0.0, 1.0);
    return o;
}
