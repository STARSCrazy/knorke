// Origin: chiaro @ 9bc5b9b, shaders/src/medium_common.hlsli, copied unchanged below this notice; the blobs in
// ../compiled are Chiaro's shadercross output of that revision, copied bit for bit.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as
// such and this notice may not be removed or altered from any source distribution. Full text:
// LICENSE-CHIARO-ZLIB.txt.
// Media and volumetrics (spec 2026-11-13-medien-volumetrik, "Modell" and addendum items 3 to 5): the uniform block of
// the medium stage, the view ray, the decoding of the medium layer, the analytic optical depth of a pixel's column,
// the static sample noise, the phase function and the edge-aware upsample of the scatter buffer in composite.
//
// View ray: under the parallel projection (E17) every scene pixel is a line in the world with direction w (toward
// the viewer, w.z > 0); a point at height h on it has the ground point InvAxes * (scene - origin - h * z_axis).
// The path length per unit of height is 1 / w.z. The ray starts at the visible point, height z0 from the surface
// buffer (0 without one).
//
// Include after common.hlsli and surface.hlsli. Define CHI_MEDIUM_PARAMS_SLOT (default b0) before the include when
// the shader has another uniform block in b0, CHI_MEDIUM_PARAMS_SPACE (default space3, fragment) in a vertex shader.
#ifndef CHI_MEDIUM_COMMON_HLSLI
#define CHI_MEDIUM_COMMON_HLSLI

#ifndef CHI_MEDIUM_PARAMS_SLOT
#define CHI_MEDIUM_PARAMS_SLOT b0
#endif
#ifndef CHI_MEDIUM_PARAMS_SPACE
#define CHI_MEDIUM_PARAMS_SPACE space3
#endif

static const uint kMediumCount = 16u; // CHI_MEDIUM_COUNT

// Matches struct MediumParams in src/rendering/MediumStage.hpp (static_asserts on size and offsets there).
cbuffer MediumParams : register(CHI_MEDIUM_PARAMS_SLOT, CHI_MEDIUM_PARAMS_SPACE)
{
    float4 MediumInvAxes;    // rows of the inverse of [x_axis y_axis]: ground = (dot(xy, v), dot(zw, v))
    float4 MediumOriginZAxis; // xy projection origin, zw z_axis (scene pixels per unit of height)
    float4 MediumSizes;      // xy scene size, zw scatter buffer size, in pixels
    float4 MediumView;       // xyz view direction w (toward the viewer, w.z > 0), w 1 / w.z
    float4 MediumSteps;      // x surface height step, y medium height step, z 1 = surface buffer, w 1 = medium layer
    float4 MediumHaze;       // x haze density at the ground (0 = no haze), y height falloff H (> 0), z noise cells
                             // per world unit (MediumWorldNoise)
    float4 MediumHazeColor;  // rgb albedo of the haze, a its Henyey-Greenstein g
    float4 MediumAmbient;    // rgb CHI_Settings::ambient
    float4 MediumTable[16];  // the media table: rgb albedo, a extinction per world unit at density 1
    float4 MediumLightPass;  // x scatter samples N per pixel and light, y transmission samples K (medium_light.frag)
    float4 MediumPhase[4];   // Henyey-Greenstein g of the media table, entry i in [i / 4][i % 4]
};

// --- view ray -------------------------------------------------------------------------------------------------

// The ground point (z = 0 plane coordinates) of the point at height h on the ray of scene position `scene`.
float2 MediumGroundAt(float2 scene, float h)
{
    float2 v = scene - MediumOriginZAxis.xy - h * MediumOriginZAxis.zw;
    return float2(dot(MediumInvAxes.xy, v), dot(MediumInvAxes.zw, v));
}

// --- medium layer ---------------------------------------------------------------------------------------------

// One pixel of the medium layer: a slab of constant density between bottom and top along the pixel's ray.
struct MediumSlab
{
    float density; // 0 ... 1
    float bottom;  // world units
    float top;     // world units; empty when top <= bottom or density 0
    uint kind;     // entry of the media table
};

