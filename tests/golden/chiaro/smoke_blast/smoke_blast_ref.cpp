// smoke_blast reference (Knorke Chiaro port): the scene of examples/smoke_demo/main.cpp of Chiaro @ 9bc5b9b at
// --time 2.5 (default options: medium blast, haze with shafts, shockwave, dust, grenade smoke and ground fog), rendered
// three times like its --screenshot frame (frame 3):
//   smoke_blast_ref <out_dir>
// writes <out_dir>/smoke_blast.txt (the resolved scene in the format of
// crates/knorke-integration/tests/common/demo_scene.rs: animations, lights and their samples, obstacles, surface boxes,
// medium draws, plus `explosion x y age power reach` and the `shockwave` desc it gave), the renderer layers after End
// as raw RGBA8 (640x360, row-major, no header: scene.rgba, glow.rgba, surface.rgba, medium.rgba), puffs.rgba (the
// eight shapes, 128x1024, as ../medium/puffs.png), medium_gpu.rgba (the same clouds drawn into GPU layers, the path
// Knorke ports; it differs from the renderer layer's medium.rgba in 3 densities by 1) and frame1.rgba / frame3.rgba
// (the 1280x720 R8G8B8A8_UNORM frames; both equal tests/golden/chiaro/smoke_blast.png).
// The scene code is copied from the demo with only the options, keys, smoke and measure modes removed; change both
// together. Build and run: build.bat next to this file (MSVC x64, against the out-of-tree Chiaro 9bc5b9b build).
// The layer dumps become scene.png, glow.png, surface.png, medium.png and medium_gpu.png next to this file (Pillow:
// Image.frombytes("RGBA", ...)), read by crates/knorke-integration/tests/gpu_smoke_blast.rs with smoke_blast.txt.
//
// Origin of the scene code: chiaro @ 9bc5b9b, examples/smoke_demo/main.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as such
// (this file is altered: options, input, smoke and measure modes removed, scene dump added) and this notice may not
// be removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

#include <Chiaro/Chiaro.hpp>
#include "SmokePuffs.hpp"

#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <optional>
#include <string>
#include <vector>

