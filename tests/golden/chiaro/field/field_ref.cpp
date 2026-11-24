// P23 reference (Knorke Chiaro port): renders an obstacle scene with C++ Chiaro (9bc5b9b) and writes the occlusion
// mask (R8G8_UNORM, 192x128) and its distance field (R16_FLOAT) as raw bytes, for a full frame and for a sequence of
// cached frames (camera pans, a moved dynamic obstacle).
//   field_ref <outdir>
// Writes <outdir>/<frame>.mask.bin and <outdir>/<frame>.field.bin for frame in full, pan_whole, pan_fraction, and
// <outdir>/updates.txt (the MaskUpdate of every frame: 0 Reused, 1 Incremental, 2 Full).
// Build and run like ../occluder/mask_ref.cpp (the same command line with field_ref.cpp). Captured 2026-11-24 on an
// AMD Radeon RX 6700 XT (Vulkan). The scene is `scene_mask` of mask_ref.cpp; the frames are mirrored in
// crates/knorke-light2d/tests/gpu_occlusion_mask.rs (`cached_sequence`); change both together.

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

std::unique_ptr<Context> ctx;
CHI_Projection projection;
Uint32 mover = 0;

Uint32 Polygon(const std::vector<float> &xy, float x, float y, float rotation, float opacity, Uint32 flags)
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
    const Uint32 id = ctx->Obstacles().CreateShape(d);
    if (id == 0) {
        std::abort();
    }
    return id;
}

void Circle(float radius, float x, float y)
{
    CHI_ShapeDesc d;
    CHI_InitShapeDesc(&d, CHI_SHAPE_CIRCLE);
    d.radius = radius;
    d.x = x;
    d.y = y;
    if (ctx->Obstacles().CreateShape(d) == 0) {
        std::abort();
    }
}

void Tiles(Uint32 columns, Uint32 rows, float cw, float ch, float x, float y, float rotation, float opacity,
           const std::vector<Uint8> &solid, const Uint8 *cellOpacity, const Uint8 *edges, float edgeWidth)
{
    CHI_TileGridDesc d;
    CHI_InitTileGridDesc(&d);
    d.solid = solid.data();
    d.cell_opacity = cellOpacity;
    d.edges = edges;
    d.columns = columns;
    d.rows = rows;
    d.cell_size[0] = cw;
    d.cell_size[1] = ch;
    d.x = x;
    d.y = y;
    d.rotation = rotation;
    d.opacity = opacity;
    d.edge_width = edgeWidth;
    if (ctx->Obstacles().CreateTileGrid(d) == 0) {
        std::abort();
    }
}

void Render()
{
    const Uint32 w = 128, h = 64;
    SDL_GPUTexture *scene = CreateTestTexture(w, h, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    SDL_GPUTexture *target = CreateTestTexture(w, h, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                                               SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER);
    UploadPixels(scene, w, h, std::vector<Uint8>(static_cast<size_t>(w) * h * 4, 255));
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(TestDevice());
    CHI_Frame info{};
    info.scene = CHI_Texture{scene, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, w, h};
    info.target = CHI_Texture{target, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, w, h};
    info.projection = &projection;
    if (!ctx->Render(cmd, info)) {
        std::fprintf(stderr, "render failed: %s\n", SDL_GetError());
        std::abort();
    }
    SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
    SDL_WaitForGPUFences(TestDevice(), true, &fence, 1);
    SDL_ReleaseGPUFence(TestDevice(), fence);
    ReleaseTestTexture(scene);
    ReleaseTestTexture(target);
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

void DumpFrame(const std::string &out, const char *name)
{
    const auto mask = Inspect(*ctx).OcclusionMask();
    const auto field = Inspect(*ctx).Field();
    if (!mask.texture || !field.texture) {
        std::abort();
    }
    Dump(out + "/" + name + ".mask.bin", DownloadBytes(mask.texture, mask.width, mask.height, 2));
    Dump(out + "/" + name + ".field.bin", DownloadBytes(field.texture, field.width, field.height, 2));
}

void Scene()
{
    CHI_LightDesc l;
    CHI_InitLightDesc(&l, CHI_LIGHT_POINT);
    l.x = 20.0f;
    l.y = 32.0f;
    l.radius = 50.0f;
    l.flags &= ~CHI_LIGHT_NO_SHADOWS;
    if (ctx->Lights().Create(l) == 0) {
        std::abort();
    }
    Polygon({48, 24, 56, 24, 56, 40, 48, 40}, 0, 0, 0, 0.75f, 0);
    Polygon({70, 10, 90, 14, 76, 30}, 0, 0, 0, 1.0f, 0);
    mover = Polygon({-10, -2, 10, -2, 10, 2, -10, 2}, 100, 40, 33, 0.5f, CHI_OBSTACLE_DYNAMIC);
    Circle(6.0f, 30.0f, 45.0f);
    Circle(0.2f, 64.25f, 32.25f);
    Polygon({10, 10, 20, 10, 20, 18, 10, 18}, 0, 0, 0, 0.6f, CHI_OBSTACLE_FOOTPRINT_SHADOW);
    std::vector<Uint8> solid(32), opacity(32);
    for (size_t i = 0; i < solid.size(); ++i) {
        solid[i] = (i * 7 + 3) % 5 == 0 ? 0 : 1;
        opacity[i] = static_cast<Uint8>(255 - (i % 4) * 60);
    }
    Tiles(8, 4, 6.0f, 5.0f, 60.0f, 38.0f, 20.0f, 0.8f, solid, opacity.data(), nullptr, 0.0f);
    const std::vector<Uint8> edgeSolid(6, 1);
    const std::vector<Uint8> edges = {1, 2, 4, 8, 5, 0};
    Tiles(3, 2, 10.0f, 10.0f, 90.0f, 5.0f, 0.0f, 1.0f, edgeSolid, nullptr, edges.data(), 3.0f);
}

} // namespace

int main(int argc, char **argv)
{
    if (argc < 2 || !InitTestDevice()) {
        std::fprintf(stderr, "usage: field_ref <outdir> (CHIARO_TEST_GPU_DRIVER=vulkan)\n");
        return 2;
    }
    const std::string out = argv[1];
    ctx = Context::Create(TestDevice());
    projection = IdentityProjection();
    Scene();
    // Local updates are never cheaper at this size: lift the ratio so that the cached frames are incremental.
    Inspect(*ctx).SetFullRebuildRatio(1.0);
    std::string updates;
    const auto frame = [&](const char *name) {
        Render();
        updates += std::to_string(static_cast<int>(Inspect(*ctx).LastMaskUpdate())) + "\n";
        if (name) {
            DumpFrame(out, name);
        }
    };
    frame("full");
    frame(nullptr); // unchanged: reused
    // A pan by whole pixels, and the dynamic bar moved and turned.
    projection.origin[0] -= 3.0f;
    projection.origin[1] += 2.0f;
    if (!ctx->Obstacles().SetTransform(mover, 96.0f, 44.0f, 50.0f)) {
        std::abort();
    }
    frame("pan_whole");
    // A pan by fractions of a pixel, the bar unchanged.
    projection.origin[0] -= 1.4f;
    projection.origin[1] -= 0.6f;
    frame("pan_fraction");
    FILE *f = std::fopen((out + "/updates.txt").c_str(), "wb");
    if (!f) {
        std::abort();
    }
    std::fwrite(updates.data(), 1, updates.size(), f);
    std::fclose(f);
    ctx.reset();
    return 0;
}