// R top steps, G 255 - bottom steps, B kind * 16, A density (CHI_EncodeMediumPixel).
MediumSlab DecodeMediumLayer(float4 c, float step)
{
    uint4 b = uint4(round(saturate(c) * 255.0));
    MediumSlab m;
    m.density = float(b.a) / 255.0;
    m.top = float(b.r) * step;
    m.bottom = float(255u - b.g) * step;
    m.kind = min(b.b >> 4, kMediumCount - 1u);
    return m;
}

// The scene texel a scatter buffer texel stands for: the scene pixel under its centre. The base pass computes the
// texel there, the upsample reads its guide there.
int2 MediumSourceTexel(int2 scatterTexel)
{
    float2 scene = (float2(scatterTexel) + 0.5) * MediumSizes.xy / MediumSizes.zw;
    return clamp(int2(floor(scene)), int2(0, 0), int2(MediumSizes.xy) - 1);
}

// Height z0 of the visible point of a scene texel: the surface buffer, 0 without one or for pixels without entry.
float MediumSurfaceZ(Texture2D<float4> surface, SamplerState nearestSampler, int2 texel)
{
    if (MediumSteps.z == 0.0) {
        return 0.0;
    }
    return DecodeSurface(FetchTexel(surface, nearestSampler, texel, 1.0 / MediumSizes.xy), MediumSteps.x).z;
}

// The medium slab of a scene texel; no medium without a medium layer.
MediumSlab MediumLayerAt(Texture2D<float4> medium, SamplerState nearestSampler, int2 texel)
{
    if (MediumSteps.w == 0.0) {
        MediumSlab m;
        m.density = 0.0;
        m.bottom = 0.0;
        m.top = 0.0;
        m.kind = 0u;
        return m;
    }
    return DecodeMediumLayer(FetchTexel(medium, nearestSampler, texel, 1.0 / MediumSizes.xy), MediumSteps.y);
}

// --- column: optical depth from z0 upward ---------------------------------------------------------------------

// Optical depths along the view ray from the visible point at z0 to above all media, per component.
struct MediumColumn
{
    float tauLayer;     // medium layer: density * extinction over its slab above z0, / w.z
    float tauHaze;      // haze: integral of density * exp(-z / H) from z0 to infinity, / w.z
    float3 albedoLayer; // albedo of the layer's medium
};

MediumColumn MediumColumnAt(float z0, MediumSlab m)
{
    float4 e = MediumTable[m.kind];
    MediumColumn c;
    c.tauLayer = m.density * e.a * max(m.top - max(m.bottom, z0), 0.0) * MediumView.w;
    c.tauHaze = MediumHaze.x * MediumHaze.y * exp(-z0 / MediumHaze.y) * MediumView.w;
    c.albedoLayer = e.rgb;
    return c;
}

// T(z0 -> top) = exp(-integral of sigma).
float MediumTransmittance(MediumColumn c)
{
    return exp(-(c.tauLayer + c.tauHaze));
}

// Ambient light scattered toward the viewer along the column: ambient * albedo * (1 - T), the albedo of each
// component weighted by its share of the optical depth (exact for one component or proportional densities).
float3 MediumAmbientScatter(MediumColumn c, float t)
{
    float tau = c.tauLayer + c.tauHaze;
    if (tau <= 0.0) {
        return float3(0.0, 0.0, 0.0);
    }
    float3 albedo = (c.albedoLayer * c.tauLayer + MediumHazeColor.rgb * c.tauHaze) / tau;
    return MediumAmbient.rgb * albedo * (1.0 - t);
}

// --- scattering samples ---------------------------------------------------------------------------------------

// Static, world-anchored sample offset in [0, 1) (spec addendum item 4): the R3 low-discrepancy lattice (Roberts 2018,
// alpha_i = phi3^-i, phi3 the real root of x^4 = x + 1) over the world cell of the visible point p, cells of
// 1 / MediumHaze.z world units (about one scatter texel). Neighbouring cells get well-spread offsets (blue-noise-like),
// and a cell keeps its offset when the camera moves: panning by whole world units leaves the pattern on the world,
// nothing crawls. Fixed-point arithmetic modulo 2^32 keeps the offset exact for any cell index.
float MediumWorldNoise(float3 p)
{
    int3 cell = int3(floor(p * MediumHaze.z));
    uint n = uint(cell.x) * 3518319155u + uint(cell.y) * 2882110345u + uint(cell.z) * 2360945575u + 2147483648u;
    return float(n >> 8) * (1.0 / 16777216.0);
}

