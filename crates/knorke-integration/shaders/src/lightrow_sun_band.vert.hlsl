// Light rows (ADR 0019, plan 17.13 L3): the bands a hard sun's map update builds again (`SunBands` of knorke-light2d),
// for lightrow_sun_build.frag. A band is a parallelogram in map texels counted from the map's first lattice texel; the
// map stores that texel at Start (lightrow_sun.hlsli, SunWrap), so instance 4 b + k draws band b moved back by the
// map's size along x (k & 1) and y (k & 2): together they cover the band wrapped around the texture.

struct SunBand
{
    float4 cornerSide; // xy corner, zw first side
    float4 side;       // xy second side
};

StructuredBuffer<SunBand> Bands : register(t0, space0);

cbuffer SunBandPass : register(b0, space1)
{
    float4 StartSize; // xy where the map's first lattice texel lies in the texture, zw the map's size
    uint First;       // this sun's first band in Bands
    uint3 Pad;
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
    SunBand band = Bands[First + instanceId / 4];
    uint wrap = instanceId % 4;
    float2 c = kCorners[vertexId];
    float2 p = band.cornerSide.xy + c.x * band.cornerSide.zw + c.y * band.side.xy;
    p += StartSize.xy - float2(wrap & 1, wrap >> 1) * StartSize.zw;
    VSOutput o;
    o.uv = p / StartSize.zw;
    o.position = float4(o.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
