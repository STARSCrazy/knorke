// Origin: chiaro @ 9bc5b9b, shaders/src/pattern_id.frag.hlsl, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Pattern id pass for shapes (Muster-Hindernisse, pattern.hlsli): occluder.vert draws the triangles of the pattern
// obstacles of the frame with their id, 1 + index of the obstacle's GpuPatternObstacle record, in the slots that hold
// the opacities for the mask (TriangleOpacity, at most 65535, exact in a float). Every covered texel of the id texture
// (kPatternIdFormat, R16_UNORM, cleared to 0, not blended: Vulkan does not guarantee blending for R16_UNORM, so
// the last draw wins in the fixed draw order) gets id / 65535.
float4 main(nointerpolation float2 opacity : TEXCOORD0) : SV_Target0
{
    if (opacity.x < 0.5) {
        discard;
    }
    return float4(opacity.x / 65535.0, 0.0, 0.0, 0.0);
}
