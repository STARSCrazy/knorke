// Sprite colour into the scene layer (P34), blended by its alpha (the pipeline blends RGB and
// leaves the layer alpha alone).

Texture2D<float4> Sprite : register(t0, space2);
SamplerState SpriteSampler : register(s0, space2);

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    return Sprite.Sample(SpriteSampler, uv);
}
