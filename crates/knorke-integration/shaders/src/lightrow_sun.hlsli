// Light rows (ADR 0019, plan 17.13 L3): what the programs of a sun's shadow map share. lightrow_sun_build.frag
// writes, per map texel, the receiver heights a ray toward the sun from there finds shadowed (and, for a soft sun, how
// far along the ray their ends lie); lightrow_sun_light.hlsli looks a pixel's height up there, lightrow_sun_truth.frag
// (a test tool) marches instead. Like Chiaro's directional
// lights (E24) only finite obstacles block (the height texture's .ba), with Chiaro's tolerance below a top; partly
// transparent ones and pattern obstacles let through what Chiaro's trace lets through (E25).
#ifndef KNORKE_LIGHTROW_SUN_HLSLI
#define KNORKE_LIGHTROW_SUN_HLSLI

#include "common.hlsli"

// `SunParams` of knorke-light2d; the first three rows are Chiaro's DirectionalParams.
#define LIGHTROW_SUN_PARAMS                                                                                          \
    float4 DirSlope;   /* xy unit direction toward the sun in mask texels, z height gained per texel */              \
    float4 Color;      /* rgb color * intensity */                                                                   \
    float4 World;      /* xyz unit vector toward the sun in world space, w specular factor (E47) */                  \
    float4 Domain;     /* xy mask size in texels, z tolerance below a top, w highest finite top */                   \
    float4 MapAxes;    /* xy, zw: the steps of lattice texel I and J in mask texels */                               \
    float4 MapOrigin;  /* xy the corner of the map's first lattice texel in mask texels, zw the map's size */        \
    float4 MapInverse; /* map texel coordinates of mask point p: (dot(xy, p - corner), dot(zw, p - corner)) */      \
    float4 MapBuild;   /* x texels before the receiver that do not count (Chiaro: 1), y the lowest receiver */      \
                       /* height a map texel stands in for, z a soft sun's rise (0: hard), w the height the */     \
                       /* march rises by per texel (the disc's lowest ray) */                                       \
    float4 MapLattice; /* int bits: xy the map's first lattice texel, zw the mask anchor (mask = world + zw) */  \
    float4 MapPartial; /* x 1: the map keeps partly transparent runs, y 1: their pattern ids */

static const float kSunEmpty = 1e30; // an unused interval: (kSunEmpty, -kSunEmpty)

// v mod size, in [0, size): where the map of `size` texels stores lattice texel v (wrapped around it, so a pan keeps
// every texel in place).
int2 SunWrap(int2 v, int2 size)
{
    int2 t = v % size;
    return select(t < 0, t + size, t);
}

// The pass of a partly transparent run's code (lightrow_sun_build.frag): a pattern run's (-(id + pass / 2)) is the
// mask's, the pattern's mean.
float SunMeanPass(float code)
{
    return code >= 0.0 ? code : 2.0 * (-code - floor(-code));
}

// (base, top) of the finite obstacle in mask texel `cell`, top below base where none stands.
float2 FiniteInterval(Texture2D<float4> heights, SamplerState nearestSampler, int2 cell, float2 size)
{
    float2 i = FetchTexel(heights, nearestSampler, cell, 1.0 / size).ba; // (-base, top)
    return i.y > -i.x ? float2(max(-i.x, 0.0), i.y) : float2(1.0, -1.0);
}

#endif
