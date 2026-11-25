// P30 reference (Knorke Chiaro port): renders four scenes with haze and media through C++ Chiaro (9bc5b9b, C API)
// and writes the medium stage's buffers and the final image as raw bytes.
//   medium_stage_ref <outdir> [frames]
// With `frames` each scene is also timed for that many more frames (wall clock per frame, CHI_Render + submit +
// wait), printed as median and p95; the Rust side is KNORKE_SCENE_BENCH of gpu_medium.rs.
// Writes <outdir>/<scene>.<buffer>.bin:
//  - scatter (RGBA16F, before the pre-filter), filtered (RGBA16F, what composite reads), guides (RG8),
//    polar (RGBA16F, the light shaft atlas; only with shaft rows), sun (R16F, the sun height atlas; only with
//    blocks), final (RGBA8 target, default output), and for `smoke` also layer (RGBA8, the medium layer drawn
//    with CHI_DrawMedium on GPU layers).
// Scenes (96 x 64, the checker of ../ring_light/ring_ref.c, ambient 0.1 unless noted):
//  - shafts: isometric with a fractional origin, shadow and medium resolution 0.5, haze 0.15, a box obstacle
//    (height 8), a shadowed point light beside it and a shadowed spot light (light shafts, two polar rows), and
//    the medium layer of medium_ref.cpp scaled to 96 x 64.
//  - sun: the same projection, haze 0.2, two boxes (heights 8 and 4), a shadowed directional light (a block in the
//    sun height atlas) and an unshadowed one.
//  - ring: identity projection, haze 0.12, an ambient zone, an unshadowed point light and a shockwave ring light:
//    Chiaro's dual path (the zone pass keeps the pure ambient for the medium base pass).
//  - smoke: isometric, no haze, a box and a shadowed point light; the medium layer holds a flat slab of steam and
//    two shaped puffs of smoke (a radial alpha texture), drawn with CHI_DrawMedium on GPU layers.
// The context internals are read through the headers (#define private public, Inspect); the context itself is the
// DLL's. Build with build.bat next to this file's build directory (see the P29 build of medium_ref.cpp); captured
// 2026-11-25 on an AMD Radeon RX 6700 XT (Vulkan). The scenes are mirrored in
// crates/knorke-integration/tests/gpu_medium.rs; change both together.

#define _ALLOW_KEYWORD_MACROS
#define private public
#include "core/Context.hpp"
#include "core/ContextInspect.hpp"
#undef private
#include "GpuTestHarness.hpp"

#include <Chiaro/Chiaro.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <string>
#include <algorithm>
#include <vector>

using namespace chiaro::test;
using chiaro::detail::Inspect;

