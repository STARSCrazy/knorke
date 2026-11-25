// wet_street reference (Knorke Chiaro port): the scene of examples/wet_street_demo/main.cpp of Chiaro @ 9bc5b9b
// (default options: wet, footprint shadows, soft near field), rendered once like its --screenshot frame, and its
// input layers dumped as raw RGBA8 (640x360, row-major, no header):
//   wet_street_ref <out_dir>
// writes <out_dir>/scene.rgba, material.rgba, surface.rgba (the renderer layers after End) and frame.rgba
// (the 1280x720 R8G8B8A8_UNORM frame, compared with tests/golden/chiaro/wet_street.png).
// The scene code is copied from the demo with only the options, keys and smoke mode removed; change both together.
// Build (MSVC x64, against an out-of-tree Chiaro 9bc5b9b build <b>), from a directory outside the repo:
//   cl /O2 /EHsc /std:c++20 /I <chiaro>\include /I <b>\_deps\sdl3-src\include wet_street_ref.cpp
//      /link /LIBPATH:<b>\lib\Release Chiaro.lib SDL3.lib
// and run it next to Chiaro.dll and SDL3.dll of <b>\bin\Release (Vulkan: SDL_GPU_DRIVER=vulkan).
// The dumps become the RGBA PNG fixtures next to this file (scene, material, surface), read by
// crates/knorke-integration/tests/gpu_wet_street.rs (converted with Pillow: Image.frombytes("RGBA", ...).save).
//
// Origin of the scene code: chiaro @ 9bc5b9b, examples/wet_street_demo/main.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as such
// (this file is altered: options, input and smoke mode removed, layer dumps added) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

#include <Chiaro/Chiaro.hpp>

#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>

#include <array>
#include <cstdio>
#include <optional>
#include <string>
#include <vector>

