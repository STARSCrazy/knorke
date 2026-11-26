// Light rows (ADR 0019): what the row programs share. A lamp's row holds, per angle around its ground point, the
// obstacles a ray meets on its way out; the light looks its shadow up there instead of marching per pixel.
// Everything runs in occlusion mask texels (the ground plane); heights are world units.
// The constants here are those of `knorke_light2d::rows` (ROW_ANGLES, MARCH_STEP, MAX_REACH, EDGE_*).
#ifndef KNORKE_LIGHTROW_HLSLI
#define KNORKE_LIGHTROW_HLSLI

#define CHI_POINT_LIGHTS // point and spot lights only: no area varyings
#include "light_common.hlsli"
#include "surface.hlsli"

// `RowLamp` of knorke-light2d: Chiaro's light record and where its row lies.
struct RowLamp
{
    GpuLight light;
    float4 row; // x the lamp's row in the row atlas, -1 = casts no shadow; y source radius in world units
};

// Every row program takes Chiaro's surface light varyings (lightrow.vert). The row programs have no profiles, so
// `profile` carries RowLamp.row instead.
#define ROW_SLOT(i) i.profile.x
#define ROW_SIZE(i) i.profile.y
#define ROW_GROUND(i) i.shadow.zw // the lamp's ground point in mask texels
#define ROW_HEIGHT(i) i.shadow.y  // the lamp's height

static const float kTwoPi = 6.28318530718;
static const int kRuns = 4;                // runs kept per angle
static const float kEdgeFar = 63.0;        // lightrow_edge.frag: the outline lies farther than the pass walked
static const float kInfiniteTop = 60000.0; // tops from here on are infinitely tall (the mask writes 65504)

// A row texel holds two runs as four floats, each two 12-bit numbers (exact in a 32-bit float): distances in quarter
// texels (up to 1023.75), heights in hundredths; top 4095 is infinitely tall.
float2 PackRun(float4 run) // rn, rf, base, top
{
    float2 d = round(clamp(run.xy, 0.0, 1023.75) * 4.0);
    float b = round(clamp(run.z, 0.0, 40.9) * 100.0);
    float t = run.w >= 40.9 ? 4095.0 : round(max(run.w, 0.0) * 100.0);
    return float2(d.x + 4096.0 * d.y, b + 4096.0 * t);
}

float4 UnpackRun(float2 packed)
{
    float2 hi = floor(packed / 4096.0);
    float2 lo = packed - hi * 4096.0;
    return float4(lo.x * 0.25, hi.x * 0.25, lo.y * 0.01, hi.y >= 4095.0 ? 65504.0 : hi.y * 0.01);
}

#endif
