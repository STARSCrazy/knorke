// P22 reference (Knorke Chiaro port): renders two obstacle scenes with C++ Chiaro (9bc5b9b) and writes the
// occlusion mask (R8G8_UNORM, 192x128) and, for the scenes in height mode, the height texture (RGBA16F) as raw
// bytes. The third scene adds the application's sprite mask (CHI_Frame::buffers.occlusion) in height mode
// (occlusion_mask_height.frag).
//   mask_ref <outdir>
// Writes <outdir>/mask_scene.mask.bin, <outdir>/height_scene.mask.bin, <outdir>/height_scene.height.bin,
// <outdir>/sprite_height.mask.bin, <outdir>/sprite_height.height.bin.
// Uses Chiaro's internal context and test harness (like tests/gpu/OcclusionTests.cpp). Build (MSVC x64, against an
// out-of-tree Chiaro 9bc5b9b Release build <b> of source tree <c>; nothing in <c> is changed):
//   cl /O2 /std:c++20 /EHsc /utf-8 /MD /I<c>\src /I<c>\include /I<b>\generated /I<b>\_deps\sdl3-build\include-revision
//      /I<b>\_deps\sdl3-src\include /I<c>\tests\gpu mask_ref.cpp <c>\tests\gpu\GpuTestHarness.cpp
//      /link <b>\chiaro_core.dir\Release\chiaro_core.lib <b>\lib\Release\Chiaro.lib <b>\lib\Release\SDL3.lib
// Run next to Chiaro.dll and SDL3.dll with CHIARO_TEST_GPU_DRIVER=vulkan. Captured 2026-11-24 on an AMD Radeon
// RX 6700 XT (Vulkan); sprite_height added 2026-11-25 on the same machine. The scenes are mirrored in
// crates/knorke-light2d/tests/gpu_occluder_raster.rs (`scene_mask`, `scene_height`) and
// crates/knorke-light2d/tests/gpu_occlusion_mask.rs (`sprite_height`); change both together.

#include "GpuTestHarness.hpp"
#include "core/Context.hpp"
#include "core/ContextInspect.hpp"

#include <Chiaro/Chiaro.h>

#include <cstdio>
#include <cstdlib>
#include <string>
#include <vector>

using namespace chiaro::test;
using chiaro::detail::Context;

namespace {

struct Scene
{
    std::unique_ptr<Context> ctx = Context::Create(TestDevice());

    void Light()
    {
        CHI_LightDesc d;
        CHI_InitLightDesc(&d, CHI_LIGHT_POINT);
        d.x = 20.0f;
        d.y = 32.0f;
        d.radius = 50.0f;
        d.flags &= ~CHI_LIGHT_NO_SHADOWS;
        if (ctx->Lights().Create(d) == 0) {
            std::abort();
        }
    }

    void Polygon(const std::vector<float> &xy, float x, float y, float rotation, float opacity, Uint32 flags,
                 float height = 0.0f, float base = 0.0f)
    {
        CHI_ShapeDesc d;
        CHI_InitShapeDesc(&d, CHI_SHAPE_POLYGON);
        d.vertices = xy.data();
        d.vertex_count = static_cast<Uint32>(xy.size() / 2);
        d.flags = flags;
        d.x = x;
        d.y = y;
        d.rotation = rotation;
        d.opacity = opacity;
        d.height = height;
        d.base_height = base;
        if (ctx->Obstacles().CreateShape(d) == 0) {
            std::abort();
        }
    }

    void Circle(float radius, float x, float y, float height = 0.0f, float base = 0.0f)
    {
        CHI_ShapeDesc d;
        CHI_InitShapeDesc(&d, CHI_SHAPE_CIRCLE);
        d.radius = radius;
        d.x = x;
        d.y = y;
        d.height = height;
        d.base_height = base;
        if (ctx->Obstacles().CreateShape(d) == 0) {
            std::abort();
        }
    }

