// P26 reference (Knorke Chiaro port): renders five scenes with C++ Chiaro (9bc5b9b) that use the light profile atlas,
// the light mask atlas and the ambient zone pass, and writes the light buffer (RGBA16F), in the direction mode also
// the direction buffer and the pure ambient target, as raw bytes, plus the light programs each scene used and a hash
// of each atlas texture.
//   atlas_ref <outdir>
// Writes <outdir>/<scene>.light.bin for scene in profile, mask, zones, zones_only, zones_direction,
// <outdir>/zones_direction.direction.bin and <outdir>/zones_direction.ambient.bin, <outdir>/programs.txt (one line per
// scene: name, then "base:features" of every light program the context created, as in ../light/light_ref.cpp) and
// <outdir>/atlases.txt (one line per scene with an atlas: "<scene> profile|mask <width> <height> <FNV-1a 64 of the
// texture bytes, hex>").
// Build and run like ../light/light_ref.cpp (the same command line with atlas_ref.cpp; its build_ref.bat style).
// Captured 2026-11-24 on an AMD Radeon RX 6700 XT (Vulkan). The scenes are mirrored in
// crates/knorke-light2d/tests/gpu_atlas_zones.rs (`scene_*`); change both together.

#include "GpuTestHarness.hpp"
#include "core/Context.hpp"
#include "core/ContextInspect.hpp"
#include "gpu/LightProgram.hpp"
#include "lighting/AmbientZone.hpp"
#include "lighting/AmbientZoneStore.hpp"
#include "lighting/Mask.hpp"
#include "lighting/Profile.hpp"

#include <Chiaro/Chiaro.h>

#include <cmath>
#include <cstdint>
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
    Uint32 profile = 0;
    float profileRotation = 0;
    Uint32 mask = 0;
    float maskAngle = 0; ///< 0: the default of the desc
    bool window = false;
    CHI_Vec3 maskCenter{0, 0, 0}, maskU{0, 0, 0}, maskV{0, 0, 0};
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
    d.profile.id = l.profile;
    d.profile_rotation = l.profileRotation;
    d.mask.id = l.mask;
    if (l.maskAngle != 0) {
        d.mask_angle = l.maskAngle;
    }
    if (l.window) {
        d.mask_mode = CHI_MASK_WINDOW;
        d.mask_center = l.maskCenter;
        d.mask_u = l.maskU;
        d.mask_v = l.maskV;
    }
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

std::string tables;
int tableCount = 0;

Uint32 Profile(CHI_ProfilePreset preset, float g0 = -1, float g1 = -1, float g2 = -1, float g3 = -1)
{
    CHI_ProfileShape shape;
    CHI_InitProfileShape(&shape, preset);
    const float g[4] = {g0, g1, g2, g3};
    for (int i = 0; i < 4; ++i) {
        if (g[i] >= 0) {
            shape.gain[i] = g[i];
        }
    }
    std::vector<float> table;
    if (!chiaro::detail::BuildShapeProfile(shape, table)) {
        std::abort();
    }
    // The table before the atlas: a mismatch of the atlas hash then tells table from upload.
    {
        std::uint64_t h = 0xcbf29ce484222325ull;
        const auto *b = reinterpret_cast<const Uint8 *>(table.data());
        for (size_t i = 0; i < table.size() * sizeof(float); ++i) {
            h = (h ^ b[i]) * 0x100000001b3ull;
        }
        char line[96];
        std::snprintf(line, sizeof line, "table profile %u %zu %016llx\n", static_cast<unsigned>(tableCount++),
                      table.size(), static_cast<unsigned long long>(h));
        tables += line;
        if (const char *dir = std::getenv("ATLAS_TABLE_DIR")) {
            const std::string path = std::string(dir) + "/table" + std::to_string(tableCount - 1) + ".bin";
            if (std::FILE *f = std::fopen(path.c_str(), "wb")) {
                std::fwrite(b, 1, table.size() * sizeof(float), f);
                std::fclose(f);
            }
        }
    }
    const Uint32 id = ctx->CreateProfile(table);
    if (id == 0) {
        std::abort();
    }
    return id;
}

Uint32 MaskShape(CHI_MaskPreset preset, CHI_Color glass)
{
    CHI_MaskShape shape;
    chiaro::detail::InitMaskShape(shape, preset);
    shape.glass = glass;
    if (!chiaro::detail::ValidateMaskShape(shape)) {
        std::abort();
    }
    std::vector<Uint8> texels;
    chiaro::detail::BuildShapeMask(shape, texels);
    const Uint32 id = ctx->CreateMask(std::move(texels));
    if (id == 0) {
        std::abort();
    }
    return id;
}

