// P28 reference (Knorke Chiaro port): renders two scenes with pattern obstacles with C++ Chiaro (9bc5b9b) and writes
// the occlusion mask (R8G8_UNORM), the light buffer (RGBA16F) and the final target (RGBA8, default output settings) as
// raw bytes.
//   pattern_ref <outdir>
// Writes <outdir>/<scene>.mask.bin, <scene>.light.bin and <scene>.target.bin for scene in height, flat, rotated.
//  - height: obstacles with finite heights. A thin slat fence (box, slat pattern) under a shadowed directional light
//    (the iso_night moon) and a lattice tower (box, lattice pattern with a repeat height) under a shadowed spot light
//    (the iso_night floodlight).
//  - flat: the same boxes infinitely tall (column means), a circle with the lattice, a tile grid whose cells take the
//    slats through a pattern table, under a shadowed point light and the directional light.
//  - rotated: a rotated box (lattice) and a rotated polygon (slats) with finite heights under a shadowed point light
//    and the directional light; measures the rotation (Chiaro: double cos/sin, Knorke: f32 det::sin_cos).
// The sheet is iso_night_demo's PatternSheet (examples/iso_night_demo/Scene.cpp): 20 x 16, slats x 0..3, lattice
// x 4..19.
// Build and run like ../occluder/mask_ref.cpp (the same command line with pattern_ref.cpp). Captured 2026-11-25 on an
// AMD Radeon RX 6700 XT (Vulkan). The scenes are mirrored in crates/knorke-light2d/tests/gpu_patterns.rs
// (`scene_height`, `scene_flat`, `scene_rotated`) and crates/knorke-integration/tests/gpu_patterns.rs (`rotated`);
// change all together.

#include "GpuTestHarness.hpp"
#include "core/Context.hpp"
#include "core/ContextInspect.hpp"

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

namespace {

constexpr Uint32 kW = 96, kH = 48;

std::unique_ptr<Context> ctx;
Uint32 patterns[2] = {};

void Fail(const char *what)
{
    std::fprintf(stderr, "%s failed: %s\n", what, SDL_GetError());
    std::abort();
}

std::vector<Uint8> PatternSheet()
{
    constexpr int w = 20, h = 16;
    std::vector<Uint8> px(static_cast<size_t>(w) * h * 4, 0);
    for (int y = 0; y < h; ++y) {
        for (int x = 0; x < w; ++x) {
            bool closed = false;
            if (x < 4) {
                closed = x >= 2;
            } else {
                const float dx = static_cast<float>(x - 4) + 0.5f - 8.0f, dy = static_cast<float>(y) + 0.5f - 8.0f;
                closed = std::fabs(std::fabs(dx) + std::fabs(dy) - 7.0f) < 1.1f;
            }
            const size_t i = (static_cast<size_t>(y) * w + static_cast<size_t>(x)) * 4;
            px[i] = px[i + 1] = px[i + 2] = 255;
            px[i + 3] = closed ? 255 : 0;
        }
    }
    return px;
}

void CreatePatterns()
{
    const std::vector<Uint8> sheet = PatternSheet();
    chiaro::detail::PatternParams p[2];
    p[0].rect = SDL_Rect{0, 0, 4, 16};
    p[0].repeatWidth = 3.0f;
    p[1].rect = SDL_Rect{4, 0, 16, 16};
    p[1].repeatWidth = 8.0f;
    p[1].repeatHeight = 8.0f;
    if (!ctx->CreatePatterns(20, 16, sheet.data(), 80, SDL_PIXELFORMAT_RGBA32, p, 2, patterns, "pattern_ref")) {
        Fail("CreatePatterns");
    }
}

CHI_ObstacleID Shape(CHI_ShapeDesc &s, float x, float y, float height, float rotation, Uint32 pattern)
{
    s.x = x;
    s.y = y;
    s.height = height;
    s.rotation = rotation;
    const CHI_ObstacleID id{ctx->Obstacles().CreateShape(s)};
    if (id.id == 0 || !ctx->Obstacles().SetPattern(id.id, pattern, ctx->Patterns(), "pattern_ref")) {
        Fail("Box");
    }
    return id;
}

CHI_ObstacleID Box(float x, float y, float w, float d, float height, Uint32 pattern, float rotation = 0.0f)
{
    CHI_ShapeDesc s;
    CHI_InitShapeDesc(&s, CHI_SHAPE_BOX);
    s.width = w;
    s.depth = d;
    return Shape(s, x, y, height, rotation, pattern);
}

void Light(CHI_LightType type, float x, float y, float height, float radius, CHI_Vec3 direction, float intensity)
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
    d.flags &= ~CHI_LIGHT_NO_SHADOWS;
    if (ctx->Lights().Create(d) == 0) {
        Fail("light");
    }
}

