// Origin: chiaro @ 9bc5b9b, shaders/src/medium_world_top.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Medium world map, highest top and extent (medium_world_top.vert): the cell's value, blended MAX into the 2 x 1 target.
struct PSInput
{
    float4 value : TEXCOORD0;
};

float4 main(PSInput i) : SV_Target0
{
    return i.value;
}