/** A 16 x 1 gobo: left half white, right half black. */
Uint32 HalfGobo()
{
    std::vector<Uint8> px(16 * 4, 255);
    for (int x = 8; x < 16; ++x) {
        px[static_cast<size_t>(x) * 4] = px[static_cast<size_t>(x) * 4 + 1] = px[static_cast<size_t>(x) * 4 + 2] = 0;
    }
    std::vector<Uint8> texels;
    if (!chiaro::detail::BuildPixelMask(16, 1, px.data(), 16 * 4, texels)) {
        std::abort();
    }
    const Uint32 id = ctx->CreateMask(std::move(texels));
    if (id == 0) {
        std::abort();
    }
    return id;
}

void Zone(CHI_AmbientZoneDesc d)
{
    const chiaro::detail::AmbientZoneParams p = chiaro::detail::ZoneParamsFromDesc(d);
    if (!chiaro::detail::ValidateZone(p, "atlas_ref") ||
        ctx->AmbientZones().Create(chiaro::detail::MakeStoredZone(p)) == 0) {
        std::abort();
    }
}

CHI_AmbientZoneDesc Box(float x, float y, float w, float d, CHI_Color c, float soft = 0.0f, Sint32 priority = 0,
                        float rotation = 0.0f, bool relative = false)
{
    CHI_AmbientZoneDesc z;
    CHI_AmbientBox(&z, x, y, w, d, c);
    z.soft_edge = soft;
    z.priority = priority;
    z.rotation = rotation;
    if (relative) {
        z.flags |= CHI_AMBIENT_ZONE_RELATIVE;
    }
    return z;
}

