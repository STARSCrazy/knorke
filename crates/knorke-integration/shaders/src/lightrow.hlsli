// Light row probe (ADR 0019, throwaway): what the passes share.
// Everything runs in occlusion mask texels (the ground plane); heights are world units.
#ifndef KNORKE_LIGHTROW_HLSLI
#define KNORKE_LIGHTROW_HLSLI

// One lamp; `struct Lamp` of gpu_lightrow_probe.rs. A point light is a spot with a full cone.
struct Lamp
{
    float4 positionRadius; // xyz world position, w radius
    float4 colorFalloff;   // rgb colour * intensity, a falloff exponent
    float4 directionType;  // xyz unit direction, w 1 = the cone applies
    float4 cone;           // x cos(inner), y cos(outer), z ground radius, w reach in mask texels
    float4 mask;           // xy ground point in mask texels, z the lamp's row in the row atlas, w source radius (world)
};

// The lamp of an instance, passed on unchanged by lightrow.vert.
#define LIGHTROW_VARYINGS                              \
    float2 coord : TEXCOORD0;                          \
    nointerpolation float4 positionRadius : TEXCOORD1; \
    nointerpolation float4 colorFalloff : TEXCOORD2;   \
    nointerpolation float4 directionType : TEXCOORD3;  \
    nointerpolation float4 cone : TEXCOORD4;           \
    nointerpolation float4 mask : TEXCOORD5;

struct LightRowVaryings
{
    LIGHTROW_VARYINGS
};

static const float kTwoPi = 6.28318530718;
static const int kRuns = 4;            // K: runs kept per angle
static const float kEdgeFar = 63.0;     // lightrow_edge.frag: the outline lies farther than the pass walked
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