namespace {

constexpr int kW = 640;
constexpr int kH = 360;
constexpr float kHeightStep = 0.1f;
constexpr float kZoom = 17.0f;
constexpr float kTime = 2.5f;

constexpr float kFirstBlast = 1.0f;
constexpr float kGrenadeDelay = 4.0f;

struct V2
{
    float x, y;
};

constexpr V2 kBlast{7.0f, 8.0f};
constexpr V2 kGrenade{2.5f, 11.0f};
constexpr float kWallY = 3.0f, kWallDepth = 0.3f, kWallHeight = 3.0f;
constexpr float kGapX0 = 14.2f, kGapX1 = 15.8f;
constexpr V2 kShaftLamp{15.0f, 1.2f};
constexpr int kDustCount = 220;
constexpr int kColumnCount = 160;
constexpr int kFogCount = 120;
constexpr V2 kFog{3.0f, 5.0f};

constexpr Uint32 kMediumDust = 0, kMediumSmoke = 1, kMediumFog = 4;
constexpr float kShockReach = 6.0f;
constexpr float kPowerMedium = 1.0f;

using demo::Hash;

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

void BoxColors(SDL_Renderer *r, const chiaro::Projection &p, float x, float y, float w, float d, float h, SDL_FColor c)
{
    const float x1 = x + w, y1 = y + d;
    Quad(r, p, {{{x, y1, 0.0f}, {x1, y1, 0.0f}, {x1, y1, h}, {x, y1, h}}}, c);
    const SDL_FColor side{c.r * 0.8f, c.g * 0.8f, c.b * 0.8f, 1.0f};
    Quad(r, p, {{{x1, y, 0.0f}, {x1, y1, 0.0f}, {x1, y1, h}, {x1, y, h}}}, side);
    Flat(r, p, x, y, x1, y1, h, SDL_FColor{c.r * 0.55f, c.g * 0.55f, c.b * 0.55f, 1.0f});
}

struct Wall
{
    float x, y, w, d, h;
};

constexpr std::array<Wall, 5> kWalls{{{9.0f, -3.0f, 11.0f, 3.0f, 5.0f},
                                      {9.0f, 0.0f, 0.3f, kWallY, kWallHeight},
                                      {19.7f, 0.0f, 0.3f, kWallY, kWallHeight},
                                      {9.0f, kWallY, kGapX0 - 9.0f, kWallDepth, kWallHeight},
                                      {kGapX1, kWallY, 20.0f - kGapX1, kWallDepth, kWallHeight}}};

float Smooth(float a, float b, float x)
{
    const float t = std::clamp((x - a) / (b - a), 0.0f, 1.0f);
    return t * t * (3.0f - 2.0f * t);
}

void DustDraws(const chiaro::Projection &p, float age, demo::PuffSet &puffs, std::vector<chiaro::MediumDraw> &out)
{
    if (age < 0.0f) {
        return;
    }
    for (int i = 0; i < kDustCount; ++i) {
        const Uint32 k = static_cast<Uint32>(i) * 11u;
        const float birth = 0.3f * Hash(k + 1u);
        const float life = 9.0f + 9.0f * Hash(k + 2u);
        const float a = age - birth;
        if (a < 0.0f || a > life) {
            continue;
        }
        const float u = a / life;
        const float angle = Hash(k + 3u) * 6.2831853f;
        const float speed = 1.5f + 5.5f * Hash(k + 4u);
        const float reach = speed * (1.0f - std::exp(-a * 0.7f)) / 0.7f;
        const float swirl = 0.25f * std::sin(a * 0.8f + Hash(k + 5u) * 6.0f);
        const float rise = (1.0f + 3.0f * Hash(k + 6u)) * (1.0f - std::exp(-a * 1.2f));
        const float sink = 0.12f * a * a / life;
        const float z = std::max(0.15f, rise - sink + 0.3f);
        const float radius = 0.35f + 0.5f * Hash(k + 7u) + (0.5f + 0.8f * Hash(k + 8u)) * std::sqrt(a);
        const float fadeIn = Smooth(0.0f, 0.25f, a);
        const float fadeOut = 1.0f - Smooth(0.45f, 1.0f, u);
        const float density = (0.08f + 0.2f * Hash(k + 9u)) * fadeIn * fadeOut;
        demo::AddPuff(out, p, puffs.Pick(k), kBlast.x + std::cos(angle + swirl) * reach,
                      kBlast.y + std::sin(angle + swirl) * reach, z, radius, density, kMediumDust);
    }
}

void ColumnDraws(const chiaro::Projection &p, float age, demo::PuffSet &puffs, std::vector<chiaro::MediumDraw> &out)
{
    if (age < 0.0f) {
        return;
    }
    for (int i = 0; i < kColumnCount; ++i) {
        const Uint32 k = static_cast<Uint32>(i) * 13u + 5000u;
        const float born = 12.0f * static_cast<float>(i) / static_cast<float>(kColumnCount);
        const float life = 8.0f + 4.0f * Hash(k + 1u);
        const float a = age - born;
        if (a < 0.0f || a > life) {
            continue;
        }
        const float u = a / life;
        const float angle = Hash(k + 2u) * 6.2831853f;
        const float spread = (0.15f + 0.5f * Hash(k + 3u)) * std::sqrt(a) * 1.4f;
        const float z = 0.3f + 3.2f * (1.0f - std::exp(-a * 0.45f)) * (0.7f + 0.3f * Hash(k + 4u));
        const float wind = 0.35f * a * (0.5f + z * 0.2f);
        const float radius = 0.4f + 0.45f * std::sqrt(a) + 0.4f * Hash(k + 5u);
        const float density = (0.12f + 0.18f * Hash(k + 6u)) * Smooth(0.0f, 0.4f, a) * (1.0f - Smooth(0.5f, 1.0f, u));
        demo::AddPuff(out, p, puffs.Pick(k), kGrenade.x + std::cos(angle) * spread + wind,
                      kGrenade.y + std::sin(angle) * spread, z, radius, density, kMediumSmoke);
    }
}

/** The bytes of a whole R8G8B8A8 GPU texture; empty on failure. */
std::vector<Uint8> Download(SDL_GPUDevice *device, SDL_GPUTexture *tex, Uint32 w, Uint32 h)
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
    std::vector<Uint8> bytes;
    if (const auto *mapped = static_cast<const Uint8 *>(SDL_MapGPUTransferBuffer(device, tb, false))) {
        bytes.assign(mapped, mapped + tbi.size);
        SDL_UnmapGPUTransferBuffer(device, tb);
    }
    SDL_ReleaseGPUTransferBuffer(device, tb);
    return bytes;
}

