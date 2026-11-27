// Origin: chiaro @ 9bc5b9b, shaders/src/rect.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Instanced rectangles of texels for the local mask and field updates (E21):
// one quad of 6 vertices per instance, no vertex buffers. Rects[i] holds the
// half-open texel rectangle x0, y0, x1, y1 of instance i in a target of
// DomainSize.xy texels. uv matches fullscreen.vert, so the jump flooding
// fragment shaders run unchanged on the covered texels.

cbuffer RectParams : register(b0, space1)
{
    int4 Rects[64];    // SdfShadows / DirtyRectList::kMaxDirtyRects
    float4 DomainSize; // xy target size in texels, zw unused
};

struct VSOutput
{
    float2 uv : TEXCOORD0;
    float4 position : SV_Position;
};

static const float2 kCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

VSOutput main(uint vertexId : SV_VertexID, uint instanceId : SV_InstanceID)
{
    int4 r = Rects[instanceId];
    float2 texel = lerp(float2(r.xy), float2(r.zw), kCorners[vertexId]);
    VSOutput o;
    o.uv = texel / DomainSize.xy;
    o.position = float4(o.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