// --- phase function ---------------------------------------------------------------------------------------------

// Henyey-Greenstein phase function times 4 pi: 1 for g = 0 (isotropic, the unit the scattering had before), so the
// light's colour, haze factor and albedo keep their meaning. cosTheta = cos of the scattering angle between the light's
// travel (light -> sample) and the way to the viewer (w); g > 0 favours forward scattering. |g| <= 0.9 (validated)
// keeps the denominator >= 0.01.
float MediumPhaseHG(float g, float cosTheta)
{
    float d = 1.0 + g * g - 2.0 * g * cosTheta;
    return (1.0 - g * g) / (d * sqrt(d));
}

// g of entry `kind` of the media table.
float MediumAnisotropy(uint kind)
{
    return MediumPhase[kind >> 2][kind & 3u];
}

// Scattering toward the viewer and extinction at height h of a pixel's column for light arriving from direction
// toLight (unit, sample -> light; 0 counts as cos theta = 0): rgb sum over the layer's slab and the haze of
// sigma_s * phase, each component with its own g (so a mix weighs the phases by the components' sigma_s), a extinction.
float4 MediumScatteringAt(float h, MediumSlab m, float3 toLight)
{
    float cosTheta = -dot(toLight, MediumView.xyz);
    float4 e = MediumTable[m.kind];
    float layer = (h >= m.bottom && h <= m.top) ? m.density * e.a : 0.0;
    float haze = MediumHaze.x * exp(-h / MediumHaze.y);
    float3 s = layer * e.rgb * MediumPhaseHG(MediumAnisotropy(m.kind), cosTheta) +
               haze * MediumHazeColor.rgb * MediumPhaseHG(MediumHazeColor.a, cosTheta);
    return float4(s, layer + haze);
}

// --- pre-filter and upsample ----------------------------------------------------------------------------------
//
// Two steps (E106): the medium stage's filter pass (medium_filter.frag) runs a [1 2 1] x [1 2 1] joint bilateral
// pre-filter over the scatter buffer at its own resolution, which smooths the sample pattern (spec addendum item 4),
// and writes each texel's guide next to it; composite then upsamples the filtered buffer with 2 x 2 taps
// (MediumUpsample) or, with quality.medium_filter CHI_FILTER_NEAREST, takes one of them whole (MediumUpsampleNearest).

// Guide of a scene texel for the joint bilateral filters, in bytes: x surface height in surface height steps, y medium
// density * 255. Whole numbers, so the guide buffer (R8G8 UNORM, MediumGuideTexel) holds them exactly and a guide read
// from it equals the one computed at full resolution. Every texture is read through its own sampler: SDL_GPU binds
// texture and sampler as pairs.
float2 MediumGuide(Texture2D<float4> surface, SamplerState surfaceSampler, Texture2D<float4> medium,
                   SamplerState mediumSampler, int2 texel)
{
    return float2(round(MediumSurfaceZ(surface, surfaceSampler, texel) / MediumSteps.x),
                  round(MediumLayerAt(medium, mediumSampler, texel).density * 255.0));
}

// Squared guide distance: heights in units of 2 surface height steps, densities in units of 0.1 (25.5 bytes). A roof
// 5 steps above the ground gets exp(-6.25) of the weight of a matching tap.
float MediumGuideDistance(float2 a, float2 b)
{
    float2 d = (a - b) / float2(2.0, 25.5);
    return dot(d, d);
}

// The guide of a scatter texel from the guide buffer (R8G8 UNORM, bytes / 255).
float2 MediumGuideTexel(Texture2D<float4> guides, SamplerState guideSampler, int2 tap)
{
    return round(FetchTexel(guides, guideSampler, tap, 1.0 / MediumSizes.zw).rg * 255.0);
}

