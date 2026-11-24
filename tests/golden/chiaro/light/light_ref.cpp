// P25 reference (Knorke Chiaro port): renders five light scenes with C++ Chiaro (9bc5b9b) and writes the light buffer
// (RGBA16F) and, in the material modes, the specular or direction buffer as raw bytes, plus the light programs each
// scene used.
//   light_ref <outdir>
// Writes <outdir>/<scene>.light.bin (and <scene>.specular.bin / <scene>.direction.bin) for scene in flat, shadow,
// height, exact, direction, and <outdir>/programs.txt (one line per scene: name, then "base:features" of every light
// program the context created, in table order; base 0 Light, 1 LightSdf, 2 Directional; features as bit mask).
// Build and run like ../occluder/mask_ref.cpp (the same command line with light_ref.cpp). Captured 2026-11-24 on an
// AMD Radeon RX 6700 XT (Vulkan). The scenes are mirrored in crates/knorke-light2d/tests/gpu_lighting_stage.rs
// (`scene_*`); change both together.

#include "GpuTestHarness.hpp"
#include "core/Context.hpp"
#include "core/ContextInspect.hpp"
#include "gpu/LightProgram.hpp"

#include <Chiaro/Chiaro.h>

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <memory>
#include <string>
#include <vector>

using namespace chiaro::test;
using chiaro::detail::Context;
using chiaro::detail::Inspect;
using chiaro::detail::kLightPrograms;

