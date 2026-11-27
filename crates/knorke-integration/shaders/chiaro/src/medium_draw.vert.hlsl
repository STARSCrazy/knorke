// Origin: chiaro @ 9bc5b9b, shaders/src/medium_draw.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// One cloud of CHI_DrawMedium on GPU layers: a rectangle in layer pixels (6 vertices, no vertex buffers). The
// colour carries the encoded medium pixel (bytes / 255); A is the density itself with a shape texture, which
// medium_draw.frag scales by the texture's alpha (it also thins the slab of R, G). Output layout as the vertex
// shader of the SDL renderer (COLOR0, TEXCOORD0), so the same fragment shaders also serve SDL_GPURenderState.

cbuffer DrawParams : register(b0, space1)
{
    float4 Rect;  // x, y, width, height in layer pixels
    float4 Color; // R top, G 255 - bottom, B kind * 16, A density (or its byte / 255), each as byte / 255
    float4 Layer; // xy layer size in pixels, zw unused
};

struct VSOutput
{
    float4 color : COLOR0;
    float2 uv : TEXCOORD0;
    float4 position : SV_Position;
};

static const float2 kCorners[6] = {
    float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
    float2(1.0, 0.0), float2(1.0, 1.0), float2(0.0, 1.0),
};

VSOutput main(uint vertexId : SV_VertexID)
{
    float2 corner = kCorners[vertexId];
    float2 pixel = Rect.xy + corner * Rect.zw;
    VSOutput o;
    o.color = Color;
    o.uv = corner;
    o.position = float4(pixel / Layer.xy * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
