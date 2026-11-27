// Origin: chiaro @ 9bc5b9b, shaders/src/shockwave_light.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Ring light of the shockwaves (E143): the light of a flat torus around the ring, added (ONE + ONE) to the light
// buffer. t is the distance of the pixel's world point (SurfaceAt: the surface buffer's height, else the ground) from
// the ring circle on the ground in half widths, the light colour * intensity * (1 + cos(pi t)) / 2 for t < 1. No
// shadows, no normals. Same as RingLightWeight in src/core/Shockwave.cpp.
// t0 surface buffer (nearest; a stand-in that is not read without one).

#include "surface.hlsli"

Texture2D<float4> SurfaceTex : register(t0, space2);
SamplerState SurfaceSampler : register(s0, space2); // nearest

// Matches struct RingFragParams in src/rendering/ShockwaveLightPass.cpp.
cbuffer RingFragParams : register(b0, space3)
{
    float4 InvAxes;     // rows of the inverse of [x_axis y_axis]
    float4 OriginZAxis; // xy projection origin, zw z_axis (scene pixels per unit of height)
    float4 SceneStep;   // xy scene size in pixels, z surface height step, w 1 = surface buffer present
};

struct PSInput
{
    float2 scene : TEXCOORD0;
    nointerpolation float4 ring : TEXCOORD1;
    nointerpolation float4 color : TEXCOORD2;
};

float4 main(PSInput i) : SV_Target0
{
    float3 world;
    SurfaceAt(SurfaceTex, SurfaceSampler, i.scene, SceneStep, world, InvAxes, OriginZAxis);
    float a = length(world.xy - i.ring.xy) - i.ring.z;
    float t = length(float2(a, world.z)) * i.ring.w;
    if (t >= 1.0) {
        discard;
    }
    return float4(i.color.rgb * (0.5 + 0.5 * cos(3.14159265 * t)), 0.0);
}
