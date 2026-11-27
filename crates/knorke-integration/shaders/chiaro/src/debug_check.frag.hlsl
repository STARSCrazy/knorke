// Origin: chiaro @ 9bc5b9b, shaders/src/debug_check.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Debug checks on the GPU (E83): writes the findings of debug_check.vert (blended ONE + ONE into a 1x1 target).
float4 main(nointerpolation float4 findings : TEXCOORD0) : SV_Target0
{
    return findings;
}
