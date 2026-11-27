// Origin: chiaro @ 9bc5b9b, shaders/src/medium_draw.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// A cloud of the medium layer. The colour is the encoded pixel (R top, G 255 - bottom, B kind * 16, A density). With
// a shape texture the cloud is a volume: at shape alpha a the density (A) is scaled by a and the slab shrinks toward
// its middle to sqrt(1 - (1 - a)^2) of its thickness, so a puff is round in height too. Where the shape is clear
// or its slab is thinned to no height step nothing is written: R, G, B would otherwise raise the height span and
// the kind of a neighbouring cloud under the MAX blend, and A would add density without air.
// CHI_FLAT (medium_draw_flat.frag): no shape texture, the slab as given.
// Inputs as the vertex shader of the SDL renderer, so SDL_GPURenderState can use this shader too.

struct PSInput
{
    float4 color : COLOR0;
    float2 uv : TEXCOORD0;
};

#ifdef CHI_FLAT
float4 main(PSInput i) : SV_Target0
{
    return i.color;
}
#else
Texture2D<float4> Shape : register(t0, space2);
SamplerState ShapeSampler : register(s0, space2);

float4 main(PSInput i) : SV_Target0
{
    float a = Shape.Sample(ShapeSampler, i.uv).a;
    if (a <= 0.0) {
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    // On the bytes, so that renderer and GPU layers round alike.
    float top = round(i.color.r * 255.0);
    float bottom = 255.0 - round(i.color.g * 255.0);
    float mid = 0.5 * (bottom + top);
    float half = 0.5 * (top - bottom);
    float k = sqrt(saturate(a * (2.0 - a))); // sqrt(1 - (1 - a)^2)
    float t = floor(mid + half * k + 0.5);
    float b = floor(mid - half * k + 0.5);
    if (t <= b) { // thinned to nothing at the rim: no air, so no density either
        return float4(0.0, 0.0, 0.0, 0.0);
    }
    return float4(t / 255.0, (255.0 - b) / 255.0, i.color.b, i.color.a * a);
}
#endif
