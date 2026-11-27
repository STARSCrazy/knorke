// Light rows (ADR 0019): the pass after the row build, for lamps with a size. Per angle it walks
// the neighbouring angles of the row (RowSize.z to each side, two texels each) and writes
//   first.x, first.y  for the four runs how many angles to the left and to the right hold the same obstacle
//                     (6 bits each: L0, R0, L1, R1 and L2, R2, L3, R3); kEdgeFar means "farther than walked"
//   first.w           for the same eight where along the run the outline stands: the middle of the obstacle's run
//                     in the last angle that holds it, as eighths of this angle's run (3 bits each)
//   first.z           how many angles away the nearest run stands that this angle does not hold, to the left
//                     (6 bits) and to the right (6 bits); 0 = none within reach
//   second            those two runs, packed as in the row
// With them the light knows where an obstacle's outline lies across the ray.
#include "lightrow.hlsli"

Texture2D<float4> RowFirst : register(t0, space2);
Texture2D<float4> RowSecond : register(t1, space2);
SamplerState RowFirstSampler : register(s0, space2);  // nearest
SamplerState RowSecondSampler : register(s1, space2); // nearest

cbuffer EdgeParams : register(b0, space3)
{
    float4 RowSize; // x angles, y lamps, z most angles walked to each side (below kEdgeFar)
    float4 Near;    // x lamp size (world) per angle of penumbra behind an obstacle one texel away, y the distance
                    // in texels from which on a penumbra is whole: nearer obstacles get theirs cut
    float4 HeightCode; // how the row stores heights (PackRun in lightrow.hlsli)
};

#include "lightrow_runs.hlsli"

struct EdgeOutput
{
    float4 first : SV_Target0;
    float4 second : SV_Target1;
};

EdgeOutput main(SurfaceLightVaryings i)
{
    float a = floor(i.scene.x * RowSize.x);
    float4 own[kRuns];
    float4 pass;
    Runs(a, ROW_SLOT(i), own, pass);
    OpaqueOnly(own, pass);
    EdgeOutput o;
    o.first = float4(0.0, 0.0, 0.0, 0.0);
    o.second = o.first;
    if (ROW_SIZE(i) <= 0.0) {
        return o; // a lamp without a size never reads this
    }
    // A lamp point at the lamp's rim sees past an outline d texels away size / d off the middle ray: no penumbra
    // reaches farther across. A run is followed that far; a run this angle does not hold is looked for as far as
    // one Near.y texels away would matter.
    float reach = ROW_SIZE(i) / Near.x;
    int width = int(min(RowSize.z, ceil(reach / Near.y)));
    float same[2][kRuns];
    float where = 0.0;
    float4 strange[2];
    float away[2];
    for (int side = 0; side < 2; ++side) {
        float4 current[kRuns]; // the run followed from angle to angle
        bool alive[kRuns];
        float need[kRuns];
        for (int k = 0; k < kRuns; ++k) {
            current[k] = own[k];
            alive[k] = own[k].y > 0.0;
            same[side][k] = 0.0;
            need[k] = min(ceil(reach / max(own[k].x, 1.0)) + 1.0, float(width));
        }
        strange[side] = float4(0.0, 0.0, 0.0, 0.0);
        away[side] = 0.0;
        [loop] for (int j = 1; j <= width; ++j) {
            float4 other[kRuns];
            Runs(a + (side == 0 ? -float(j) : float(j)), ROW_SLOT(i), other, pass);
            OpaqueOnly(other, pass);
            bool taken[kRuns] = { false, false, false, false };
            bool following = false;
            for (int k = 0; k < kRuns; ++k) {
                if (!alive[k]) {
                    continue;
                }
                if (float(j) > need[k]) {
                    same[side][k] = kEdgeFar;
                    alive[k] = false;
                    continue;
                }
                int match = -1;
                for (int q = 0; q < kRuns; ++q) {
                    if (match < 0 && !taken[q] && Same(current[k], other[q])) {
                        match = q;
                    }
                }
                if (match >= 0) {
                    taken[match] = true;
                    current[k] = other[match];
                    same[side][k] = float(j);
                    following = true;
                } else {
                    alive[k] = false;
                }
            }
            for (int q = 0; q < kRuns; ++q) {
                if (away[side] == 0.0 && !taken[q] && other[q].y > 0.0) {
                    strange[side] = other[q];
                    away[side] = float(j);
                }
            }
            if (!following && away[side] > 0.0) {
                break;
            }
        }
        for (int n = 0; n < kRuns; ++n) {
            same[side][n] = alive[n] ? kEdgeFar : same[side][n]; // still followed where the walk ended
            float along = (0.5 * (current[n].x + current[n].y) - own[n].x) / max(own[n].y - own[n].x, 1e-3);
            where += round(saturate(along) * 7.0) * pow(8.0, float(2 * n + side));
        }
    }
    o.first = float4(same[0][0] + 64.0 * same[1][0] + 4096.0 * (same[0][1] + 64.0 * same[1][1]),
                     same[0][2] + 64.0 * same[1][2] + 4096.0 * (same[0][3] + 64.0 * same[1][3]),
                     away[0] + 64.0 * away[1], where);
    o.second = float4(PackRun(strange[0], HeightCode), PackRun(strange[1], HeightCode));
    return o;
}
