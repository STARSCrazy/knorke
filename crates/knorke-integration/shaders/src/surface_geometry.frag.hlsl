// Surface geometry (P35): the interpolated surface pixel, written unblended. Normal and alpha are
// the same at all three vertices and the height is affine across a planar face, so linear
// interpolation and the UNORM rounding of the target give the encoded bytes.

float4 main(float4 color : TEXCOORD0) : SV_Target0
{
    return color;
}