namespace {

constexpr Uint32 kW = 96, kH = 48;

std::unique_ptr<Context> ctx;

struct Light
{
    CHI_LightType type = CHI_LIGHT_POINT;
    float x = 0, y = 0, height = CHI_HEIGHT_AUTO, radius = 30;
    CHI_Color color{1, 1, 1};
    float intensity = 1, source = 0, falloff = 2;
    bool shadows = false;
    CHI_Vec3 direction{1, 0, 0};
    float inner = 20, outer = 30;
    CHI_Vec3 axis{1, 0, 0};
    float halfWidth = 0.5f;
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
    d.source_size = l.source;
    d.falloff = l.falloff;
    d.direction = l.direction;
    d.inner_angle = l.inner;
    d.outer_angle = l.outer;
    d.axis = l.axis;
    d.half_width = l.halfWidth;
    if (l.shadows) {
        d.flags &= ~CHI_LIGHT_NO_SHADOWS;
    } else {
        d.flags |= CHI_LIGHT_NO_SHADOWS;
    }
    if (ctx->Lights().Create(d) == 0) {
        std::fprintf(stderr, "light rejected: %s\n", SDL_GetError());
        std::abort();
    }
}

void Polygon(const std::vector<float> &xy, float height, float opacity)
{
    CHI_ShapeDesc d;
    CHI_InitShapeDesc(&d, CHI_SHAPE_POLYGON);
    d.vertices = xy.data();
    d.vertex_count = static_cast<Uint32>(xy.size() / 2);
    d.height = height;
    d.opacity = opacity;
    if (ctx->Obstacles().CreateShape(d) == 0) {
        std::abort();
    }
}

void Circle(float radius, float x, float y, float height)
{
    CHI_ShapeDesc d;
    CHI_InitShapeDesc(&d, CHI_SHAPE_CIRCLE);
    d.radius = radius;
    d.x = x;
    d.y = y;
    d.height = height;
    if (ctx->Obstacles().CreateShape(d) == 0) {
        std::abort();
    }
}

void Ambient(float r, float g, float b, float lightResolution)
{
    const bool ok = EditSettings(ctx->GetSettings(), [&](CHI_Settings &s) {
        s.ambient = {r, g, b};
        if (lightResolution < 1.0f) {
            s.quality.preset = CHI_QUALITY_CUSTOM;
            s.quality.light_resolution = lightResolution;
            s.quality.shadow_resolution = 1.0f;
        }
    });
    if (!ok) {
        std::abort();
    }
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

void DumpTexture(const std::string &path, const chiaro::detail::TextureInfo &t)
{
    if (!t.texture) {
        std::fprintf(stderr, "%s: no texture\n", path.c_str());
        std::abort();
    }
    Dump(path, DownloadBytes(t.texture, t.width, t.height, 8));
}

std::string programs;

void Render(const std::string &out, const char *name, bool material)
{
    SDL_GPUTexture *scene = CreateTestTexture(kW, kH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    SDL_GPUTexture *target = CreateTestTexture(kW, kH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                                               SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER);
    SDL_GPUTexture *mat = nullptr;
    UploadPixels(scene, kW, kH, std::vector<Uint8>(static_cast<size_t>(kW) * kH * 4, 255));
    const CHI_Projection projection = IdentityProjection();
    CHI_Frame info{};
    info.scene = CHI_Texture{scene, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, kW, kH};
    info.target = CHI_Texture{target, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, kW, kH};
    info.projection = &projection;
    if (material) {
        mat = CreateTestTexture(kW, kH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        UploadPixels(mat, kW, kH, Material());
        info.buffers.material = CHI_Texture{mat, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, kW, kH};
    }
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(TestDevice());
    if (!ctx->Render(cmd, info)) {
        std::fprintf(stderr, "render failed: %s\n", SDL_GetError());
        std::abort();
    }
    SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
    SDL_WaitForGPUFences(TestDevice(), true, &fence, 1);
    SDL_ReleaseGPUFence(TestDevice(), fence);
    const auto inspect = Inspect(*ctx);
    DumpTexture(out + "/" + name + ".light.bin", inspect.LightBuffer());
    if (inspect.Specular().texture) {
        DumpTexture(out + "/" + name + ".specular.bin", inspect.Specular());
    }
    if (inspect.Direction().texture) {
        DumpTexture(out + "/" + name + ".direction.bin", inspect.Direction());
    }
    programs += name;
    for (const auto &p : kLightPrograms) {
        if (inspect.UsedProgram(p.Program())) {
            programs += " " + std::to_string(static_cast<int>(p.base)) + ":" + std::to_string(p.features);
        }
    }
    programs += "\n";
    ReleaseTestTexture(scene);
    ReleaseTestTexture(target);
    if (mat) {
        ReleaseTestTexture(mat);
    }
}

/** No obstacles: every light in one unshadowed draw. */
void SceneFlat()
{
    Ambient(0.05f, 0.06f, 0.07f, 1.0f);
    Add({.x = 20, .y = 24, .radius = 30, .color = {1.0f, 0.8f, 0.6f}, .intensity = 1.2f});
    Add({.x = 48, .y = 10, .height = 8, .radius = 20, .falloff = 1.5f});
    Add({.type = CHI_LIGHT_SPOT, .x = 70, .y = 30, .radius = 40, .direction = {-1.0f, 0.3f, -0.2f}, .inner = 15,
         .outer = 35});
    Add({.type = CHI_LIGHT_LINE, .x = 40, .y = 40, .radius = 18, .color = {0.4f, 0.7f, 1.0f}, .axis = {12, 0, 0}});
    Add({.type = CHI_LIGHT_DIRECTIONAL, .intensity = 0.1f, .direction = {0.3f, 0.4f, -1.0f}});
}

/** Flat obstacles (one partly transparent): hard and soft point, spot and rect light shadows, one fill light. */
void SceneShadow()
{
    Ambient(0.03f, 0.03f, 0.03f, 1.0f);
    Polygon({40, 14, 48, 14, 48, 30, 40, 30}, 0.0f, 1.0f);
    Polygon({60, 8, 72, 12, 64, 22}, 0.0f, 0.6f);
    Circle(4.0f, 26.0f, 34.0f, 0.0f);
    Add({.x = 18, .y = 20, .radius = 50, .shadows = true});
    Add({.x = 80, .y = 36, .radius = 45, .color = {1.0f, 0.7f, 0.5f}, .source = 3, .shadows = true});
    Add({.type = CHI_LIGHT_SPOT, .x = 50, .y = 44, .radius = 40, .source = 1, .shadows = true,
         .direction = {0, -1, 0}, .inner = 20, .outer = 40});
    Add({.type = CHI_LIGHT_RECT, .x = 30, .y = 6, .radius = 30, .color = {0.6f, 0.8f, 1.0f}, .shadows = true,
         .direction = {0, 1, 0}, .axis = {6, 0, 0}, .halfWidth = 2});
    Add({.x = 88, .y = 6, .radius = 15});
}

/** Obstacles with heights (one infinitely tall): raised point and line light, a shadow-casting directional light. */
void SceneHeight()
{
    Ambient(0.04f, 0.04f, 0.05f, 1.0f);
    Polygon({40, 14, 48, 14, 48, 30, 40, 30}, 6.0f, 1.0f);
    Polygon({60, 8, 72, 12, 64, 22}, 3.0f, 1.0f);
    Circle(4.0f, 26.0f, 34.0f, 0.0f);
    Add({.x = 18, .y = 20, .height = 10, .radius = 50, .source = 1, .shadows = true});
    Add({.type = CHI_LIGHT_LINE, .x = 70, .y = 40, .height = 4, .radius = 30, .shadows = true, .axis = {0, 6, 0}});
    Add({.type = CHI_LIGHT_DIRECTIONAL, .intensity = 0.6f, .source = 1, .shadows = true,
         .direction = {0.6f, 0.3f, -0.8f}});
}

/** A material buffer: unshadowed and shadowed lights, a shadowed spot and a shadowed directional light. */
void SceneMaterial(float lightResolution)
{
    Ambient(0.05f, 0.05f, 0.05f, lightResolution);
    Polygon({70, 30, 78, 30, 78, 40, 70, 40}, 6.0f, 1.0f);
    Circle(3.0f, 20.0f, 10.0f, 4.0f);
    Add({.x = 20, .y = 24, .radius = 40, .color = {1.0f, 0.9f, 0.8f}});
    Add({.x = 60, .y = 10, .radius = 50, .source = 2, .shadows = true});
    Add({.type = CHI_LIGHT_SPOT, .x = 90, .y = 44, .height = 6, .radius = 50, .color = {0.5f, 0.7f, 1.0f},
         .shadows = true, .direction = {-1.0f, -0.5f, -0.3f}, .inner = 20, .outer = 35});
    Add({.type = CHI_LIGHT_DIRECTIONAL, .intensity = 0.4f, .source = 1, .shadows = true,
         .direction = {-0.5f, 0.4f, -0.7f}});
}

} // namespace

int main(int argc, char **argv)
{
    if (argc < 2 || !InitTestDevice()) {
        std::fprintf(stderr, "usage: light_ref <outdir> (CHIARO_TEST_GPU_DRIVER=vulkan)\n");
        return 2;
    }
    const std::string out = argv[1];
    const auto run = [&](const char *name, void (*scene)(), bool material) {
        ctx = Context::Create(TestDevice());
        scene();
        Render(out, name, material);
        ctx.reset();
    };
    run("flat", SceneFlat, false);
    run("shadow", SceneShadow, false);
    run("height", SceneHeight, false);
    run("exact", [] { SceneMaterial(1.0f); }, true);
    run("direction", [] { SceneMaterial(0.5f); }, true);
    Dump(out + "/programs.txt", std::vector<Uint8>(programs.begin(), programs.end()));
    return 0;
}
