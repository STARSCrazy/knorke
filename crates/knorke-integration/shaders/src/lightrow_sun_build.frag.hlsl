// Light rows (ADR 0019, plan 17.13 L3): a sun's shadow map (lightrow_sun.hlsli), one fullscreen draw over the
// map, or over the bands an update rebuilds (lightrow_sun_band.vert). A map texel stores lattice texel (I, J)
// (SunWrap); its middle c, in world texels so that the result does not depend on the camera, casts the ray c + DirSlope.xy * q
// toward the sun; it walks the mask (world texel + anchor) texel by texel (DDA)
// from q = MapBuild.x until it leaves the mask or rises (by MapBuild.w per texel, the lowest ray of a soft sun's disc)
// above the highest finite top from the lowest receiver (MapBuild.y). A texel crossed over [q0, q1] with a finite obstacle (base, top) shadows the receiver heights
// [base - s q1, top - tolerance - s q0) at c (s = DirSlope.z > 0; a sun below the horizon, s <= 0, q0 and q1 swapped and no
// rise to end the march): the ray from there sweeps the heights z + s q0 to
// z + s q1 over it, and Chiaro blocks where they meet the interval (WithinInterval). The map keeps the union of these
// as two sorted intervals (lo, hi); more are hulled where the gap is smallest. SV_Target1 (a soft sun's second target;
// a hard sun's pipeline has none) holds how far along the ray each interval's ends lie, (q of hi, q of lo), so the
// light can soften them; an end cut at the lowest receiver is 0 (no receiver sees it).
// SUN_PARTIAL (lightrow_sun_build_partial.frag, a map with partly transparent runs, MapPartial.x; a map without
// keeps the program above unchanged): a texel whose mask opacity o lies below kOpaqueOpacity (E25) does not join
// the intervals: a run
// of such texels with one interval is one crossing (Chiaro's trace multiplies once per crossing, by its entering
// opacity) and goes to SV_Target2/3 as (lo, hi, q where it begins, pass), lo -kSunEmpty where cut at the lowest
// receiver; pass is 1 - o of its first texel, or, on a pattern obstacle (MapPartial.y: the id texture),
// -(id + (1 - o) / 2) and the light reads the pattern (lightrow_sun_light.hlsli). A third run joins the second:
// their intervals hulled, their passes multiplied (a pattern's mean, the mask's opacity of its texels).
#include "lightrow_sun.hlsli"

Texture2D<float4> Heights : register(t0, space2);
SamplerState HeightSampler : register(s0, space2); // nearest
#ifdef SUN_PARTIAL
Texture2D<float4> Mask : register(t1, space2);       // opacity in r
Texture2D<float4> PatternIds : register(t2, space2); // R16_UNORM, 1 + record index; read with MapPartial.y only
SamplerState MaskSampler : register(s1, space2);     // nearest
SamplerState PatternIdSampler : register(s2, space2); // nearest
#endif

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
#ifdef SUN_PARTIAL
    float4 run0 : SV_Target2;
    float4 run1 : SV_Target3;
#endif
};

#ifdef SUN_PARTIAL
static const float4 kNoRun = float4(kSunEmpty, -kSunEmpty, 0.0, 1.0);

// Closes run r (pass code in w) into the two slots.
void Close(inout float4 runs[2], inout int m, float4 r)
{
    if (m < 2) {
        runs[m] = r;
        ++m;
        return;
    }
    float4 s = runs[1];
    runs[1] = float4(min(s.x, r.x), max(s.y, r.y), min(s.z, r.z), SunMeanPass(s.w) * SunMeanPass(r.w));
}

// The run's code: its pass, or -(id + pass / 2) on a pattern obstacle.
float4 Coded(float4 run, uint id)
{
    return float4(run.xyz, id > 0u ? -(float(id) + 0.5 * run.w) : run.w);
}
#endif

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
#ifdef SUN_PARTIAL
    float4 runs[2] = {kNoRun, kNoRun};
    int m = 0;
    bool open = false; // the run being walked: its interval, record id and (lo, hi, q, 1 - o)
    float2 runI = float2(0.0, 0.0);
    uint runId = 0u;
    float4 run = kNoRun;
#endif
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
        // Where over the texel the ray sweeps the lowest and the highest receiver heights: a sun below the horizon
        // (s <= 0) has them the other way round.
        float2 ends = s > 0.0 ? float2(leave, q) : float2(q, leave);
        float2 i = FiniteInterval(Heights, HeightSampler, texel, Domain.xy);
        float hi = i.y - Domain.z - s * ends.y;
        bool blocks = i.y > i.x && hi > zMin;
        float lo = i.x - s * ends.x;
#ifdef SUN_PARTIAL
        float o = blocks ? FetchTexel(Mask, MaskSampler, texel, 1.0 / Domain.xy).r : 1.0;
        bool through = o < kOpaqueOpacity;
        if (open && !(through && all(i == runI))) {
            Close(runs, m, Coded(run, runId));
            open = false;
        }
        if (blocks && through) {
            uint id = 0u;
            if (MapPartial.y > 0.0) {
                id = uint(FetchTexel(PatternIds, PatternIdSampler, texel, 1.0 / Domain.xy).r * 65535.0 + 0.5);
            }
            if (!open) {
                open = true;
                runI = i;
                runId = id;
                run = float4(0.0, hi, q, 1.0 - o);
            }
            runId = runId > 0u ? runId : id; // the id texture may give a thin obstacle's texels late
            run.x = lo < zMin ? -kSunEmpty : lo;
            blocks = false;
        }
#endif
        if (blocks) {
            Insert(iv, at, n, float2(max(lo, zMin), hi), float2(ends.y, lo < zMin ? 0.0 : ends.x));
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
#ifdef SUN_PARTIAL
    if (open) {
        Close(runs, m, Coded(run, runId));
    }
#endif
    float2 e = float2(kSunEmpty, -kSunEmpty);
    MapOutput o;
    o.intervals = float4(n > 0 ? iv[0] : e, n > 1 ? iv[1] : e);
    o.reach = float4(n > 0 ? at[0] : 0.0, n > 1 ? at[1] : 0.0);
#ifdef SUN_PARTIAL
    o.run0 = runs[0];
    o.run1 = runs[1];
#endif
    return o;
}
