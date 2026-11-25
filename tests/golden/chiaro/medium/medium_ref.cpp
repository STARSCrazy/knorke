// P29 reference (Knorke Chiaro port): renders two scenes with a medium layer with C++ Chiaro (9bc5b9b) and writes the
// medium world map (RGBA16F, MediumMapStage) and the light buffer (RGBA16F, the `_medium` light programs) as raw
// bytes.
//   medium_ref <outdir>
// Writes <outdir>/<scene>.map.bin and <scene>.light.bin for scene in top, iso.
//  - top: identity projection (z axis 0), no obstacles (the map computes its own grid), an unshadowed point light
//    and an unshadowed directional light (drawn per pixel because of the map).
//  - iso: an isometric projection with a fractional origin (anchor remainder, slab heights shift the splat), shadow
//    resolution 0.5 (3 taps per block side), a box obstacle with a finite height (the map takes the occlusion
//    stage's grid), a shadowed spot light and a shadowed directional light.
// The medium layer is the same in both (MediumLayer below, integer formulas): two clouds of different kinds, heights
// and densities. Chiaro also runs its medium stage (P30) on these frames; it does not touch the dumped buffers.
// Build and run like ../occluder/mask_ref.cpp (the same command line with medium_ref.cpp). Captured 2026-11-25 on an
// AMD Radeon RX 6700 XT (Vulkan). The scenes are mirrored in crates/knorke-light2d/tests/gpu_medium_map.rs
// (`scene_top`, `scene_iso`, `medium_layer`); change both together.

#include "GpuTestHarness.hpp"
#include "core/Context.hpp"
#include "core/ContextInspect.hpp"

#include <Chiaro/Chiaro.h>

#include <cstdio>
#include <cstdlib>
#include <memory>
#include <string>
#include <vector>

using namespace chiaro::test;
using chiaro::detail::Context;
using chiaro::detail::Inspect;

namespace {

constexpr Uint32 kW = 96, kH = 48;

std::unique_ptr<Context> ctx;
CHI_Projection projection{};

void Fail(const char *what)
{
    std::fprintf(stderr, "%s failed: %s\n", what, SDL_GetError());
    std::abort();
}

/** Two clouds: kind 0 from 2 to 12 steps around (30, 20), kind 3 from 10 to 30 steps around (66, 26). */
std::vector<Uint8> MediumLayer()
{
    std::vector<Uint8> px(static_cast<size_t>(kW) * kH * 4, 0);
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

void Light(CHI_LightType type, float x, float y, float height, float radius, CHI_Vec3 direction, float intensity,
           bool shadows)
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
    if (ctx->Lights().Create(d) == 0) {
        Fail("light");
    }
}

void Settings(float shadowResolution)
{
    if (!EditSettings(ctx->GetSettings(), [&](CHI_Settings &s) {
            s.ambient = {0.03f, 0.03f, 0.04f};
            if (shadowResolution < 1.0f) {
                s.quality.preset = CHI_QUALITY_CUSTOM;
                s.quality.light_resolution = 1.0f;
                s.quality.shadow_resolution = shadowResolution;
                s.quality.medium_resolution = 0.5f;
            }
        })) {
        std::abort();
    }
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

void Render(const std::string &out, const char *name)
{
    SDL_GPUTexture *scene = CreateTestTexture(kW, kH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    SDL_GPUTexture *medium = CreateTestTexture(kW, kH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    SDL_GPUTexture *target = CreateTestTexture(kW, kH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                                               SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER);
    UploadPixels(scene, kW, kH, std::vector<Uint8>(static_cast<size_t>(kW) * kH * 4, 255));
    UploadPixels(medium, kW, kH, MediumLayer());
    CHI_Frame info{};
    info.scene = CHI_Texture{scene, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, kW, kH};
    info.target = CHI_Texture{target, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, kW, kH};
    info.buffers.medium = CHI_Texture{medium, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, kW, kH};
    info.projection = &projection;
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(TestDevice());
    if (!ctx->Render(cmd, info)) {
        Fail("render");
    }
    SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
    SDL_WaitForGPUFences(TestDevice(), true, &fence, 1);
    SDL_ReleaseGPUFence(TestDevice(), fence);
    const auto inspect = Inspect(*ctx);
    const auto map = inspect.MediumMap();
    const auto light = inspect.LightBuffer();
    if (!map.texture || !light.texture) {
        std::abort();
    }
    std::printf("%s: map %u x %u, light %u x %u\n", name, map.width, map.height, light.width, light.height);
    Dump(out + "/" + name + ".map.bin", DownloadBytes(map.texture, map.width, map.height, 8));
    Dump(out + "/" + name + ".light.bin", DownloadBytes(light.texture, light.width, light.height, 8));
    ReleaseTestTexture(scene);
    ReleaseTestTexture(medium);
    ReleaseTestTexture(target);
}

/** Identity projection, no obstacles, unshadowed lights. */
void SceneTop()
{
    projection = IdentityProjection();
    Settings(1.0f);
    Light(CHI_LIGHT_POINT, 48.0f, 24.0f, 20.0f, 60.0f, {1.0f, 0.0f, 0.0f}, 1.5f, false);
    Light(CHI_LIGHT_DIRECTIONAL, 0.0f, 0.0f, CHI_HEIGHT_AUTO, 30.0f, {-0.7f, 0.15f, -0.7f}, 0.5f, false);
}

/** Isometric, shadow resolution 0.5, a box, shadowed spot and directional light. */
void SceneIso()
{
    projection = CHI_Projection{{48.3f, 10.6f}, {1.6f, 0.8f}, {-1.6f, 0.8f}, {0.0f, -1.2f}};
    Settings(0.5f);
    CHI_ShapeDesc s;
    CHI_InitShapeDesc(&s, CHI_SHAPE_BOX);
    s.x = 14.0f;
    s.y = 8.0f;
    s.width = 4.0f;
    s.depth = 4.0f;
    s.height = 8.0f;
    if (ctx->Obstacles().CreateShape(s) == 0) {
        Fail("box");
    }
    Light(CHI_LIGHT_SPOT, 8.0f, 16.0f, 12.0f, 40.0f, {1.0f, -0.2f, -0.4f}, 1.5f, true);
    Light(CHI_LIGHT_DIRECTIONAL, 0.0f, 0.0f, CHI_HEIGHT_AUTO, 30.0f, {-0.7f, 0.15f, -0.7f}, 0.5f, true);
}

} // namespace

int main(int argc, char **argv)
{
    if (argc < 2 || !InitTestDevice()) {
        std::fprintf(stderr, "usage: medium_ref <outdir> (CHIARO_TEST_GPU_DRIVER=vulkan)\n");
        return 2;
    }
    const std::string out = argv[1];
    const auto run = [&](const char *name, void (*scene)()) {
        ctx = Context::Create(TestDevice());
        scene();
        Render(out, name);
        ctx.reset();
    };
    run("top", SceneTop);
    run("iso", SceneIso);
    return 0;
}