namespace {

int g_bench = 0; // frames to time per scene (0: none)

constexpr Uint32 kW = 96, kH = 64;
constexpr SDL_GPUTextureFormat kRgba = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
const CHI_Projection kIso{{48.3f, 10.6f}, {1.6f, 0.8f}, {-1.6f, 0.8f}, {0.0f, -1.2f}};

void Check(bool ok, const char *what)
{
    if (!ok) {
        std::fprintf(stderr, "%s failed: %s\n", what, SDL_GetError());
        std::abort();
    }
}

std::vector<Uint8> Checker()
{
    std::vector<Uint8> px(size_t{kW} * kH * 4);
    for (Uint32 y = 0; y < kH; ++y) {
        for (Uint32 x = 0; x < kW; ++x) {
            Uint8 *c = &px[(size_t{y} * kW + x) * 4];
            const int checker = ((x / 8) + (y / 8)) & 1 ? 12 : -12;
            c[0] = static_cast<Uint8>(110 + checker);
            c[1] = static_cast<Uint8>(120 + checker);
            c[2] = static_cast<Uint8>(130 + checker);
            c[3] = 255;
        }
    }
    return px;
}

/** medium_ref.cpp's two clouds (kind 0 around (30, 20), kind 3 around (66, 26)) on 96 x 64. */
std::vector<Uint8> MediumLayer()
{
    std::vector<Uint8> px(size_t{kW} * kH * 4, 0);
    for (int y = 0; y < static_cast<int>(kH); ++y) {
        for (int x = 0; x < static_cast<int>(kW); ++x) {
            Uint8 *p = &px[(static_cast<size_t>(y) * kW + static_cast<size_t>(x)) * 4];
            const int ax = x - 30, ay = y - 20, a = ax * ax + ay * ay;
            const int bx = x - 66, by = y - 26, b = bx * bx + by * by;
            if (a < 196) {
                p[0] = 12;
                p[1] = 255 - 2;
                p[2] = 0;
                p[3] = static_cast<Uint8>(255 - a * 255 / 196);
            } else if (b < 100) {
                p[0] = 30;
                p[1] = 255 - 10;
                p[2] = 3 * 16;
                p[3] = static_cast<Uint8>(200 - b * 2);
            }
        }
    }
    return px;
}

void Light(CHI_Context *fx, CHI_LightType type, float x, float y, float height, float radius, CHI_Vec3 direction,
           float intensity, bool shadows)
{
    CHI_LightDesc d;
    CHI_InitLightDesc(&d, type);
    d.x = x;
    d.y = y;
    d.height = height;
    d.radius = radius;
    d.direction = direction;
    d.intensity = intensity;
    d.inner_angle = 25;
    d.outer_angle = 45;
    if (shadows) {
        d.flags &= ~CHI_LIGHT_NO_SHADOWS;
    } else {
        d.flags |= CHI_LIGHT_NO_SHADOWS;
    }
    Check(CHI_CreateLight(fx, &d).id != 0, "light");
}

void Box(CHI_Context *fx, float x, float y, float w, float d, float height)
{
    CHI_ShapeDesc s;
    CHI_InitShapeDesc(&s, CHI_SHAPE_BOX);
    s.x = x;
    s.y = y;
    s.width = w;
    s.depth = d;
    s.height = height;
    Check(CHI_CreateShape(fx, &s).id != 0, "box");
}

void Settings(CHI_Context *fx, float haze, bool half)
{
    Check(EditSettings(fx,
                       [&](CHI_Settings &s) {
                           s.ambient = {0.1f, 0.1f, 0.1f};
                           s.haze.density = haze;
                           if (half) {
                               s.quality.preset = CHI_QUALITY_CUSTOM;
                               s.quality.light_resolution = 1.0f;
                               s.quality.shadow_resolution = 0.5f;
                               s.quality.medium_resolution = 0.5f;
                           }
                       }),
          "settings");
}

void Dump(const std::string &path, const std::vector<Uint8> &bytes)
{
    FILE *f = std::fopen(path.c_str(), "wb");
    if (!f) {
        std::abort();
    }
    std::fwrite(bytes.data(), 1, bytes.size(), f);
    std::fclose(f);
    std::printf("%s: %zu bytes\n", path.c_str(), bytes.size());
}

void DumpTarget(const std::string &path, const chiaro::detail::RenderTarget &t, Uint32 bpp)
{
    if (t.Texture()) {
        Dump(path, DownloadBytes(t.Texture(), t.Width(), t.Height(), bpp));
    }
}

/** Renders one frame of `fx` (projection `p`, optional medium layer) and writes the buffers of `name`. */
void Render(const std::string &out, const char *name, CHI_Context *fx, const CHI_Projection &p,
            SDL_GPUTexture *medium)
{
    SDL_GPUTexture *scene = CreateTestTexture(kW, kH, kRgba, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    SDL_GPUTexture *target = CreateTestTexture(kW, kH, kRgba, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET |
                                                                   SDL_GPU_TEXTUREUSAGE_SAMPLER);
    UploadPixels(scene, kW, kH, Checker());
    CHI_Frame info{};
    info.scene = CHI_FromGPU(scene, kRgba, kW, kH);
    info.target = CHI_FromGPU(target, kRgba, kW, kH);
    if (medium) {
        info.buffers.medium = CHI_FromGPU(medium, kRgba, kW, kH);
    }
    info.projection = &p;
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(TestDevice());
    Check(CHI_Render(fx, cmd, &info), "render");
    SubmitAndWait(cmd);
    const auto &m = fx->impl->m_medium;
    const std::string base = out + "/" + name;
    DumpTarget(base + ".scatter.bin", m.m_scatter, 8);
    DumpTarget(base + ".filtered.bin", m.m_filtered, 8);
    DumpTarget(base + ".guides.bin", m.m_guides, 2);
    if (!m.m_rows.empty()) {
        DumpTarget(base + ".polar.bin", m.m_polar, 8);
    }
    if (!m.m_sunBlocks.empty()) {
        DumpTarget(base + ".sun.bin", m.m_sunHeights, 2);
    }
    std::printf("%s: %u medium lights, %zu shaft rows, %zu sun blocks\n", name, m.m_lightCount, m.m_rows.size(),
                m.m_sunBlocks.size());
    Dump(base + ".final.bin", DownloadPixels(target, kW, kH));
    if (g_bench > 0) {
        std::vector<double> us;
        for (int i = 0; i < g_bench; ++i) {
            const Uint64 t0 = SDL_GetTicksNS();
            SDL_GPUCommandBuffer *c = SDL_AcquireGPUCommandBuffer(TestDevice());
            Check(CHI_Render(fx, c, &info), "render");
            SubmitAndWait(c);
            us.push_back(static_cast<double>(SDL_GetTicksNS() - t0) / 1000.0);
        }
        std::sort(us.begin(), us.end());
        std::printf("bench %s (C++), %d frames: median %.1f us, p95 %.1f us\n", name, g_bench, us[us.size() / 2],
                    us[us.size() * 95 / 100]);
    }
    ReleaseTestTexture(scene);
    ReleaseTestTexture(target);
}

const CHI_Vec3 kMoon{-0.7f, 0.15f, -0.7f};

void Shafts(const std::string &out)
{
    CHI_Context *fx = CHI_CreateContext(TestDevice(), CHI_VERSION);
    Settings(fx, 0.15f, true);
    Box(fx, 14.0f, 8.0f, 4.0f, 4.0f, 8.0f);
    Light(fx, CHI_LIGHT_POINT, 10.0f, 12.0f, 4.0f, 30.0f, {1.0f, 0.0f, 0.0f}, 2.0f, true);
    Light(fx, CHI_LIGHT_SPOT, 6.0f, 2.0f, 10.0f, 35.0f, {1.0f, 0.4f, -0.5f}, 2.0f, true);
    SDL_GPUTexture *medium = CreateTestTexture(kW, kH, kRgba, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    UploadPixels(medium, kW, kH, MediumLayer());
    Render(out, "shafts", fx, kIso, medium);
    ReleaseTestTexture(medium);
    CHI_DestroyContext(fx);
}

void Sun(const std::string &out)
{
    CHI_Context *fx = CHI_CreateContext(TestDevice(), CHI_VERSION);
    Settings(fx, 0.2f, true);
    Box(fx, 14.0f, 8.0f, 4.0f, 4.0f, 8.0f);
    Box(fx, 24.0f, 4.0f, 6.0f, 3.0f, 4.0f);
    Light(fx, CHI_LIGHT_DIRECTIONAL, 0.0f, 0.0f, CHI_HEIGHT_AUTO, 30.0f, kMoon, 1.0f, true);
    Light(fx, CHI_LIGHT_DIRECTIONAL, 0.0f, 0.0f, CHI_HEIGHT_AUTO, 30.0f, {0.3f, -0.5f, -0.8f}, 0.4f, false);
    Render(out, "sun", fx, kIso, nullptr);
    CHI_DestroyContext(fx);
}

void Ring(const std::string &out)
{
    CHI_Context *fx = CHI_CreateContext(TestDevice(), CHI_VERSION);
    Settings(fx, 0.12f, false);
    CHI_AmbientZoneDesc z;
    CHI_AmbientBox(&z, 60.0f, 30.0f, 40.0f, 30.0f, CHI_Color{0.4f, 0.3f, 0.2f});
    z.soft_edge = 4.0f;
    Check(CHI_CreateAmbientZone(fx, &z).id != 0, "zone");
    Light(fx, CHI_LIGHT_POINT, 30.0f, 30.0f, 10.0f, 50.0f, {1.0f, 0.0f, 0.0f}, 1.5f, false);
    CHI_ShockwaveDesc w;
    CHI_InitShockwaveDesc(&w);
    w.x = 60.0f;
    w.y = 32.0f;
    w.radius = 14.0f;
    w.width = 6.0f;
    w.strength = 0.0f;
    w.light = CHI_Color{1.0f, 0.6f, 0.3f};
    w.light_intensity = 2.0f;
    Check(CHI_SetFrameShockwaves(fx, &w, 1), "shockwaves");
    const CHI_Projection p = IdentityProjection();
    Render(out, "ring", fx, p, nullptr);
    CHI_DestroyContext(fx);
}

/** A 16 x 16 puff: alpha 1 - r over the radius (CHI_MediumDraw::texture), colour that must not matter. */
std::vector<Uint8> Puff()
{
    std::vector<Uint8> px(16 * 16 * 4);
    for (int y = 0; y < 16; ++y) {
        for (int x = 0; x < 16; ++x) {
            const float dx = (static_cast<float>(x) + 0.5f) / 8.0f - 1.0f;
            const float dy = (static_cast<float>(y) + 0.5f) / 8.0f - 1.0f;
            const float r = std::sqrt(dx * dx + dy * dy);
            Uint8 *c = &px[static_cast<size_t>(y * 16 + x) * 4];
            c[0] = 10;
            c[1] = 200;
            c[2] = 30;
            c[3] = static_cast<Uint8>(r >= 1.0f ? 0 : static_cast<int>((1.0f - r) * 255.0f + 0.5f));
        }
    }
    return px;
}

CHI_MediumDraw Draw(SDL_FRect rect, float density, float bottom, float top, Uint32 medium, CHI_Texture *texture)
{
    CHI_MediumDraw d{};
    d.texture = texture;
    d.rect = rect;
    d.density = density;
    d.bottom = bottom;
    d.top = top;
    d.medium = medium;
    return d;
}

void Smoke(const std::string &out)
{
    CHI_Context *fx = CHI_CreateContext(TestDevice(), CHI_VERSION);
    Settings(fx, 0.0f, true);
    Box(fx, 14.0f, 8.0f, 4.0f, 4.0f, 8.0f);
    Light(fx, CHI_LIGHT_POINT, 10.0f, 12.0f, 6.0f, 30.0f, {1.0f, 0.0f, 0.0f}, 2.0f, true);
    CHI_Layers *layers = CHI_CreateGPULayers(fx, kW, kH, CHI_LAYER_SCENE | CHI_LAYER_MEDIUM);
    Check(layers != nullptr, "layers");
    SDL_GPUTexture *puff = CreateTestTexture(16, 16, kRgba, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    UploadPixels(puff, 16, 16, Puff());
    CHI_Texture shape = CHI_FromGPU(puff, kRgba, 16, 16);
    // The first pass of the frame clears the medium layer (CHI_GetLayerColorTargets).
    SDL_GPUColorTargetInfo infos[8];
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(TestDevice());
    const Uint32 n = CHI_GetLayerColorTargets(layers, CHI_LAYER_MEDIUM, infos, 8);
    Check(n == 1, "layer targets");
    SDL_EndGPURenderPass(SDL_BeginGPURenderPass(cmd, infos, n, nullptr));
    SubmitAndWait(cmd);
    const CHI_MediumDraw draws[3] = {
        Draw({40.0f, 30.0f, 40.0f, 14.0f}, 0.4f, 0.0f, 6.0f, CHI_MEDIUM_STEAM, nullptr),
        Draw({20.0f, 10.0f, 24.0f, 24.0f}, 0.8f, 2.0f, 30.0f, CHI_MEDIUM_SMOKE, &shape),
        Draw({30.5f, 18.25f, 17.0f, 13.0f}, 0.5f, 6.0f, 20.0f, CHI_MEDIUM_SMOKE, &shape),
    };
    Check(CHI_DrawMediums(layers, draws, 3), "draw mediums");
    SDL_GPUTexture *medium = CHI_GetLayerTexture(layers, CHI_LAYER_MEDIUM).texture;
    Dump(out + "/smoke.layer.bin", DownloadPixels(medium, kW, kH));
    Render(out, "smoke", fx, kIso, medium);
    ReleaseTestTexture(puff);
    CHI_DestroyLayers(layers);
    CHI_DestroyContext(fx);
}

} // namespace

int main(int argc, char **argv)
{
    if (argc < 2 || !InitTestDevice()) {
        std::fprintf(stderr, "usage: medium_stage_ref <outdir> (CHIARO_TEST_GPU_DRIVER=vulkan)\n");
        return 2;
    }
    const std::string out = argv[1];
    g_bench = argc > 2 ? std::atoi(argv[2]) : 0;
    Shafts(out);
    Sun(out);
    Ring(out);
    Smoke(out);
    return 0;
}