    /** sprite: optional RGBA8 occlusion buffer of the scene size (alpha at or above 0.5 occludes). */
    void Render(const std::vector<Uint8> *sprite = nullptr)
    {
        const Uint32 w = 128, h = 64;
        SDL_GPUTexture *occlusion = nullptr;
        if (sprite) {
            occlusion = CreateTestTexture(w, h, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER);
            UploadPixels(occlusion, w, h, *sprite);
        }
        SDL_GPUTexture *scene = CreateTestTexture(w, h, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                                                  SDL_GPU_TEXTUREUSAGE_SAMPLER);
        SDL_GPUTexture *target = CreateTestTexture(
            w, h, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER);
        UploadPixels(scene, w, h, std::vector<Uint8>(static_cast<size_t>(w) * h * 4, 255));
        CHI_Projection projection = IdentityProjection();
        SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(TestDevice());
        CHI_Frame info{};
        info.scene = CHI_Texture{scene, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, w, h};
        info.target = CHI_Texture{target, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, w, h};
        info.projection = &projection;
        if (occlusion) {
            info.buffers.occlusion = CHI_Texture{occlusion, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, w, h};
        }
        if (!ctx->Render(cmd, info)) {
            std::fprintf(stderr, "render failed: %s\n", SDL_GetError());
            std::abort();
        }
        SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
        SDL_WaitForGPUFences(TestDevice(), true, &fence, 1);
        SDL_ReleaseGPUFence(TestDevice(), fence);
        ReleaseTestTexture(scene);
        ReleaseTestTexture(target);
        if (occlusion) {
            ReleaseTestTexture(occlusion);
        }
    }

    void Dump(const std::string &path, const std::vector<Uint8> &bytes) const
    {
        FILE *f = std::fopen(path.c_str(), "wb");
        if (!f) {
            std::abort();
        }
        std::fwrite(bytes.data(), 1, bytes.size(), f);
        std::fclose(f);
        std::printf("%s: %zu bytes\n", path.c_str(), bytes.size());
    }

    void DumpMask(const std::string &path) const
    {
        const auto mask = chiaro::detail::Inspect(*ctx).OcclusionMask();
        if (!mask.texture) {
            std::abort();
        }
        Dump(path, DownloadBytes(mask.texture, mask.width, mask.height, 2));
    }

    void DumpHeight(const std::string &path) const
    {
        const auto height = chiaro::detail::Inspect(*ctx).OcclusionHeight();
        if (!height.texture) {
            std::abort();
        }
        Dump(path, DownloadBytes(height.texture, height.width, height.height, 8));
    }
};

void Tiles(Scene &s, Uint32 columns, Uint32 rows, float cw, float ch, float x, float y, float rotation, float opacity,
           const std::vector<Uint8> &solid, const Uint8 *cellOpacity, const Uint8 *edges, float edgeWidth,
           float levelHeight, const Uint8 *base)
{
    CHI_TileGridDesc d;
    CHI_InitTileGridDesc(&d);
    d.solid = solid.data();
    d.cell_opacity = cellOpacity;
    d.edges = edges;
    d.base_levels = base;
    d.columns = columns;
    d.rows = rows;
    d.cell_size[0] = cw;
    d.cell_size[1] = ch;
    d.x = x;
    d.y = y;
    d.rotation = rotation;
    d.opacity = opacity;
    d.edge_width = edgeWidth;
    d.level_height = levelHeight;
    if (s.ctx->Obstacles().CreateTileGrid(d) == 0) {
        std::abort();
    }
}

} // namespace

