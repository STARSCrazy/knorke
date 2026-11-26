// Light rows (ADR 0019): the reference picture, a test tool only. The same light, the shadow from an exact march per
// pixel over the height texture, from the lamp to one texel before the receiver in quarter texels, as many steps as
// the way needs, no distance field. A sample blocks if the ray's height there lies in the texel's interval, with
// Chiaro's tolerance below a top (shadow_trace.hlsli, WithinInterval). A lamp whose height lies in the interval of
// its own texel does not count that obstacle while the ray stays in it, as the row build does. A partly transparent
// texel (mask opacity o below kOpaqueOpacity) multiplies by 1 - o once per crossing, as Chiaro's trace.
// A lamp with a source radius R is Domain.w x Domain.w points: across the ray in the ground plane and upright,
// each within +-R; the result is the mean of their marches.
#include "lightrow_surface.hlsli"

static const float kStep = 0.25;

// (base, top) of the texel under p; top below base where nothing stands or p is outside the mask.
float2 IntervalAt(float2 p)
{
    if (any(p < 0.0) || any(p >= Domain.xy)) {
        return float2(1.0, -1.0);
    }
    float2 i = Heights.SampleLevel(HeightSampler, (floor(p) + 0.5) / Domain.xy, 0.0).rg;
    return i.y > -i.x ? float2(max(-i.x, 0.0), i.y) : float2(1.0, -1.0);
}

// The share of the light from the lamp point (mask texels, height zl) that reaches the place `start` at height z.
float March(float2 lamp, float zl, float2 start, float z)
{
    float r = length(start - lamp);
    if (r < 1e-3) {
        return 1.0;
    }
    float2 dir = (start - lamp) / r;
    float2 own = IntervalAt(lamp);
    bool skipping = own.y >= own.x && own.x <= zl && zl <= own.y;
    int steps = int(floor((r - RowSize.z) / kStep));
    float seen = 1.0;
    bool inside = false;
    [loop] for (int s = 1; s <= steps; ++s) {
        float q = float(s) * kStep;
        float2 here = IntervalAt(lamp + dir * q);
        if (skipping) {
            if (all(here == own)) {
                continue;
            }
            skipping = false;
        }
        float h = lerp(zl, z, q / r);
        bool blocked = h >= here.x && h < here.y - RowSize.w;
        if (blocked) {
            float o = FetchTexel(Mask, MaskSampler, int2(floor(lamp + dir * q)), 1.0 / Domain.xy).r;
            if (o >= kOpaqueOpacity) {
                return 0.0;
            }
            seen *= inside ? 1.0 : 1.0 - o;
        }
        inside = blocked;
    }
    return seen;
}

float Sight(SurfaceLightVaryings l, float2 start, float z, float r)
{
    float size = ROW_SIZE(l);
    if (size <= 0.0) {
        return March(ROW_GROUND(l), ROW_HEIGHT(l), start, z);
    }
    float2 dir = (start - ROW_GROUND(l)) / r;
    float2 across = float2(-dir.y, dir.x);
    // `size` world units across the ray, in mask texels.
    float texels = size / length(float2(dot(WorldFromMask.xy, across), dot(WorldFromMask.zw, across)));
    float sum = 0.0;
    int points = int(Domain.w);
    [loop] for (int i = 0; i < points; ++i) {
        [loop] for (int j = 0; j < points; ++j) {
            float2 at = (float2(i, j) + 0.5) / float(points) * 2.0 - 1.0;
            sum += March(ROW_GROUND(l) + across * (at.x * texels), ROW_HEIGHT(l) + at.y * size, start, z);
        }
    }
    return sum / float(points * points);
}
