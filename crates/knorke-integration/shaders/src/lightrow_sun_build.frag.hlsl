// Light rows (ADR 0019, plan 17.13 L3): a sun's shadow map (lightrow_sun.hlsli), one fullscreen draw over the
// map, or over the bands an update rebuilds (lightrow_sun_band.vert). A map texel stores lattice texel (I, J)
// (SunWrap); its middle c, in world texels so that the result does not depend on the camera, casts the ray c + DirSlope.xy * q
// toward the sun; it walks the mask (world texel + anchor) texel by texel (DDA)
// from q = MapBuild.x until it leaves the mask or rises (by MapBuild.w per texel, the lowest ray of a soft sun's disc)
// above the highest finite top from the lowest receiver (MapBuild.y). A texel crossed over [q0, q1] with a finite obstacle (base, top) shadows the receiver heights
// [base - s q1, top - tolerance - s q0) at c (s = DirSlope.z): the ray from there sweeps the heights z + s q0 to
// z + s q1 over it, and Chiaro blocks where they meet the interval (WithinInterval). The map keeps the union of these
// as two sorted intervals (lo, hi); more are hulled where the gap is smallest. SV_Target1 (a soft sun's second target;
// a hard sun's pipeline has none) holds how far along the ray each interval's ends lie, (q of hi, q of lo), so the
// light can soften them; an end cut at the lowest receiver is 0 (no receiver sees it).
#include "lightrow_sun.hlsli"

Texture2D<float4> Heights : register(t0, space2);
SamplerState HeightSampler : register(s0, space2); // nearest

cbuffer SunParams : register(b0, space3)
{
    LIGHTROW_SUN_PARAMS
};

static const int kSlots = 2;
static const int kMaxSteps = 4096; // above mask width plus height (MAX_SUN_MAP in knorke-light2d)

struct MapOutput
{
    float4 intervals : SV_Target0;
    float4 reach : SV_Target1;
};

// Interval v = (lo, hi) with its ends' distances w = (q of hi, q of lo) into the sorted, disjoint intervals iv[0..n);
// a merged end keeps the distance of the interval it comes from.
void Insert(inout float2 iv[kSlots + 1], inout float2 at[kSlots + 1], inout int n, float2 v, float2 w)
{
    [loop] for (int k = n - 1; k >= 0; --k) {
        if (iv[k].x <= v.y && v.x <= iv[k].y) { // overlap or touch: absorb
            w = float2(iv[k].y > v.y ? at[k].x : w.x, iv[k].x < v.x ? at[k].y : w.y);
            v = float2(min(v.x, iv[k].x), max(v.y, iv[k].y));
            [loop] for (int m = k; m + 1 < n; ++m) {
                iv[m] = iv[m + 1];
                at[m] = at[m + 1];
            }
            --n;
        }
    }
    int pos = n;
    [loop] while (pos > 0 && iv[pos - 1].x > v.x) {
        iv[pos] = iv[pos - 1];
        at[pos] = at[pos - 1];
        --pos;
    }
    iv[pos] = v;
    at[pos] = w;
    ++n;
    if (n > kSlots) { // hull the neighbours with the smallest gap
        int best = 0;
        float gap = kSunEmpty;
        [loop] for (int k = 0; k + 1 < n; ++k) {
            float g = iv[k + 1].x - iv[k].y;
            if (g < gap) {
                gap = g;
                best = k;
            }
        }
        at[best].x = iv[best + 1].y > iv[best].y ? at[best + 1].x : at[best].x;
        iv[best].y = max(iv[best].y, iv[best + 1].y);
        [loop] for (int m = best + 1; m + 1 < n; ++m) {
            iv[m] = iv[m + 1];
            at[m] = at[m + 1];
        }
        --n;
    }
}

MapOutput main(float2 uv : TEXCOORD0)
{
    int2 size = int2(MapOrigin.zw);
    int2 corner = asint(MapLattice.xy);
    int2 anchor = asint(MapLattice.zw);
    // The lattice texel this map texel stores: the one in [corner, corner + size) congruent to it.
    int2 local = SunWrap(int2(floor(uv * MapOrigin.zw)) - corner, size);
    float2 ij = float2(corner + local) + 0.5;
    float2 c = ij.x * MapAxes.xy + ij.y * MapAxes.zw;
    float2 d = DirSlope.xy;
    float s = DirSlope.z;
    float zMin = MapBuild.y;

    float2 iv[kSlots + 1];
    float2 at[kSlots + 1];
    int n = 0;
    float q = MapBuild.x;
    float2 p = c + d * q;
    int2 cell = int2(floor(p));
    int2 step = int2(sign(d));
    float2 inv = float2(d.x != 0.0 ? 1.0 / abs(d.x) : kSunEmpty, d.y != 0.0 ? 1.0 / abs(d.y) : kSunEmpty);
    float2 frac = p - float2(cell);
    float2 next = q + float2(step.x > 0 ? 1.0 - frac.x : frac.x, step.y > 0 ? 1.0 - frac.y : frac.y) * inv;
    [loop] for (int k = 0; k < kMaxSteps; ++k) {
        int2 texel = cell + anchor;
        if (any(texel < 0) || any(texel >= int2(Domain.xy)) || zMin + MapBuild.w * q > Domain.w) {
            break;
        }
        float leave = min(next.x, next.y);
        float2 i = FiniteInterval(Heights, HeightSampler, texel, Domain.xy);
        float hi = i.y - Domain.z - s * q;
        if (i.y > i.x && hi > zMin) {
            float lo = i.x - s * leave;
            Insert(iv, at, n, float2(max(lo, zMin), hi), float2(q, lo < zMin ? 0.0 : leave));
        }
        q = leave;
        if (next.x < next.y) {
            cell.x += step.x;
            next.x += inv.x;
        } else {
            cell.y += step.y;
            next.y += inv.y;
        }
    }
    float2 e = float2(kSunEmpty, -kSunEmpty);
    MapOutput o;
    o.intervals = float4(n > 0 ? iv[0] : e, n > 1 ? iv[1] : e);
    o.reach = float4(n > 0 ? at[0] : 0.0, n > 1 ? at[1] : 0.0);
    return o;
}
