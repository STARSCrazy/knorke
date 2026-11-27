// Origin: chiaro @ 9bc5b9b, shaders/src/fullscreen.vert.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Fullscreen triangle without vertex buffers. Draw with 3 vertices.
// uv (0,0) is the top-left corner of the target on every backend.

struct VSOutput
{
    float2 uv : TEXCOORD0;
    float4 position : SV_Position;
};

VSOutput main(uint id : SV_VertexID)
{
    VSOutput o;
    o.uv = float2((id << 1) & 2, id & 2);
    o.position = float4(o.uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}
