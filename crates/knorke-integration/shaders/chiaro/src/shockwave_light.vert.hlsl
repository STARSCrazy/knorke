// Origin: chiaro @ 9bc5b9b, shaders/src/shockwave_light.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Ring light of the shockwaves (E143): one instance per wave with light in view, in list order. The quad is the
// wave's scene rectangle (CollectShockwaves: its torus, the ground circle of radius + width / 2 up to width / 2 high,
// projected, plus one scene pixel); the fragments find each pixel's world point and the distance to the ring.
// Rasterised at light buffer resolution like the light quads and the ambient zones.

// Matches struct RingPassParams in src/rendering/ShockwaveLightPass.cpp (GpuRingLight in src/core/Shockwave.hpp).
cbuffer RingPassParams : register(b0, space1)
{
    float2 SceneSize; // scene size in pixels
    float2 Pad;
    float4 Rects[16];  // x0, y0, x1, y1 in scene pixels
    float4 Rings[16];  // centre x, y (world), radius, 2 / width
    float4 Colors[16]; // rgb light * light_intensity
};

struct VSOutput
{
    float2 scene : TEXCOORD0;
    nointerpolation float4 ring : TEXCOORD1;
    nointerpolation float4 color : TEXCOORD2;
    float4 position : SV_Position;
};

static const float2 kCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

VSOutput main(uint vertexId : SV_VertexID, uint instanceId : SV_InstanceID)
{
    float4 rect = Rects[instanceId];
    float2 scene = lerp(rect.xy, rect.zw, kCorners[vertexId]);
    VSOutput o;
    o.scene = scene;
    o.ring = Rings[instanceId];
    o.color = Colors[instanceId];
    o.position = float4(scene / SceneSize * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