bool WriteRaw(const std::vector<Uint8> &bytes, const std::string &path)
{
    FILE *f = bytes.empty() ? nullptr : std::fopen(path.c_str(), "wb");
    if (!f) {
        return false;
    }
    const bool ok = std::fwrite(bytes.data(), 1, bytes.size(), f) == bytes.size();
    return std::fclose(f) == 0 && ok;
}

SDL_GPUTexture *GpuTexture(SDL_Texture *t)
{
    return t ? static_cast<SDL_GPUTexture *>(SDL_GetPointerProperty(SDL_GetTextureProperties(t),
                                                                    SDL_PROP_TEXTURE_GPU_TEXTURE_POINTER, nullptr))
             : nullptr;
}

Uint32 IndexOf(const std::vector<Uint32> &ids, Uint32 id)
{
    if (id == 0) {
        return 0;
    }
    const auto it = std::find(ids.begin(), ids.end(), id);
    return it == ids.end() ? 0xffffffffu : static_cast<Uint32>(it - ids.begin()) + 1;
}

/** A light line as in iso_night_ref.cpp's WriteLight (the scene has no profiles and masks). */
void WriteLight(FILE *f, const CHI_LightDesc &d, const std::vector<Uint32> &animations)
{
    std::fprintf(f, "light %d %u", static_cast<int>(d.type), d.flags);
    for (float v : {d.x, d.y, d.height, d.color.r, d.color.g, d.color.b, d.intensity, d.specular, d.haze, d.radius,
                    d.falloff, d.source_size, d.flicker, d.flicker_speed, d.direction.x, d.direction.y, d.direction.z,
                    d.inner_angle, d.outer_angle, d.axis.x, d.axis.y, d.axis.z, d.half_width}) {
        std::fprintf(f, " %.9g", v);
    }
    std::fprintf(f, " 0 %.9g %u %.9g 0 %d", d.profile_rotation, IndexOf(animations, d.animation.id),
                 d.animation_offset, static_cast<int>(d.mask_mode));
    for (float v : {d.mask_angle, d.mask_rotation, d.mask_center.x, d.mask_center.y, d.mask_center.z, d.mask_u.x,
                    d.mask_u.y, d.mask_u.z, d.mask_v.x, d.mask_v.y, d.mask_v.z}) {
        std::fprintf(f, " %.9g", v);
    }
    std::fprintf(f, "\n");
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
    SDL_Window *window = device ? SDL_CreateWindow("smoke_blast_ref", 1280, 720, 0) : nullptr;
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
    fx->SetAmbient(chiaro::Color(0.07f, 0.08f, 0.12f));
    fx->SetScaling(CHI_SCALE_INTEGER, CHI_FILTER_NEAREST);
    fx->SetTonemap(CHI_TONEMAP_SOFT);
    fx->SetHeightStep(kHeightStep);
    fx->SetBloom(0.5f);
    fx->SetGlowScale(2.0f);
    const bool configured =
        fx->SetQuality(CHI_QUALITY_EXACT) && fx->Edit([&](chiaro::Settings &s) {
            s.quality.medium_filter = CHI_FILTER_LINEAR;
            s.haze.density = 0.015f;
            s.haze.color = chiaro::Color(0.85f, 0.88f, 1.0f);
            s.haze.flags = 0u;
        });
    std::optional<chiaro::Layers> layers =
        fx->CreateLayers(renderer, kW, kH, chiaro::Layer::Surface | chiaro::Layer::Glow | chiaro::Layer::Medium);
    const chiaro::Projection projection = chiaro::IsoProjection(11.0f, 7.0f, kZoom, 1.0f, kW, kH);
    demo::PuffSet puffs;
    if (!configured || !layers || !layers->SetProjection(projection) || !puffs.Create(renderer)) {
        SDL_Log("setup failed: %s", chiaro::GetError());
        return 1;
    }

    std::vector<chiaro::ShapeDesc> shapes;
    for (const Wall &w : kWalls) {
        shapes.push_back(chiaro::Box(w.x, w.y, w.w, w.d, w.h));
    }
    shapes.push_back(chiaro::Box(kBlast.x + 1.25f, kBlast.y - 1.15f, 0.7f, 0.7f, 1.0f));
    chiaro::Obstacles obstacles = fx->CreateShapes(shapes);

    chiaro::AnimationDesc flashDesc(chiaro::AnimationType::Envelope);
    flashDesc.start = kFirstBlast;
    flashDesc.period = 0.0f;
    flashDesc.minimum = 0.0f;
    flashDesc.attack = 0.04f;
    flashDesc.hold = 0.12f;
    flashDesc.decay = 0.9f;
    chiaro::AnimationDesc emberDesc(chiaro::AnimationType::Envelope);
    emberDesc.start = kFirstBlast;
    emberDesc.period = 0.0f;
    emberDesc.minimum = 0.0f;
    emberDesc.attack = 0.4f;
    emberDesc.hold = 5.0f;
    emberDesc.decay = 9.0f;
    std::optional<chiaro::AnimationHandle> flashAnim = fx->CreateAnimation(flashDesc);
    std::optional<chiaro::AnimationHandle> emberAnim = fx->CreateAnimation(emberDesc);
    if (!obstacles || !flashAnim || !emberAnim) {
        SDL_Log("scene failed: %s", chiaro::GetError());
        return 1;
    }
    std::vector<chiaro::LightDesc> lightDescs;
    lightDescs.push_back(chiaro::PointLight(kBlast.x, kBlast.y, 10.0f, chiaro::Color(1.0f, 0.75f, 0.45f), 7.0f)
                             .Height(1.2f)
                             .SourceSize(0.4f)
                             .Haze(3.0f)
                             .Animation(flashAnim->id));
    lightDescs.push_back(chiaro::PointLight(kBlast.x, kBlast.y, 4.5f, chiaro::Color(1.0f, 0.4f, 0.12f), 1.6f)
                             .Height(0.4f)
                             .Shadows(false)
                             .Flicker(0.45f, 9.0f)
                             .Haze(4.0f)
                             .Animation(emberAnim->id));
    lightDescs.push_back(chiaro::PointLight(kShaftLamp.x, kShaftLamp.y, 13.0f, chiaro::Color(1.0f, 0.92f, 0.75f), 3.0f)
                             .Height(2.2f)
                             .SourceSize(0.1f)
                             .Haze(1.5f));
    lightDescs.push_back(
        chiaro::PointLight(kGrenade.x + 3.0f, kGrenade.y - 1.5f, 9.0f, chiaro::Color(0.7f, 0.85f, 1.0f), 1.6f)
            .Height(3.5f)
            .SourceSize(0.15f)
            .Haze(1.5f));
    chiaro::Lights lights = fx->CreateLights(lightDescs);
    if (!lights) {
        SDL_Log("lights failed: %s", chiaro::GetError());
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
    SDL_GPUTexture *offTex = SDL_CreateGPUTexture(device, &ti);
    const float now = kTime;
    const float blastAge = now - kFirstBlast;
    const float grenadeAge = blastAge - kGrenadeDelay;
    const chiaro::ShockwaveDesc wave = chiaro::ExplosionShockwave(kBlast.x, kBlast.y, blastAge, kPowerMedium, kShockReach);
    std::vector<chiaro::MediumDraw> clouds;
    int failures = 0;
    for (int frame = 1; frame <= 3; ++frame) {
        failures += fx->SetFrameShockwaves({wave}) ? 0 : 1;
        SDL_SetRenderDrawBlendMode(renderer, SDL_BLENDMODE_NONE);
        bool drawn = layers->Begin();
        Flat(renderer, projection, -6.0f, -4.0f, 24.0f, 20.0f, 0.0f, SDL_FColor{0.17f, 0.18f, 0.20f, 1.0f});
        Flat(renderer, projection, -6.0f, -0.5f, 9.0f, 0.5f, 0.0f, SDL_FColor{0.28f, 0.28f, 0.30f, 1.0f});
        for (int i = 0; i <= 30; ++i) {
            const float c = static_cast<float>(i) - 6.0f;
            const SDL_FColor seam{0.11f, 0.115f, 0.13f, 1.0f};
            Flat(renderer, projection, c - 0.03f, 4.0f, c + 0.03f, 20.0f, 0.0f, seam);
            if (i <= 16) {
                Flat(renderer, projection, -6.0f, c + 4.0f - 0.03f, 24.0f, c + 4.0f + 0.03f, 0.0f, seam);
            }
        }
        for (const Wall &w : kWalls) {
            BoxColors(renderer, projection, w.x, w.y, w.w, w.d, w.h, SDL_FColor{0.42f, 0.38f, 0.34f, 1.0f});
        }
        BoxColors(renderer, projection, kBlast.x + 1.25f, kBlast.y - 1.15f, 0.7f, 0.7f, 1.0f,
                  SDL_FColor{0.35f, 0.18f, 0.12f, 1.0f});
        for (const Wall &w : kWalls) {
            drawn = drawn && layers->DrawSurfaceBox(w.x, w.y, w.w, w.d, w.h);
        }
        drawn = drawn && layers->DrawSurfaceBox(kBlast.x + 1.25f, kBlast.y - 1.15f, 0.7f, 0.7f, 1.0f);
        drawn = drawn && layers->SetTarget(chiaro::Layer::Glow);
        if (blastAge > 0.0f) {
            const float life = std::clamp(1.0f - (blastAge - 4.0f) / 12.0f, 0.0f, 1.0f);
            for (int i = 0; i < 16; ++i) {
                const Uint32 k = static_cast<Uint32>(i);
                const float a = Hash(k + 500u) * 6.2831853f;
                const float rr = 0.2f + 1.3f * Hash(k + 600u);
                const float flick = 0.6f + 0.4f * std::sin(now * (5.0f + Hash(k + 700u) * 6.0f));
                const float g = life * flick;
                const float x = kBlast.x + std::cos(a) * rr, y = kBlast.y + std::sin(a) * rr;
                Flat(renderer, projection, x - 0.08f, y - 0.08f, x + 0.08f, y + 0.08f, 0.02f,
                     SDL_FColor{1.0f, 0.38f, 0.1f, g});
            }
        }
        clouds.clear();
        DustDraws(projection, blastAge, puffs, clouds);
        ColumnDraws(projection, grenadeAge, puffs, clouds);
        demo::AddFogPatch(clouds, projection, puffs, kFog.x, kFog.y, 3.0f, kFogCount, now, kMediumFog, 1u);
        if (!clouds.empty()) {
            drawn = drawn && layers->DrawMediums(clouds);
        }
        drawn = layers->End() && drawn;
        failures += drawn ? 0 : 1;
        SDL_RenderPresent(renderer);
        SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(device);
        chiaro::Frame f{.layers = *layers, .projection = &projection, .time = static_cast<double>(now)};
        f.target = chiaro::Texture{offTex, nullptr, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, 1280, 720};
        failures += fx->Render(cmd, f) ? 0 : 1;
        SDL_SubmitGPUCommandBuffer(cmd);
        if (frame == 1 || frame == 3) {
            failures += WriteRaw(Download(device, offTex, 1280, 720), out + "/frame" + std::to_string(frame) + ".rgba")
                            ? 0
                            : 1;
        }
    }

    const std::pair<Uint32, const char *> dumps[] = {{chiaro::Layer::Scene, "scene"},
                                                     {chiaro::Layer::Glow, "glow"},
                                                     {chiaro::Layer::Surface, "surface"},
                                                     {chiaro::Layer::Medium, "medium"}};
    for (const auto &[kind, name] : dumps) {
        SDL_Texture *t = layers->GetRendererTexture(kind);
        const bool rgba = t && t->format == SDL_PIXELFORMAT_ABGR8888;
        failures += rgba && WriteRaw(Download(device, GpuTexture(t), kW, kH), out + "/" + name + ".rgba") ? 0 : 1;
    }
    std::vector<Uint8> shapesPx;
    for (Uint32 k = 0; k < 8; ++k) {
        const std::vector<Uint8> px = Download(device, GpuTexture(puffs.Pick(k)->renderer_texture), 128, 128);
        shapesPx.insert(shapesPx.end(), px.begin(), px.end());
    }
    failures += WriteRaw(shapesPx, out + "/puffs.rgba") ? 0 : 1;
    // The same clouds into GPU layers (CHI_FromGPU shapes, the path Knorke's MediumBatch ports): medium_gpu.rgba.
    {
        std::optional<chiaro::Layers> gpuLayers = fx->CreateGPULayers(kW, kH, chiaro::Layer::Medium);
        std::vector<CHI_Texture> gpuShapes;
        for (Uint32 k = 0; k < 8; ++k) {
            gpuShapes.push_back(
                chiaro::FromGPU(GpuTexture(puffs.Pick(k)->renderer_texture), SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, 128, 128));
        }
        std::vector<chiaro::MediumDraw> gpuClouds = clouds;
        for (chiaro::MediumDraw &d : gpuClouds) {
            for (Uint32 k = 0; k < 8; ++k) {
                if (d.texture == puffs.Pick(k)) {
                    d.texture = &gpuShapes[k];
                    break;
                }
            }
        }
        bool ok = gpuLayers && gpuLayers->SetProjection(projection);
        SDL_GPUColorTargetInfo clear[1]{};
        if (ok && gpuLayers->GetColorTargets(chiaro::Layer::Medium, clear) == 1) { // cleared by its own pass
            SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(device);
            SDL_EndGPURenderPass(SDL_BeginGPURenderPass(cmd, clear, 1, nullptr));
            ok = SDL_SubmitGPUCommandBuffer(cmd);
        } else {
            ok = false;
        }
        ok = ok && gpuLayers->DrawMediums(gpuClouds);
        SDL_WaitForGPUIdle(device);
        failures += ok && WriteRaw(Download(device, gpuLayers->GetTexture(chiaro::Layer::Medium).texture, kW, kH),
                                   out + "/medium_gpu.rgba")
                        ? 0
                        : 1;
        if (!ok) {
            SDL_Log("gpu layers failed: %s", chiaro::GetError());
        }
    }

    // The resolved scene (format: crates/knorke-integration/tests/gpu_iso_night.rs).
    FILE *f = std::fopen((out + "/smoke_blast.txt").c_str(), "wb");
    if (!f) {
        return 1;
    }
    std::fprintf(f, "# smoke_blast at t = 2.5, written by tests/golden/chiaro/smoke_blast/smoke_blast_ref.cpp\n");
    std::vector<CHI_LightDesc> descs(lights.Ids().size());
    for (size_t i = 0; i < descs.size(); ++i) {
        failures += CHI_GetLight(fx->Get(), lights.Ids()[i], &descs[i]) ? 0 : 1;
    }
    const std::vector<Uint32> animations = {descs[0].animation.id, descs[1].animation.id}; // flash, embers
    for (const CHI_AnimationDesc &a : {static_cast<CHI_AnimationDesc>(flashDesc), static_cast<CHI_AnimationDesc>(emberDesc)}) {
        std::fprintf(f, "animation %d %.17g %.9g %.9g %.9g %.9g %.9g %.9g %u %.9g %.9g %.9g\n",
                     static_cast<int>(a.type), a.start, a.period, a.minimum, a.low_color[0], a.low_color[1],
                     a.low_color[2], a.duty, a.count, a.attack, a.hold, a.decay);
    }
    for (const CHI_LightDesc &d : descs) {
        WriteLight(f, d, animations);
    }
    for (size_t i = 0; i < 2; ++i) { // the two animated lights come first
        const auto a = fx->SampleLightAnimation(lights.Ids()[i], kTime);
        failures += a ? 0 : 1;
        if (a) {
            std::fprintf(f, "sample %zu %.9g %.9g %.9g %.9g %.9g\n", i, a->intensity, a->color[0], a->color[1],
                         a->color[2], a->rotation);
        }
    }
    for (const chiaro::ShapeDesc &d : shapes) {
        std::fprintf(f, "shape %d %u %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g 0\n", static_cast<int>(d.type),
                     d.flags, d.x, d.y, d.rotation, d.height, d.base_height, d.opacity, d.radius, d.width, d.depth);
    }
    for (const Wall &w : kWalls) {
        std::fprintf(f, "surface_box %.9g %.9g %.9g %.9g %.9g\n", w.x, w.y, w.w, w.d, w.h);
    }
    std::fprintf(f, "surface_box %.9g %.9g %.9g %.9g %.9g\n", kBlast.x + 1.25f, kBlast.y - 1.15f, 0.7f, 0.7f, 1.0f);
    for (const chiaro::MediumDraw &d : clouds) {
        Uint32 puff = 0;
        for (Uint32 k = 0; k < 8 && puff == 0; ++k) {
            puff = d.texture == puffs.Pick(k) ? k + 1 : 0;
        }
        std::fprintf(f, "medium_draw %.9g %.9g %.9g %.9g %.9g %.9g %.9g %u %u\n", d.rect.x, d.rect.y, d.rect.w,
                     d.rect.h, d.density, d.bottom, d.top, d.medium, puff);
    }
    std::fprintf(f, "explosion %.9g %.9g %.9g %.9g %.9g\n", kBlast.x, kBlast.y, blastAge, kPowerMedium, kShockReach);
    std::fprintf(f, "shockwave %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g\n", wave.x, wave.y, wave.radius, wave.width,
                 wave.strength, wave.light.r, wave.light.g, wave.light.b, wave.light_intensity);
    failures += std::fclose(f) == 0 ? 0 : 1;
    SDL_Log("%d failures, %zu medium draws", failures, clouds.size());

    lights.Reset();
    obstacles.Reset();
    layers.reset();
    fx.reset();
    puffs.Reset();
    SDL_ReleaseGPUTexture(device, offTex);
    SDL_DestroyRenderer(renderer);
    SDL_ReleaseWindowFromGPUDevice(device, window);
    SDL_DestroyWindow(window);
    SDL_DestroyGPUDevice(device);
    SDL_Quit();
    return failures == 0 ? 0 : 1;
}