void Ambient()
{
    if (!EditSettings(ctx->GetSettings(), [](CHI_Settings &s) { s.ambient = {0.03f, 0.03f, 0.04f}; })) {
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
    SDL_GPUTexture *target = CreateTestTexture(kW, kH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                                               SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER);
    UploadPixels(scene, kW, kH, std::vector<Uint8>(static_cast<size_t>(kW) * kH * 4, 255));
    const CHI_Projection projection = IdentityProjection();
    CHI_Frame info{};
    info.scene = CHI_Texture{scene, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, kW, kH};
    info.target = CHI_Texture{target, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, kW, kH};
    info.projection = &projection;
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(TestDevice());
    if (!ctx->Render(cmd, info)) {
        Fail("render");
    }
    SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
    SDL_WaitForGPUFences(TestDevice(), true, &fence, 1);
    SDL_ReleaseGPUFence(TestDevice(), fence);
    const auto inspect = Inspect(*ctx);
    const auto mask = inspect.OcclusionMask();
    const auto light = inspect.LightBuffer();
    if (!mask.texture || !light.texture) {
        std::abort();
    }
    Dump(out + "/" + name + ".mask.bin", DownloadBytes(mask.texture, mask.width, mask.height, 2));
    Dump(out + "/" + name + ".light.bin", DownloadBytes(light.texture, light.width, light.height, 8));
    Dump(out + "/" + name + ".target.bin", DownloadBytes(target, kW, kH, 4));
    ReleaseTestTexture(scene);
    ReleaseTestTexture(target);
}

/** Finite heights: fence under the moon, tower under the floodlight. */
void SceneHeight()
{
    Ambient();
    CreatePatterns();
    Box(70.0f, 24.0f, 1.0f, 36.0f, 6.0f, patterns[0]);
    Box(30.0f, 24.0f, 8.0f, 8.0f, 16.0f, patterns[1]);
    Light(CHI_LIGHT_SPOT, 42.0f, 24.0f, 12.0f, 50.0f, {-1.0f, 0.0f, -0.4f}, 1.5f);
    Light(CHI_LIGHT_DIRECTIONAL, 0.0f, 0.0f, CHI_HEIGHT_AUTO, 30.0f, {-0.7f, 0.15f, -0.7f}, 0.6f);
}

/** Infinitely tall: column means; a circle and a tile grid with a pattern table. */
void SceneFlat()
{
    Ambient();
    CreatePatterns();
    Box(70.0f, 24.0f, 1.0f, 36.0f, 0.0f, patterns[0]);
    Box(30.0f, 30.0f, 8.0f, 8.0f, 0.0f, patterns[1]);
    CHI_ShapeDesc c;
    CHI_InitShapeDesc(&c, CHI_SHAPE_CIRCLE);
    c.x = 52.0f;
    c.y = 12.0f;
    c.radius = 4.0f;
    const Uint32 circle = ctx->Obstacles().CreateShape(c);
    if (circle == 0 || !ctx->Obstacles().SetPattern(circle, patterns[1], ctx->Patterns(), "pattern_ref")) {
        Fail("circle");
    }
    // A 4 x 2 grid of 3 x 3 cells at (12, 6); the second row reads the slats through table entry 1.
    const Uint8 solid[8] = {1, 1, 0, 1, 1, 1, 1, 0};
    const Uint8 indices[8] = {0, 0, 0, 0, 1, 1, 1, 1};
    CHI_TileGridDesc g;
    CHI_InitTileGridDesc(&g);
    g.x = 12.0f;
    g.y = 6.0f;
    g.columns = 4;
    g.rows = 2;
    g.cell_size[0] = g.cell_size[1] = 3.0f;
    g.solid = solid;
    const Uint32 grid = ctx->Obstacles().CreateTileGrid(g);
    if (grid == 0 || !ctx->Obstacles().SetPattern(grid, patterns[1], ctx->Patterns(), "pattern_ref") ||
        !ctx->Obstacles().SetTilePatternTable(grid, &patterns[0], 1, ctx->Patterns(), "pattern_ref") ||
        !ctx->Obstacles().SetTilePatternCells(grid, SDL_Rect{0, 0, 4, 2}, indices, "pattern_ref")) {
        Fail("grid");
    }
    Light(CHI_LIGHT_POINT, 46.0f, 26.0f, CHI_HEIGHT_AUTO, 60.0f, {1.0f, 0.0f, 0.0f}, 1.2f);
    Light(CHI_LIGHT_DIRECTIONAL, 0.0f, 0.0f, CHI_HEIGHT_AUTO, 30.0f, {-0.7f, 0.15f, -0.7f}, 0.4f);
}

/** Rotated pattern obstacles: a box and a polygon. */
void SceneRotated()
{
    Ambient();
    CreatePatterns();
    Box(30.0f, 24.0f, 10.0f, 6.0f, 10.0f, patterns[1], 30.0f);
    static const float tri[6] = {-5.0f, -8.0f, 6.0f, -2.0f, -1.0f, 9.0f};
    CHI_ShapeDesc s;
    CHI_InitShapeDesc(&s, CHI_SHAPE_POLYGON);
    s.vertices = tri;
    s.vertex_count = 3;
    Shape(s, 66.0f, 22.0f, 8.0f, 17.0f, patterns[0]);
    Light(CHI_LIGHT_POINT, 48.0f, 26.0f, 14.0f, 60.0f, {1.0f, 0.0f, 0.0f}, 1.2f);
    Light(CHI_LIGHT_DIRECTIONAL, 0.0f, 0.0f, CHI_HEIGHT_AUTO, 30.0f, {-0.7f, 0.15f, -0.7f}, 0.4f);
}

} // namespace

int main(int argc, char **argv)
{
    if (argc < 2 || !InitTestDevice()) {
        std::fprintf(stderr, "usage: pattern_ref <outdir> (CHIARO_TEST_GPU_DRIVER=vulkan)\n");
        return 2;
    }
    const std::string out = argv[1];
    const auto run = [&](const char *name, void (*scene)()) {
        ctx = Context::Create(TestDevice());
        scene();
        Render(out, name);
        ctx.reset();
    };
    run("height", SceneHeight);
    run("flat", SceneFlat);
    run("rotated", SceneRotated);
    return 0;
}
