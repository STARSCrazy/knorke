// Origin: chiaro @ 9bc5b9b, shaders/src/medium_world.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium world map, splat (medium_world.vert). Two RG32F targets per cell:
//   SV_Target0 (sigma, 1), added (ONE + ONE): the sum of sigma and the number of points that landed in the cell;
//   SV_Target1 (top, -bottom), blended MAX: the union of their slabs, the highest top and the lowest bottom.
// 32-bit floats: a cell at a low shadow resolution takes hundreds of points (fp16 counts stop at 2048 and sums
// overflow at 65504).
struct PSInput
{
    float4 value : TEXCOORD0; // (sigma, bottom, top, 0)
};

struct SplatOutput
{
    float2 sums : SV_Target0;
    float2 slab : SV_Target1;
};

SplatOutput main(PSInput i)
{
    SplatOutput o;
    o.sums = float2(i.value.x, 1.0);
    o.slab = float2(i.value.z, -i.value.y);
    return o;
}
