// Light rows (ADR 0019, plan 17.13 L7): a block of the medium stage's sun shadow heights (Chiaro's
// medium_sun_height.frag) read from the sun's map instead of marched through the distance field. A texel holds the
// height below which a point over its mask texel lies in the sun's shadow: the highest end of the receiver heights
// the map shadows there (its intervals and, from opacity 0.5 on, its partly transparent runs; their lower ends are
// not kept, as Chiaro's), Chiaro's tolerance below a top given back. A sun straight overhead has no map
// (SunHeightRows.x 1): the finite top of the texel itself, as Chiaro's.
#include "lightrow_sun.hlsli"

Texture2D<float4> Map : register(t0, space2);
Texture2D<float4> Run0 : register(t1, space2);     // read with MapPartial.x only
Texture2D<float4> Run1 : register(t2, space2);
Texture2D<float4> Heights : register(t3, space2);  // read straight overhead only
Texture2D<float4> Mask : register(t4, space2);
SamplerState MapSampler : register(s0, space2);    // nearest
SamplerState Run0Sampler : register(s1, space2);   // nearest
SamplerState Run1Sampler : register(s2, space2);   // nearest
SamplerState HeightSampler : register(s3, space2); // nearest
SamplerState MaskSampler : register(s4, space2);   // nearest

cbuffer SunParams : register(b0, space3)
{
    LIGHTROW_SUN_PARAMS
};

// `SunHeightRowsParams` of knorke-light2d: x 1 straight overhead (SunParams not read), yz the mask size in texels.
cbuffer SunHeightRowsParams : register(b1, space3)
{
    float4 SunHeightRows;
};

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float2 size = SunHeightRows.yz;
    float2 start = uv * size; // the texel's centre in mask texels
    if (SunHeightRows.x > 0.5) {
        int2 cell = clamp(int2(floor(start)), int2(0, 0), int2(size) - 1);
        float2 i = FiniteInterval(Heights, HeightSampler, cell, size);
        bool solid = FetchTexel(Mask, MaskSampler, cell, 1.0 / size).r >= 0.5;
        return float4(solid && i.y > i.x ? i.y : 0.0, 0.0, 0.0, 0.0);
    }
    float2 rel = start - MapOrigin.xy;
    float2 ij = clamp(floor(float2(dot(MapInverse.xy, rel), dot(MapInverse.zw, rel))), 0.0, MapOrigin.zw - 1.0);
    float2 c = MapOrigin.xy + (ij.x + 0.5) * MapAxes.xy + (ij.y + 0.5) * MapAxes.zw;
    int2 texel = SunWrap(asint(MapLattice.xy) + int2(ij), int2(MapOrigin.zw));
    float2 inv = 1.0 / MapOrigin.zw;
    float4 iv = FetchTexel(Map, MapSampler, texel, inv);
    float hi = max(iv.x <= iv.y ? iv.y : -kSunEmpty, iv.z <= iv.w ? iv.w : -kSunEmpty);
    if (MapPartial.x > 0.0) {
        float4 runs[2] = {FetchTexel(Run0, Run0Sampler, texel, inv), FetchTexel(Run1, Run1Sampler, texel, inv)};
        [unroll] for (int k = 0; k < 2; ++k) {
            float4 r = runs[k];
            if (r.y > -kSunEmpty * 0.5 && r.x <= r.y && SunMeanPass(r.w) <= 0.5) {
                hi = max(hi, r.y);
            }
        }
    }
    // The map's heights are on the ray from c: a receiver at start lies higher on it by DirSlope.z per texel toward
    // the sun.
    float shift = DirSlope.z * dot(start - c, DirSlope.xy);
    return float4(max(hi + Domain.z + shift, 0.0), 0.0, 0.0, 0.0);
}
