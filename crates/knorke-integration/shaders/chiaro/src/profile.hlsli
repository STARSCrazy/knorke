// Origin: chiaro @ 9bc5b9b, shaders/src/profile.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Light profiles (docs/superpowers/specs/2026-11-12-lichtprofil-design.md): relative intensity over polar angle
// and azimuth, one block of 72 rows (azimuth 0, 5, ..., 355 degrees) by 181 texels (polar 0 ... 180 degrees) per
// profile in an R32_FLOAT atlas. Same sampling as SampleProfileTable and ProfileFactor in src/lighting/Profile.cpp.
//
// A fragment shader that lights point or spot lights defines CHI_PROFILE_SLOT, the sampler slot of the atlas
// (after all its other samplers), before it includes this file; CHI_PROFILE_FACTOR then samples it. Without
// CHI_PROFILE_SLOT (vertex shaders, directional programs) the factor is 1.
#ifndef CHI_PROFILE_HLSLI
#define CHI_PROFILE_HLSLI

#include "common.hlsli"

#ifdef CHI_PROFILE_SLOT

#define CHI_PROFILE_PASTE(a, b) a##b
#define CHI_PROFILE_REGISTER(kind, slot) register(CHI_PROFILE_PASTE(kind, slot), space2)

Texture2D<float4> ProfileAtlas : CHI_PROFILE_REGISTER(t, CHI_PROFILE_SLOT);
SamplerState ProfileSampler : CHI_PROFILE_REGISTER(s, CHI_PROFILE_SLOT); // nearest

static const int kProfilePolar = 181;
static const int kProfileAzimuth = 72;

float ProfileTexel(int polar, int row, float2 invSize)
{
    return FetchTexel(ProfileAtlas, ProfileSampler, int2(polar, row), invSize).r;
}

// The profile value toward `toPixel` (light to pixel). profile: x first atlas row or < 0 without a profile (then
// exactly 1), yzw the C0 unit vector; axis: the light's unit direction; invHeight: 1 / atlas height in texels.
float ProfileFactorOf(float4 profile, float3 axis, float invHeight, float3 toPixel)
{
    if (profile.x < 0.0) {
        return 1.0;
    }
    float d = length(toPixel);
    float3 w = d > 1e-6 ? toPixel / d : axis;
    float3 r = profile.yzw;
    float3 c90 = cross(r, axis);
    // On the axis the azimuth is undefined. std::atan2 (ProfileFactor) and D3D12 give 0 for atan2(0, 0), SPIR-V's
    // Atan2 (GLSL.std.450) leaves it undefined and gives NaN on Vulkan, which blackens or poisons the pixel under a
    // profiled light and every medium sample on its axis: take 0 there explicitly.
    float ax = dot(w, r);
    float ay = dot(w, c90);
    float azimuth = (ax != 0.0 || ay != 0.0) ? degrees(atan2(ay, ax)) : 0.0;
    if (azimuth < 0.0) {
        azimuth += 360.0;
    }
    float u = clamp(degrees(acos(clamp(dot(w, axis), -1.0, 1.0))), 0.0, 180.0);
    int i0 = min(int(u), kProfilePolar - 2);
    float f = u - float(i0);
    float v = azimuth / 5.0;
    float vf = floor(v);
    float g = v - vf;
    int j0 = int(vf) % kProfileAzimuth;
    int j1 = (j0 + 1) % kProfileAzimuth;
    int row = int(profile.x);
    float2 invSize = float2(1.0 / float(kProfilePolar), invHeight);
    float a0 = ProfileTexel(i0, row + j0, invSize);
    float a = a0 + (ProfileTexel(i0 + 1, row + j0, invSize) - a0) * f;
    float b0 = ProfileTexel(i0, row + j1, invSize);
    float b = b0 + (ProfileTexel(i0 + 1, row + j1, invSize) - b0) * f;
    return a + (b - a) * g;
}

#define CHI_PROFILE_FACTOR(profile, axis, invHeight, toPixel) ProfileFactorOf(profile, axis, invHeight, toPixel)

#else

#define CHI_PROFILE_FACTOR(profile, axis, invHeight, toPixel) 1.0

#endif

#endif
