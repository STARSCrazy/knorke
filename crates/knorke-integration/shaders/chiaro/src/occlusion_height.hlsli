// Origin: chiaro @ 9bc5b9b, shaders/src/occlusion_height.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Outputs of the occlusion mask programs with heights (E22): SV_Target0 the
// R8 mask (obstacle opacity, E25), SV_Target1 the RGBA16F height texture holding
// (-base, top) in world units: RG of every obstacle, BA of the obstacles with
// a finite height only (directional lights ignore infinitely tall ones, E24).
// The height target blends with MAX, so overlapping obstacles leave the
// enclosing interval. Free texels hold -kNoHeight in all channels; infinitely
// tall obstacles have top kInfiniteTop in RG and nothing in BA.
#ifndef CHI_OCCLUSION_HEIGHT_HLSLI
#define CHI_OCCLUSION_HEIGHT_HLSLI

static const float kInfiniteTop = 65504.0; // OcclusionStage::kInfiniteTop
static const float kNoHeight = 65504.0;

struct MaskHeightOutput
{
    float4 mask : SV_Target0;
    float4 height : SV_Target1;
};

// The mask holds the opacity in R and the footprint flag (CHI_ObstacleCommon::shadow_footprint) in G.
MaskHeightOutput OccupiedFootprint(float4 interval, float opacity, float footprint)
{
    MaskHeightOutput o;
    o.mask = float4(opacity, footprint, 0.0, 0.0);
    o.height = interval;
    return o;
}

MaskHeightOutput Occupied(float4 interval, float opacity)
{
    return OccupiedFootprint(interval, opacity, 0.0);
}

MaskHeightOutput Occupied(float4 interval)
{
    return Occupied(interval, 1.0);
}

#endif