// The scatter buffer (rgb scattered light, a T) at scene position uv from the pre-filtered buffer: bilinear 2 x 2 taps,
// each also weighted by exp(-guide distance) to the pixel's guide (joint bilateral), so the upsample does not blur
// across a roof edge or a cloud's rim. If every tap differs strongly (a pixel whose taps all lie on another surface),
// the tap with the nearest guide.
float4 MediumUpsample(Texture2D<float4> filtered, SamplerState filteredSampler, Texture2D<float4> guides,
                      SamplerState guideSampler, Texture2D<float4> surface, SamplerState surfaceSampler,
                      Texture2D<float4> medium, SamplerState mediumSampler, float2 uv)
{
    float2 sceneSize = MediumSizes.xy;
    float2 scatterSize = MediumSizes.zw;
    int2 texel = clamp(int2(floor(uv * sceneSize)), int2(0, 0), int2(sceneSize) - 1);
    float2 guide = MediumGuide(surface, surfaceSampler, medium, mediumSampler, texel);

    float2 p = uv * scatterSize - 0.5;
    int2 i0 = int2(floor(p));
    float2 f = p - float2(i0);
    int2 maxTexel = int2(scatterSize) - 1;
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    float weight = 0.0;
    float4 nearest = float4(0.0, 0.0, 0.0, 1.0);
    float nearestDistance = 3.4e38;
    [unroll] for (int k = 0; k < 4; ++k) {
        int2 o = int2(k & 1, k >> 1);
        int2 tap = clamp(i0 + o, int2(0, 0), maxTexel);
        float4 v = FetchTexel(filtered, filteredSampler, tap, 1.0 / scatterSize);
        float d = MediumGuideDistance(guide, MediumGuideTexel(guides, guideSampler, tap));
        float2 b = float2(o.x != 0 ? f.x : 1.0 - f.x, o.y != 0 ? f.y : 1.0 - f.y);
        float w = b.x * b.y * exp(-d);
        sum += v * w;
        weight += w;
        if (d < nearestDistance) {
            nearestDistance = d;
            nearest = v;
        }
    }
    return weight > 1e-3 ? sum / weight : nearest;
}

// Blocky upsample (quality.medium_filter CHI_FILTER_NEAREST): the whole filtered texel the pixel lies in, unless
// another of the 2 x 2 texels around it has a nearer guide (the pixel is on a roof whose texel was computed on the
// ground, or the other way round); then that one. Hard steps of one scatter texel, edge-aware like the smooth upsample.
float4 MediumUpsampleNearest(Texture2D<float4> filtered, SamplerState filteredSampler, Texture2D<float4> guides,
                             SamplerState guideSampler, Texture2D<float4> surface, SamplerState surfaceSampler,
                             Texture2D<float4> medium, SamplerState mediumSampler, float2 uv)
{
    float2 sceneSize = MediumSizes.xy;
    float2 scatterSize = MediumSizes.zw;
    int2 texel = clamp(int2(floor(uv * sceneSize)), int2(0, 0), int2(sceneSize) - 1);
    float2 guide = MediumGuide(surface, surfaceSampler, medium, mediumSampler, texel);
    int2 maxTexel = int2(scatterSize) - 1;
    int2 own = clamp(int2(floor(uv * scatterSize)), int2(0, 0), maxTexel);
    int2 best = own;
    float bestDistance = MediumGuideDistance(guide, MediumGuideTexel(guides, guideSampler, own));
    if (bestDistance > 0.0) {
        // The other three texels of the 2 x 2 around the pixel: toward the nearer neighbour on each axis.
        float2 p = uv * scatterSize;
        int2 toward = int2(p.x - float(own.x) < 0.5 ? -1 : 1, p.y - float(own.y) < 0.5 ? -1 : 1);
        [unroll] for (int k = 1; k < 4; ++k) {
            int2 tap = clamp(own + int2(k & 1, k >> 1) * toward, int2(0, 0), maxTexel);
            float d = MediumGuideDistance(guide, MediumGuideTexel(guides, guideSampler, tap));
            if (d < bestDistance) {
                bestDistance = d;
                best = tap;
            }
        }
    }
    return FetchTexel(filtered, filteredSampler, best, 1.0 / scatterSize);
}

#endif