int main(int argc, char **argv)
{
    if (argc < 2 || !InitTestDevice()) {
        std::fprintf(stderr, "usage: mask_ref <outdir> (CHIARO_TEST_GPU_DRIVER=vulkan)\n");
        return 2;
    }
    const std::string out = argv[1];
    {
        // scene_mask
        Scene s;
        s.Light();
        s.Polygon({48, 24, 56, 24, 56, 40, 48, 40}, 0, 0, 0, 0.75f, 0);
        s.Polygon({70, 10, 90, 14, 76, 30}, 0, 0, 0, 1.0f, 0);
        s.Polygon({-10, -2, 10, -2, 10, 2, -10, 2}, 100, 40, 33, 0.5f, CHI_OBSTACLE_DYNAMIC);
        s.Circle(6.0f, 30.0f, 45.0f);
        s.Circle(0.2f, 64.25f, 32.25f);
        s.Polygon({10, 10, 20, 10, 20, 18, 10, 18}, 0, 0, 0, 0.6f, CHI_OBSTACLE_FOOTPRINT_SHADOW);
        std::vector<Uint8> solid(32), opacity(32);
        for (size_t i = 0; i < solid.size(); ++i) {
            solid[i] = (i * 7 + 3) % 5 == 0 ? 0 : 1;
            opacity[i] = static_cast<Uint8>(255 - (i % 4) * 60);
        }
        Tiles(s, 8, 4, 6.0f, 5.0f, 60.0f, 38.0f, 20.0f, 0.8f, solid, opacity.data(), nullptr, 0.0f, 0.0f, nullptr);
        const std::vector<Uint8> edgeSolid(6, 1);
        const std::vector<Uint8> edges = {1, 2, 4, 8, 5, 0};
        Tiles(s, 3, 2, 10.0f, 10.0f, 90.0f, 5.0f, 0.0f, 1.0f, edgeSolid, nullptr, edges.data(), 3.0f, 0.0f, nullptr);
        s.Render();
        s.DumpMask(out + "/mask_scene.mask.bin");
    }
    {
        // scene_height
        Scene s;
        s.Light();
        s.Polygon({10, 10, 30, 10, 30, 30, 10, 30}, 0, 0, 0, 1.0f, 0, 2.0f, 1.0f);
        s.Polygon({20, 20, 40, 20, 40, 40, 20, 40}, 0, 0, 0, 1.0f, CHI_OBSTACLE_DYNAMIC, 6.0f, 3.0f);
        s.Polygon({50, 10, 60, 10, 60, 30, 50, 30}, 0, 0, 0, 1.0f, 0);
        s.Circle(6.0f, 80.0f, 30.0f, 5.0f, 2.0f);
        const std::vector<Uint8> solid = {0, 1, 3, 4, 2, 0};
        const std::vector<Uint8> base = {0, 0, 2, 0, 1, 0};
        Tiles(s, 3, 2, 10.0f, 10.0f, 84.0f, 38.0f, 0.0f, 1.0f, solid, nullptr, nullptr, 0.0f, 2.0f, base.data());
        s.Render();
        s.DumpMask(out + "/height_scene.mask.bin");
        s.DumpHeight(out + "/height_scene.height.bin");
    }
    {
        // sprite_height: a finite obstacle switches on height mode; the sprite mask has an opaque block, a block
        // at alpha 128 (occludes) and one at alpha 127 (does not), one of them over the obstacle.
        Scene s;
        s.Light();
        s.Polygon({10, 10, 30, 10, 30, 30, 10, 30}, 0, 0, 0, 1.0f, 0, 3.0f, 1.0f);
        std::vector<Uint8> sprite(static_cast<size_t>(128) * 64 * 4, 0);
        const auto block = [&](Uint32 x0, Uint32 y0, Uint32 x1, Uint32 y1, Uint8 alpha) {
            for (Uint32 y = y0; y < y1; ++y) {
                for (Uint32 x = x0; x < x1; ++x) {
                    sprite[(static_cast<size_t>(y) * 128 + x) * 4 + 3] = alpha;
                }
            }
        };
        block(20, 20, 40, 34, 255);
        block(60, 8, 72, 40, 128);
        block(90, 30, 110, 50, 127);
        s.Render(&sprite);
        s.DumpMask(out + "/sprite_height.mask.bin");
        s.DumpHeight(out + "/sprite_height.height.bin");
    }
    ShutdownTestDevice();
    return 0;
}