namespace {

constexpr int kW = 640;
constexpr int kH = 360;
constexpr float kHeightStep = 0.05f;
constexpr float kFigureHeight = 1.8f;
constexpr float kFootprint = 0.45f;
constexpr float kWetness = 0.8f;
constexpr float kDiag = 0.70710678f;

struct V2
{
    float x, y;
};

constexpr std::array<V2, 4> kFigures{{{3.5f, 3.0f}, {5.0f, 4.6f}, {8.0f, 2.8f}, {9.6f, 4.4f}}};
constexpr std::array<V2, 2> kWallLamps{{{2.0f, 0.25f}, {10.0f, 0.25f}}};
constexpr std::array<V2, 3> kStreetLamps{{{1.0f, 6.5f}, {6.5f, 6.5f}, {12.0f, 6.5f}}};
constexpr std::array<V2, 2> kPuddles{{{1.9f, 7.7f}, {7.4f, 7.7f}}};

SDL_FPoint Project(const chiaro::Projection &p, float x, float y, float z)
{
    return SDL_FPoint{p.origin[0] + x * p.x_axis[0] + y * p.y_axis[0] + z * p.z_axis[0],
                      p.origin[1] + x * p.x_axis[1] + y * p.y_axis[1] + z * p.z_axis[1]};
}

void Quad(SDL_Renderer *r, const chiaro::Projection &p, const std::array<chiaro::Vec3, 4> &q, SDL_FColor c)
{
    SDL_Vertex v[4];
    for (int i = 0; i < 4; ++i) {
        v[i] = SDL_Vertex{Project(p, q[i].x, q[i].y, q[i].z), c, SDL_FPoint{0.0f, 0.0f}};
    }
    static const int kIndices[6] = {0, 1, 2, 0, 2, 3};
    SDL_RenderGeometry(r, nullptr, v, 4, kIndices, 6);
}

void Flat(SDL_Renderer *r, const chiaro::Projection &p, float x0, float y0, float x1, float y1, float z, SDL_FColor c)
{
    Quad(r, p, {{{x0, y0, z}, {x1, y0, z}, {x1, y1, z}, {x0, y1, z}}}, c);
}

void Wall(SDL_Renderer *r, const chiaro::Projection &p, V2 a, V2 b, float z0, float z1, SDL_FColor c)
{
    Quad(r, p, {{{a.x, a.y, z0}, {b.x, b.y, z0}, {b.x, b.y, z1}, {a.x, a.y, z1}}}, c);
}

V2 Left(V2 at, float half)
{
    return {at.x - half * kDiag, at.y + half * kDiag};
}
V2 Right(V2 at, float half)
{
    return {at.x + half * kDiag, at.y - half * kDiag};
}

void Puddle(SDL_Renderer *r, const chiaro::Projection &p, V2 c, SDL_FColor color)
{
    Flat(r, p, c.x - 0.8f, c.y - 0.45f, c.x + 0.8f, c.y + 0.45f, 0.0f, color);
}

void DrawColors(SDL_Renderer *r, const chiaro::Projection &p)
{
    const SDL_FColor asphalt{0.16f, 0.17f, 0.19f, 1.0f}, wall{0.42f, 0.40f, 0.37f, 1.0f};
    Flat(r, p, -6.0f, -4.0f, 20.0f, 0.0f, 0.0f, asphalt);
    Flat(r, p, -6.0f, 0.0f, 20.0f, 1.6f, 0.0f, SDL_FColor{0.30f, 0.30f, 0.31f, 1.0f});
    Flat(r, p, -6.0f, 1.6f, 20.0f, 20.0f, 0.0f, asphalt);
    for (V2 c : kPuddles) {
        Puddle(r, p, c, SDL_FColor{0.10f, 0.11f, 0.13f, 1.0f});
    }
    Wall(r, p, {-2.0f, 0.0f}, {14.0f, 0.0f}, 0.0f, 6.0f, wall);
    Wall(r, p, {14.0f, 0.0f}, {14.0f, -4.0f}, 0.0f, 6.0f,
         SDL_FColor{wall.r * 0.8f, wall.g * 0.8f, wall.b * 0.8f, 1.0f});
    Flat(r, p, -2.0f, -4.0f, 14.0f, 0.0f, 6.0f, SDL_FColor{0.20f, 0.20f, 0.22f, 1.0f});
    for (V2 at : kWallLamps) {
        Wall(r, p, {at.x - 0.15f, at.y}, {at.x + 0.15f, at.y}, 2.5f, 2.7f, SDL_FColor{0.9f, 0.95f, 1.0f, 1.0f});
    }
    for (V2 at : kStreetLamps) {
        Wall(r, p, Left(at, 0.08f), Right(at, 0.08f), 0.0f, 4.5f, SDL_FColor{0.1f, 0.1f, 0.1f, 1.0f});
    }
    for (V2 f : kFigures) {
        Wall(r, p, Left(f, 0.2f), Right(f, 0.2f), 0.0f, kFigureHeight, SDL_FColor{0.55f, 0.35f, 0.25f, 1.0f});
    }
}

bool DrawSurfaces(chiaro::Layers &layers)
{
    bool ok = layers.DrawSurfaceBox(6.0f, -2.0f, 16.0f, 4.0f, 6.0f);
    for (V2 at : kWallLamps) {
        ok = ok && layers.DrawSurfaceWall({at.x - 0.15f, at.y, 2.5f}, {at.x + 0.15f, at.y, 2.5f}, 2.7f);
    }
    for (V2 at : kStreetLamps) {
        const V2 a = Left(at, 0.08f), b = Right(at, 0.08f);
        ok = ok && layers.DrawSurfaceWall({a.x, a.y, 0.0f}, {b.x, b.y, 0.0f}, 4.5f);
    }
    for (V2 f : kFigures) {
        const V2 a = Left(f, 0.2f), b = Right(f, 0.2f);
        ok = ok && layers.DrawSurfaceWall({a.x, a.y, 0.0f}, {b.x, b.y, 0.0f}, kFigureHeight);
    }
    return ok;
}

void DrawMaterials(SDL_Renderer *r, const chiaro::Projection &p)
{
    const SDL_FColor none{0.0f, 0.0f, 0.0f, 0.0f};
    Flat(r, p, -6.0f, -4.0f, 20.0f, 20.0f, 0.0f, chiaro::MaterialColor(chiaro::material::AsphaltWet));
    Flat(r, p, -6.0f, 0.0f, 20.0f, 0.5f, 0.0f, chiaro::MaterialColor(chiaro::material::AsphaltDry));
    for (V2 c : kPuddles) {
        Puddle(r, p, c, chiaro::MaterialColor(chiaro::material::Puddle));
    }
    Wall(r, p, {-2.0f, 0.0f}, {14.0f, 0.0f}, 0.0f, 6.0f, none);
    Wall(r, p, {14.0f, 0.0f}, {14.0f, -4.0f}, 0.0f, 6.0f, none);
    for (V2 at : kStreetLamps) {
        Wall(r, p, Left(at, 0.08f), Right(at, 0.08f), 0.0f, 4.5f, none);
    }
    for (V2 f : kFigures) {
        Wall(r, p, Left(f, 0.2f), Right(f, 0.2f), 0.0f, kFigureHeight, none);
    }
}

/** Downloads a whole R8G8B8A8 GPU texture and writes its bytes to `path`. */
bool DumpRaw(SDL_GPUDevice *device, SDL_GPUTexture *tex, Uint32 w, Uint32 h, const std::string &path)
{
    SDL_GPUTransferBufferCreateInfo tbi{};
    tbi.usage = SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD;
    tbi.size = w * h * 4;
    SDL_GPUTransferBuffer *tb = SDL_CreateGPUTransferBuffer(device, &tbi);
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(device);
    SDL_GPUCopyPass *copy = SDL_BeginGPUCopyPass(cmd);
    SDL_GPUTextureRegion src{};
    src.texture = tex;
    src.w = w;
    src.h = h;
    src.d = 1;
    SDL_GPUTextureTransferInfo dst{};
    dst.transfer_buffer = tb;
    SDL_DownloadFromGPUTexture(copy, &src, &dst);
    SDL_EndGPUCopyPass(copy);
    SDL_SubmitGPUCommandBuffer(cmd);
    SDL_WaitForGPUIdle(device);
    bool ok = false;
    if (const void *mapped = SDL_MapGPUTransferBuffer(device, tb, false)) {
        if (FILE *f = std::fopen(path.c_str(), "wb")) {
            ok = std::fwrite(mapped, 1, tbi.size, f) == tbi.size;
            std::fclose(f);
        }
        SDL_UnmapGPUTransferBuffer(device, tb);
    }
    SDL_ReleaseGPUTransferBuffer(device, tb);
    return ok;
}

/** The GPU texture behind a renderer layer; fails unless it is R8G8B8A8_UNORM (bytes as Knorke's layers). */
SDL_GPUTexture *LayerGpuTexture(const chiaro::Layers &layers, Uint32 kind)
{
    SDL_Texture *t = layers.GetRendererTexture(kind);
    if (!t || t->format != SDL_PIXELFORMAT_ABGR8888) {
        SDL_Log("layer %u: no RGBA32 renderer texture (format 0x%08x)", kind, t ? t->format : 0u);
        return nullptr;
    }
    return static_cast<SDL_GPUTexture *>(
        SDL_GetPointerProperty(SDL_GetTextureProperties(t), SDL_PROP_TEXTURE_GPU_TEXTURE_POINTER, nullptr));
}

} // namespace