CHI_AmbientZoneDesc Disc(float x, float y, float r, CHI_Color c, float soft = 0.0f, Sint32 priority = 0)
{
    CHI_AmbientZoneDesc z;
    CHI_AmbientCircle(&z, x, y, r, c);
    z.soft_edge = soft;
    z.priority = priority;
    return z;
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

std::uint64_t Fnv1a(const std::vector<Uint8> &bytes)
{
    std::uint64_t h = 0xcbf29ce484222325ull;
    for (Uint8 b : bytes) {
        h = (h ^ b) * 0x100000001b3ull;
    }
    return h;
}

std::string programs, atlases;

void Atlas(const char *scene, const char *kind, const chiaro::detail::TextureInfo &t, Uint32 bytesPerPixel)
{
    if (!t.texture) {
        return;
    }
    char line[160];
    std::snprintf(line, sizeof line, "%s %s %u %u %016llx\n", scene, kind, t.width, t.height,
                  static_cast<unsigned long long>(Fnv1a(DownloadBytes(t.texture, t.width, t.height, bytesPerPixel))));
    atlases += line;
}

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
        // An empty material buffer: the direction mode (light resolution below 1) reads only the base normal.
        mat = CreateTestTexture(kW, kH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER);
        UploadPixels(mat, kW, kH, std::vector<Uint8>(static_cast<size_t>(kW) * kH * 4, 0));
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
    if (inspect.Direction().texture) {
        DumpTexture(out + "/" + name + ".direction.bin", inspect.Direction());
    }
    if (inspect.PureAmbient().texture) {
        DumpTexture(out + "/" + name + ".ambient.bin", inspect.PureAmbient());
    }
    Atlas(name, "profile", inspect.ProfileAtlas(), 4);
    Atlas(name, "mask", inspect.MaskAtlas(), 4);
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

/** Three profiles (blocks 0 to 2): a hat lamp, a shadowed street lamp spot turned by 30 degrees, a downlight. */
void SceneProfile()
{
    Ambient(0.02f, 0.02f, 0.02f, 1.0f);
    Polygon({56, 30, 62, 30, 62, 42, 56, 42}, 0.0f, 1.0f);
    Circle(3.0f, 50.0f, 14.0f, 0.0f);
    const Uint32 hat = Profile(CHI_PROFILE_HAT_LAMP, -1, 0.5f, -1, 0.0f);
    const Uint32 street = Profile(CHI_PROFILE_LED_STREET);
    const Uint32 down = Profile(CHI_PROFILE_DOWNLIGHT);
    Add({.x = 28, .y = 24, .height = 8, .radius = 40, .intensity = 2, .direction = {0, 0, -1}, .profile = hat});
    Add({.type = CHI_LIGHT_SPOT, .x = 72, .y = 22, .height = 8, .radius = 45, .color = {1.0f, 0.9f, 0.7f},
         .shadows = true, .direction = {0.2f, 0.3f, -1}, .inner = 20, .outer = 40, .profile = street,
         .profileRotation = 30});
    Add({.x = 14, .y = 38, .height = 6, .radius = 20, .direction = {0, 0, -1}, .profile = down});
    Add({.x = 90, .y = 8, .radius = 12});
}

/** A projector cross, a half gobo, and a shadowed window lamp with blue glass (mask blocks 0 to 2). */
void SceneMask()
{
    Ambient(0.02f, 0.02f, 0.02f, 1.0f);
    Polygon({40, 14, 44, 14, 44, 34, 40, 34}, 0.0f, 1.0f);
    const Uint32 cross = MaskShape(CHI_MASK_WINDOW_CROSS, CHI_Color{1.0f, 1.0f, 1.0f});
    const Uint32 gobo = HalfGobo();
    const Uint32 blue = MaskShape(CHI_MASK_WINDOW_CROSS, CHI_Color{0.5f, 0.8f, 1.0f});
    Add({.x = 66, .y = 22, .height = 8, .radius = 40, .intensity = 2, .direction = {0, 0, -1}, .mask = cross,
         .maskAngle = 60});
    Add({.x = 84, .y = 40, .height = 6, .radius = 14, .color = {1.0f, 0.9f, 0.8f}, .direction = {0, 0, -1},
         .mask = gobo, .maskAngle = 45});
    Add({.x = 14, .y = 24, .height = 5, .radius = 50, .intensity = 2, .shadows = true, .direction = {0, 0, -1},
         .mask = blue, .maskAngle = 60, .window = true, .maskCenter = {26, 24, 3}, .maskU = {0, 4, 0},
         .maskV = {0, 0, 3}});
}

/** Flat mode: box, circle, concave polygon, a relative and a rotated box, lights on top, a flat directional sum. */
void SceneZones()
{
    Ambient(0.3f, 0.35f, 0.4f, 1.0f);
    Polygon({60, 8, 64, 8, 64, 20, 60, 20}, 0.0f, 1.0f);
    Add({.x = 30, .y = 24, .radius = 30, .intensity = 1.2f});
    Add({.x = 70, .y = 14, .radius = 28, .color = {1.0f, 0.8f, 0.6f}, .source = 2, .shadows = true});
    Add({.type = CHI_LIGHT_DIRECTIONAL, .intensity = 0.1f, .direction = {0.3f, 0.4f, -1.0f}});
    Zone(Box(24, 24, 28, 20, CHI_Color{0.25f, 0.5f, 1.0f}, 4.0f));
    Zone(Disc(66, 14, 12, CHI_Color{0.1f, 0.1f, 0.1f}, 0.0f, 1));
    const float l[] = {50, 28, 90, 28, 90, 44, 74, 44, 74, 36, 50, 36};
    CHI_AmbientZoneDesc poly;
    CHI_AmbientPolygon(&poly, l, 6, CHI_Color{0.5f, 0.2f, 0.1f});
    poly.soft_edge = 2.0f;
    Zone(poly);
    Zone(Box(78, 12, 24, 16, CHI_Color{0.5f, 0.5f, 0.5f}, 0.0f, 0, 0.0f, true));
    Zone(Box(40, 36, 20, 12, CHI_Color{0.2f, 0.4f, 0.2f}, 0.0f, 0, 30.0f));
}

/** Zones without any light: no light pass, the light buffer holds the ambient of each pixel. */
void SceneZonesOnly()
{
    Ambient(0.1f, 0.1f, 0.1f, 1.0f);
    Zone(Box(48, 24, 40, 24, CHI_Color{0.25f, 0.5f, 1.0f}, 6.0f));
    Zone(Disc(20, 12, 10, CHI_Color{0.8f, 0.6f, 0.2f}, 3.0f));
}

/** A material buffer at half light resolution (direction mode): the zone pass also writes the pure ambient. */
void SceneZonesDirection()
{
    Ambient(0.3f, 0.3f, 0.3f, 0.5f);
    Add({.x = 30, .y = 24, .height = 8, .radius = 40});
    Zone(Box(48, 24, 60, 30, CHI_Color{0.1f, 0.1f, 0.1f}, 4.0f));
    Zone(Disc(80, 12, 10, CHI_Color{0.6f, 0.2f, 0.1f}));
}

} // namespace

int main(int argc, char **argv)
{
    if (argc < 2 || !InitTestDevice()) {
        std::fprintf(stderr, "usage: atlas_ref <outdir> (CHIARO_TEST_GPU_DRIVER=vulkan)\n");
        return 2;
    }
    const std::string out = argv[1];
    const auto run = [&](const char *name, void (*scene)(), bool material) {
        ctx = Context::Create(TestDevice());
        scene();
        Render(out, name, material);
        ctx.reset();
    };
    run("profile", SceneProfile, false);
    run("mask", SceneMask, false);
    run("zones", SceneZones, false);
    run("zones_only", SceneZonesOnly, false);
    run("zones_direction", SceneZonesDirection, true);
    Dump(out + "/programs.txt", std::vector<Uint8>(programs.begin(), programs.end()));
    Dump(out + "/atlases.txt", std::vector<Uint8>(atlases.begin(), atlases.end()));
    Dump(out + "/tables.txt", std::vector<Uint8>(tables.begin(), tables.end()));
    return 0;
}
