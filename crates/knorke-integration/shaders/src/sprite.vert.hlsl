// Sprite quads for the scene layers (P34). Draw 6 vertices per quad without vertex buffers;
// the quads come from a storage buffer. The first quad of a draw is a parameter, not the
// draw's first vertex: SV_VertexID includes the first vertex on Vulkan but not on D3D12.

struct Quad
{
    float4 dst; // xy top-left corner, zw size, in scene pixels
    float4 uv;  // xy the texture coordinate of the top-left corner, zw of the bottom-right one
};

StructuredBuffer<Quad> Quads : register(t0, space0);

cbuffer SpriteParams : register(b0, space1)
{
    float4 SceneSize; // xy size of the layer in pixels, z index of the draw's first quad
};

struct VSOutput
{
    float2 uv : TEXCOORD0;
    float4 position : SV_Position;
};

VSOutput main(uint id : SV_VertexID)
{
    Quad q = Quads[uint(SceneSize.z) + id / 6u];
    uint corner = id % 6u; // corners 0 1 2, then 3 2 1
    uint c = corner < 3u ? corner : 6u - corner;
    float2 t = float2(c & 1u, c >> 1u);
    float2 pixel = q.dst.xy + t * q.dst.zw;
    VSOutput o;
    o.uv = lerp(q.uv.xy, q.uv.zw, t);
    o.position = float4(pixel / SceneSize.xy * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
