// Surface geometry into the surface layer (P35): a triangle list read from a storage buffer,
// without vertex or index buffers, drawn as one draw from vertex 0 (so SV_VertexID needs no
// offset; it would include the first vertex on Vulkan but not on D3D12).

struct Vertex
{
    float2 position; // layer pixels
    uint pixel;      // the encoded surface pixel, R in the low byte
    uint unused;     // 16 bytes: the same stride in std430 (Vulkan) and D3D structured buffers
};

StructuredBuffer<Vertex> Vertices : register(t0, space0);

cbuffer SurfaceParams : register(b0, space1)
{
    float4 LayerSize; // xy size of the layer in pixels
};

struct VSOutput
{
    float4 color : TEXCOORD0;
    float4 position : SV_Position;
};

VSOutput main(uint id : SV_VertexID)
{
    Vertex v = Vertices[id];
    uint p = v.pixel;
    VSOutput o;
    o.color = float4(p & 0xFFu, (p >> 8u) & 0xFFu, (p >> 16u) & 0xFFu, p >> 24u) / 255.0;
    o.position = float4(v.position / LayerSize.xy * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
