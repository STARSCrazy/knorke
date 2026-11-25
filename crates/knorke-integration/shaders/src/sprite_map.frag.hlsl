// Sprite map (material, surface, occlusion) into its layer (P34): a texel with an entry
// (alpha above 0) replaces the layer texel exactly, an empty one leaves it. Not blended: a wet
// material entry has an alpha below 255 and must not be mixed with what lies under it.

Texture2D<float4> Sprite : register(t0, space2);
SamplerState SpriteSampler : register(s0, space2); // nearest

float4 main(float2 uv : TEXCOORD0) : SV_Target0
{
    float4 c = Sprite.Sample(SpriteSampler, uv);
    if (c.a <= 0.0) {
        discard;
    }
    return c;
}
