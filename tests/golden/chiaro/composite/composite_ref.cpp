// P27 reference (Knorke Chiaro port): renders seven composite scenes with C++ Chiaro (9bc5b9b) and writes the HDR buffer
// (composite output, RGBA16F) as raw bytes.
//   composite_ref <outdir>
// Writes <outdir>/<scene>.hdr.bin for scene in plain, glow, glow_scene, exact, direction, ambient, iso. All scenes are
// 96x48, RGBA8 UNORM scene (sRGB-encoded gradient with a checker blue channel), no obstacles, so no shadows:
//   plain      two point lights and a spot light, no glow, no material buffer;
//   glow       the same with a glow buffer (colour from the buffer, glow scale 2);
//   glow_scene the same with the glow colour taken from the scene pixel (CHI_GLOW_COLOR_SCENE, scale 1.5);
//   exact      the same lights on a material buffer (dome normal map, satin), light buffer at scene size: the exact
//              material mode with the specular buffer, plus glow;
//   direction  the same lights, light buffer at half size: the direction material mode (no surface buffer), plus glow;
//   ambient    a directional light only: no light pass, the composite multiplies by the ambient.
//   iso        three other lights under an isometric projection (origin 48/24, x 2/1, y -2/1, z 0/-2) on a material
//              buffer and a surface buffer (a plateau, a tilted ramp), light buffer at half size: the direction
//              material mode with the base normals from the surface buffer, wet ground (wetness 0.6), plus glow.
// Build and run like ../light/light_ref.cpp (the same command line with composite_ref.cpp). Captured 2026-11-24 on an
// AMD Radeon RX 6700 XT (Vulkan). The scenes are mirrored in crates/knorke-light2d/tests/gpu_composite_stage.rs
// (`scene_*`); change both together.

#include "GpuTestHarness.hpp"
#include "core/Context.hpp"
#include "core/ContextInspect.hpp"

#include <Chiaro/Chiaro.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

using namespace chiaro::test;
using chiaro::detail::Context;
using chiaro::detail::Inspect;