int main(int argc, char **argv)
{
    if (argc != 2) {
        SDL_Log("usage: %s <out_dir>", argv[0]);
        return 1;
    }
    const std::string out = argv[1];
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        return 1;
    }
    SDL_GPUDevice *device = SDL_CreateGPUDevice(
        SDL_GPU_SHADERFORMAT_SPIRV | SDL_GPU_SHADERFORMAT_DXIL | SDL_GPU_SHADERFORMAT_MSL, false, nullptr);
    SDL_Window *window = device ? SDL_CreateWindow("wet_street_ref", 1280, 720, 0) : nullptr;
    if (!window || !SDL_ClaimWindowForGPUDevice(device, window)) {
        SDL_Log("setup failed: %s", SDL_GetError());
        return 1;
    }
    SDL_Log("driver %s", SDL_GetGPUDeviceDriver(device));
    SDL_Renderer *renderer = SDL_CreateGPURenderer(device, nullptr);
    std::optional<chiaro::Context> fx = chiaro::Context::Create(device);
    if (!renderer || !fx) {
        SDL_Log("setup failed: %s", chiaro::GetError());
        return 1;
    }
    fx->SetAmbient(chiaro::Color(0.10f, 0.11f, 0.15f));
    fx->SetScaling(CHI_SCALE_INTEGER, CHI_FILTER_NEAREST);
    fx->SetTonemap(CHI_TONEMAP_SOFT);
    fx->SetHeightStep(kHeightStep);
    fx->SetBloom(0.6f);
    (void)fx->Edit([&](chiaro::Settings &s) {
        s.quality.preset = CHI_QUALITY_CUSTOM;
        s.quality.light_resolution = 1.0f;
        s.quality.shadow_resolution = 0.5f;
    });
    std::optional<chiaro::Layers> layers =
        fx->CreateLayers(renderer, kW, kH, chiaro::Layer::Surface | chiaro::Layer::Material);
    const chiaro::Projection projection = chiaro::IsoProjection(5.0f, 1.5f, 24.0f, 1.0f, kW, kH);
    if (!layers || !layers->SetProjection(projection)) {
        SDL_Log("layers failed: %s", chiaro::GetError());
        return 1;
    }
    chiaro::Obstacle building = fx->CreateShape(chiaro::Box(6.0f, -2.0f, 16.0f, 4.0f, 6.0f));
    std::vector<chiaro::ShapeDesc> figureDescs;
    for (V2 f : kFigures) {
        figureDescs.push_back(chiaro::Circle(f.x, f.y, kFootprint, kFigureHeight).Footprint(true));
    }
    chiaro::Obstacles figures = fx->CreateShapes(figureDescs);
    std::vector<chiaro::LightDesc> wall;
    for (V2 at : kWallLamps) {
        wall.push_back(chiaro::PointLight(at.x, at.y, 6.0f, chiaro::Color(0.75f, 0.9f, 1.0f), 4.0f)
                           .Height(2.6f)
                           .SourceSize(0.25f)
                           .SoftNearField(true));
    }
    chiaro::Lights wallLamps = fx->CreateLights(wall);
    std::vector<chiaro::LightDesc> street;
    for (V2 at : kStreetLamps) {
        street.push_back(chiaro::PointLight(at.x, at.y, 11.0f, chiaro::Color(1.0f, 0.78f, 0.5f), 1.6f)
                             .Height(4.5f)
                             .SourceSize(0.15f));
    }
    chiaro::Lights streetLamps = fx->CreateLights(street);
    if (!building || !figures || !wallLamps || !streetLamps || !fx->SetWetness(kWetness)) {
        SDL_Log("scene failed: %s", chiaro::GetError());
        return 1;
    }

    SDL_GPUTextureCreateInfo ti{};
    ti.type = SDL_GPU_TEXTURETYPE_2D;
    ti.format = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
    ti.usage = SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER;
    ti.width = 1280;
    ti.height = 720;
    ti.layer_count_or_depth = 1;
    ti.num_levels = 1;
    SDL_GPUTexture *shotTex = SDL_CreateGPUTexture(device, &ti);
    const SDL_GPUTextureFormat swapFormat = SDL_GetGPUSwapchainTextureFormat(device, window);
    int failures = 0;
    // As the demo: 30 frames to the swapchain, the 30th also into the screenshot texture.
    for (int frame = 1; frame <= 30; ++frame) {
        SDL_Event e;
        while (SDL_PollEvent(&e)) {
        }
        SDL_SetRenderDrawBlendMode(renderer, SDL_BLENDMODE_NONE);
        bool drawn = layers->Begin();
        DrawColors(renderer, projection);
        drawn = drawn && DrawSurfaces(*layers) && layers->SetTarget(chiaro::Layer::Material);
        DrawMaterials(renderer, projection);
        drawn = layers->End() && drawn;
        failures += drawn ? 0 : 1;
        SDL_RenderPresent(renderer);

        SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(device);
        SDL_GPUTexture *swap = nullptr;
        Uint32 sw = 0, sh = 0;
        if (!cmd || !SDL_WaitAndAcquireGPUSwapchainTexture(cmd, window, &swap, &sw, &sh)) {
            SDL_Log("frame failed: %s", SDL_GetError());
            return 1;
        }
        chiaro::Frame f{.layers = *layers, .projection = &projection};
        if (swap) {
            f.target = chiaro::Texture{swap, nullptr, swapFormat, sw, sh};
            failures += fx->Render(cmd, f) ? 0 : 1;
        }
        if (frame == 30) {
            f.target = chiaro::Texture{shotTex, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, 1280, 720};
            failures += fx->Render(cmd, f) ? 0 : 1;
        }
        SDL_SubmitGPUCommandBuffer(cmd);
    }
    const std::pair<Uint32, const char *> dumps[] = {{chiaro::Layer::Scene, "scene"},
                                                     {chiaro::Layer::Material, "material"},
                                                     {chiaro::Layer::Surface, "surface"}};
    for (const auto &[kind, name] : dumps) {
        SDL_GPUTexture *t = LayerGpuTexture(*layers, kind);
        failures += t && DumpRaw(device, t, kW, kH, out + "/" + name + ".rgba") ? 0 : 1;
    }
    failures += DumpRaw(device, shotTex, 1280, 720, out + "/frame.rgba") ? 0 : 1;
    SDL_Log("%d failures", failures);

    streetLamps.Reset();
    wallLamps.Reset();
    figures.Reset();
    building.Reset();
    layers.reset();
    fx.reset();
    SDL_ReleaseGPUTexture(device, shotTex);
    SDL_DestroyRenderer(renderer);
    SDL_ReleaseWindowFromGPUDevice(device, window);
    SDL_DestroyWindow(window);
    SDL_DestroyGPUDevice(device);
    SDL_Quit();
    return failures == 0 ? 0 : 1;
}