namespace {

constexpr Uint32 kW = 96, kH = 48;

std::unique_ptr<Context> ctx;

struct Light
{
    CHI_LightType type = CHI_LIGHT_POINT;
    float x = 0, y = 0, height = CHI_HEIGHT_AUTO, radius = 30;
    CHI_Color color{1, 1, 1};
    float intensity = 1, falloff = 2;
    CHI_Vec3 direction{1, 0, 0};
    float inner = 20, outer = 30;
};

void Add(const Light &l)
{
    CHI_LightDesc d;
    CHI_InitLightDesc(&d, l.type);
    d.x = l.x;
    d.y = l.y;
    d.height = l.height;
    d.radius = l.radius;
    d.color = l.color;
    d.intensity = l.intensity;
    d.falloff = l.falloff;
    d.direction = l.direction;
    d.inner_angle = l.inner;
    d.outer_angle = l.outer;
    d.flags |= CHI_LIGHT_NO_SHADOWS;
    if (ctx->Lights().Create(d) == 0) {
        std::fprintf(stderr, "light rejected: %s\n", SDL_GetError());
        std::abort();
    }
}

struct Config
{
    float ambient = 0.05f;
    float lightResolution = 1.0f;
    float glowScale = 1.0f;
    CHI_GlowColor glowColor = CHI_GLOW_COLOR_BUFFER;
    float wetness = 0.0f;
};

void Configure(const Config &c)
{
    const bool ok = EditSettings(ctx->GetSettings(), [&](CHI_Settings &s) {
        s.ambient = {c.ambient, c.ambient * 1.2f, c.ambient * 1.4f};
        s.glow.scale = c.glowScale;
        s.glow.color = c.glowColor;
        s.material.wetness = c.wetness;
        if (c.lightResolution < 1.0f) {
            s.quality.preset = CHI_QUALITY_CUSTOM;
            s.quality.light_resolution = c.lightResolution;
            s.quality.shadow_resolution = 1.0f;
        }
    });
    if (!ok) {
        std::abort();
    }
}

/** sRGB-encoded gradient with a checker blue channel. */
std::vector<Uint8> ScenePixels()
{
    std::vector<Uint8> px(static_cast<size_t>(kW) * kH * 4);
    for (Uint32 y = 0; y < kH; ++y) {
        for (Uint32 x = 0; x < kW; ++x) {
            const size_t p = (static_cast<size_t>(y) * kW + x) * 4;
            px[p + 0] = static_cast<Uint8>(x * 255 / (kW - 1));
            px[p + 1] = static_cast<Uint8>(y * 255 / (kH - 1));
            px[p + 2] = (((x / 8) + (y / 8)) & 1) ? 200 : 60;
            px[p + 3] = 255;
        }
    }
    return px;
}

/** Glow: orange in a rectangle, strength rising by column; zero elsewhere. */
std::vector<Uint8> GlowPixels()
{
    std::vector<Uint8> px(static_cast<size_t>(kW) * kH * 4, 0);
    for (Uint32 y = 10; y < 30; ++y) {
        for (Uint32 x = 50; x < 90; ++x) {
            const size_t p = (static_cast<size_t>(y) * kW + x) * 4;
            px[p + 0] = 255;
            px[p + 1] = 140;
            px[p + 2] = 40;
            px[p + 3] = static_cast<Uint8>(40 + (x - 50) * 5);
        }
    }
    return px;
}

/** Surface buffer: ground (height 0, up), a plateau of height 6 and a tilted ramp of height 3. */
std::vector<Uint8> Surface()
{
    std::vector<Uint8> px(static_cast<size_t>(kW) * kH * 4);
    for (Uint32 y = 0; y < kH; ++y) {
        for (Uint32 x = 0; x < kW; ++x) {
            float height = 0.0f;
            CHI_Vec3 normal{0, 0, 1};
            if (x >= 30 && x < 70 && y >= 12 && y < 36) {
                height = 6.0f;
            } else if (x >= 70 && x < 90 && y >= 12 && y < 36) {
                height = 3.0f;
                normal = {0.4f, 0.0f, 0.9f};
            }
            const Uint32 pixel = CHI_SurfacePixel(height, 1.0f, normal);
            const size_t p = (static_cast<size_t>(y) * kW + x) * 4;
            std::memcpy(&px[p], &pixel, 4);
        }
    }
    return px;
}

/** A dome normal map (radius 20 around the centre) on a flat ground, converted to material pixels (satin). */
std::vector<Uint8> Material()
{
    std::vector<Uint8> normal(static_cast<size_t>(kW) * kH * 4), color(normal.size(), 255), out(normal.size());
    for (Uint32 y = 0; y < kH; ++y) {
        for (Uint32 x = 0; x < kW; ++x) {
            const float dx = (static_cast<float>(x) + 0.5f - 48.0f) / 20.0f;
            const float dy = (static_cast<float>(y) + 0.5f - 24.0f) / 20.0f;
            const float d2 = dx * dx + dy * dy;
            const float n[3] = {d2 < 1.0f ? dx : 0.0f, d2 < 1.0f ? -dy : 0.0f, d2 < 1.0f ? std::sqrt(1.0f - d2) : 1.0f};
            const size_t p = (static_cast<size_t>(y) * kW + x) * 4;
            for (int i = 0; i < 3; ++i) {
                normal[p + i] = static_cast<Uint8>(SDL_lroundf((n[i] * 0.5f + 0.5f) * 255.0f));
            }
            normal[p + 3] = 255;
        }
    }
    CHI_NormalMapOptions o;
    CHI_InitNormalMapOptions(&o);
    o.material = CHI_GetMaterialPreset(CHI_MATERIAL_SATIN);
    o.alpha_rgba = color.data();
    o.alpha_pitch = static_cast<int>(kW * 4);
    if (!CHI_ConvertNormalMap(normal.data(), kW, kH, static_cast<int>(kW * 4), &o, out.data())) {
        std::abort();
    }
    return out;
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

void Render(const std::string &out, const char *name, bool glow, bool material, bool iso)
{
    constexpr auto kFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
    SDL_GPUTexture *scene = CreateTestTexture(kW, kH, kFormat, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    SDL_GPUTexture *target = CreateTestTexture(kW, kH, kFormat,
                                               SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER);
    SDL_GPUTexture *mat = nullptr;
    SDL_GPUTexture *glowTex = nullptr;
    SDL_GPUTexture *surfaceTex = nullptr;
    UploadPixels(scene, kW, kH, ScenePixels());
    CHI_Projection projection = IdentityProjection();
    if (iso) {
        projection = CHI_Projection{{48.0f, 24.0f}, {2.0f, 1.0f}, {-2.0f, 1.0f}, {0.0f, -2.0f}};
    }
    CHI_Frame info{};
    info.scene = CHI_Texture{scene, nullptr, kFormat, kW, kH};
    info.target = CHI_Texture{target, nullptr, kFormat, kW, kH};
    info.projection = &projection;
    if (material) {
        mat = CreateTestTexture(kW, kH, kFormat, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        UploadPixels(mat, kW, kH, Material());
        info.buffers.material = CHI_Texture{mat, nullptr, kFormat, kW, kH};
    }
    if (iso) {
        surfaceTex = CreateTestTexture(kW, kH, kFormat, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        UploadPixels(surfaceTex, kW, kH, Surface());
        info.buffers.surface = CHI_Texture{surfaceTex, nullptr, kFormat, kW, kH};
    }
    if (glow) {
        glowTex = CreateTestTexture(kW, kH, kFormat, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        UploadPixels(glowTex, kW, kH, GlowPixels());
        info.buffers.glow = CHI_Texture{glowTex, nullptr, kFormat, kW, kH};
    }
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(TestDevice());
    if (!ctx->Render(cmd, info)) {
        std::fprintf(stderr, "render failed: %s\n", SDL_GetError());
        std::abort();
    }
    SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
    SDL_WaitForGPUFences(TestDevice(), true, &fence, 1);
    SDL_ReleaseGPUFence(TestDevice(), fence);
    const auto hdr = Inspect(*ctx).Hdr();
    if (!hdr.texture) {
        std::fprintf(stderr, "%s: no HDR buffer\n", name);
        std::abort();
    }
    Dump(out + "/" + name + ".hdr.bin", DownloadBytes(hdr.texture, hdr.width, hdr.height, 8));
    ReleaseTestTexture(scene);
    ReleaseTestTexture(target);
    if (mat) {
        ReleaseTestTexture(mat);
    }
    if (glowTex) {
        ReleaseTestTexture(glowTex);
    }
    if (surfaceTex) {
        ReleaseTestTexture(surfaceTex);
    }
}

/** Two point lights and a spot light, no obstacles, no shadows. */
void Lights()
{
    Add({.x = 20, .y = 24, .radius = 40, .color = {1.0f, 0.8f, 0.6f}, .intensity = 1.2f});
    Add({.x = 60, .y = 10, .height = 8, .radius = 35, .falloff = 1.5f});
    Add({.type = CHI_LIGHT_SPOT, .x = 84, .y = 38, .height = 6, .radius = 45, .color = {0.5f, 0.7f, 1.0f},
         .direction = {-1.0f, -0.5f, -0.3f}, .inner = 20, .outer = 35});
}

/** Three lights for the isometric scene (world units; one world unit is about two scene pixels). */
void IsoLights()
{
    Add({.x = -4, .y = 2, .height = 5, .radius = 24, .color = {1.0f, 0.8f, 0.6f}, .intensity = 1.2f});
    Add({.x = 8, .y = 4, .radius = 18, .falloff = 1.5f});
    Add({.type = CHI_LIGHT_SPOT, .x = 4, .y = -8, .height = 6, .radius = 26, .color = {0.5f, 0.7f, 1.0f},
         .direction = {-0.3f, 0.8f, -0.5f}, .inner = 15, .outer = 35});
}

/** A directional light only: it folds into the ambient, there is no light pass. */
void Directional()
{
    Add({.type = CHI_LIGHT_DIRECTIONAL, .intensity = 0.3f, .direction = {0.3f, 0.4f, -1.0f}});
}

} // namespace

int main(int argc, char **argv)
{
    if (argc < 2 || !InitTestDevice()) {
        std::fprintf(stderr, "usage: composite_ref <outdir> (CHIARO_TEST_GPU_DRIVER=vulkan)\n");
        return 2;
    }
    const std::string out = argv[1];
    const auto run = [&](const char *name, const Config &config, void (*scene)(), bool glow, bool material, bool iso = false) {
        ctx = Context::Create(TestDevice());
        Configure(config);
        scene();
        Render(out, name, glow, material, iso);
        ctx.reset();
    };
    run("plain", {}, Lights, false, false);
    run("glow", {.glowScale = 2.0f}, Lights, true, false);
    run("glow_scene", {.glowScale = 1.5f, .glowColor = CHI_GLOW_COLOR_SCENE}, Lights, true, false);
    run("exact", {}, Lights, true, true);
    run("direction", {.lightResolution = 0.5f}, Lights, true, true);
    run("ambient", {.ambient = 0.2f}, Directional, false, false);
    run("iso", {.lightResolution = 0.5f, .wetness = 0.6f}, IsoLights, true, true, true);
    return 0;
}
