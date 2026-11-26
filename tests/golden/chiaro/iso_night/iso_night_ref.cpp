// iso_night reference (Knorke Chiaro port): the scene of examples/iso_night_demo of Chiaro @ 9bc5b9b (plus the one
// Scene.cpp fix below) at --time 5,
// in the five variants of the P3 goldens, rendered once like the demo's --screenshot frame:
//   plain: --no-heights --no-surface --no-haze --no-fog --no-moon-shadows      hard: --hard --no-haze
//   t5: no arguments      cutaway: --cutaway      grade_crt: --grade look --crt subtle --dither
//   iso_night_ref <plain|hard|t5|cutaway|grade_crt> <out_dir>
// writes <out_dir>/<variant>.txt (the resolved scene: lights, frame lights, obstacles, patterns and their sheet,
// the ambient zone, mask, profiles, animations and their samples at t = 5, the ground fog's medium draws, the
// output grade; format: crates/knorke-integration/tests/common/demo_scene.rs), the renderer layers after End as raw RGBA8 (640x360, row-major, no header: scene.rgba, glow.rgba,
// surface.rgba, medium.rgba), the eight fog puff shapes (puffs.rgba, 128x1024, variant k in rows 128k ...) and
// frame.rgba (the 1280x720 R8G8B8A8_UNORM frame, compared with tests/golden/chiaro/iso_night_<variant>.png outside
// the HUD rect).
// The demo's Scene.cpp is compiled into this file straight from the Chiaro tree: 9bc5b9b with one fix in
// PatternSheet(), `closed = x < 2;` instead of `closed = x >= 2;` (the slat column at u = 0 is drawn as a slat, so it
// must block; it let light pass before), not committed in Chiaro. `private` is opened
// for it to read the scene's descs. main() repeats what the demo's main.cpp sets up for these variants; change
// both together. The HUD overlay is left out: the golden is compared outside its rect (manifest.toml).
// Build (MSVC x64, against an out-of-tree Chiaro 9bc5b9b build <b>), from a directory outside the repo:
//   cl /O2 /EHsc /std:c++20 /I <chiaro>\include /I <chiaro>\examples\iso_night_demo /I <chiaro>\examples\common
//      /I <b>\_deps\sdl3-src\include iso_night_ref.cpp /link /LIBPATH:<b>\lib\Release Chiaro.lib SDL3.lib
// (build.bat next to this file does that for every variant).
// and run it next to Chiaro.dll and SDL3.dll of <b>\bin\Release (Vulkan: SDL_GPU_DRIVER=vulkan). The layer dumps
// become the PNG fixtures next to this file (Pillow: Image.frombytes("RGBA", ...).save): scene.png and glow.png
// (byte-identical in every variant but cutaway: scene_cutaway.png), surface.png (hard, t5, grade_crt; plain draws no
// surface; cutaway: surface_cutaway.png), medium.png (the fog of t5, cutaway and grade_crt; the same in all three)
// and ../medium/puffs.png, read by crates/knorke-integration/tests/gpu_iso_night.rs together with <variant>.txt.
//
// Origin: chiaro @ 9bc5b9b, examples/iso_night_demo/main.cpp (setup, ApplyGrade, ApplyCrt, the fog patch) and Scene.cpp (the animation descs of
// CreateAnimatedLights, the profile shapes and pattern descs of Create, copied below because they are locals).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any express or
// implied warranty; the origin must not be misrepresented; altered source versions must be plainly marked as such
// (this file is altered: options, input, HUD and media removed, scene dump added) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

// Every header Scene.cpp and Scene.hpp use comes first, so that opening `private` touches only the scene.
#include <Chiaro/Chiaro.hpp>
#include "SmokePuffs.hpp"

#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>

#include <algorithm>
#include <array>
#include <bit>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <functional>
#include <optional>
#include <span>
#include <string>
#include <utility>
#include <vector>

/** The surface draws of the frame in call order: the helper geometry and the figures' surface entries. */
std::vector<std::string> g_surfaceDraws;
SDL_Texture *g_surfaceLayer = nullptr; ///< the surface layer's renderer texture

void RecordSurface(const char *kind, std::initializer_list<float> values)
{
    std::string line = kind;
    char v[32];
    for (float f : values) {
        std::snprintf(v, sizeof(v), " %.9g", f);
        line += v;
    }
    g_surfaceDraws.push_back(line);
}

namespace chiaro {
/** The scene's layers with every helper geometry call recorded. Chiaro queues these and draws them in one batch when
 *  the target changes, after what the renderer drew in between (the figures). Used by a cast of the real Layers:
 *  it adds no data. */
class RecLayers : public Layers
{
public:
    bool DrawSurfaceBox(float x, float y, float width, float depth, float height) noexcept
    {
        RecordSurface("surface_box", {x, y, width, depth, height});
        return Layers::DrawSurfaceBox(x, y, width, depth, height);
    }
    bool DrawSurfaceWall(Vec3 a, Vec3 b, float top) noexcept
    {
        RecordSurface("surface_wall", {a.x, a.y, a.z, b.x, b.y, b.z, top});
        return Layers::DrawSurfaceWall(a, b, top);
    }
    bool DrawSurfaceFloor(std::span<const float> xy, float height) noexcept
    {
        std::string line = "surface_floor";
        char v[32];
        for (float f : xy) {
            std::snprintf(v, sizeof(v), " %.9g", f);
            line += v;
        }
        std::snprintf(v, sizeof(v), " %.9g", height);
        g_surfaceDraws.push_back(line + v);
        return Layers::DrawSurfaceFloor(xy, height);
    }
};
} // namespace chiaro

/** SDL_RenderTexture for Scene.cpp: records the figures' surface entries (the only textures it draws). */
bool RecRenderTexture(SDL_Renderer *r, SDL_Texture *t, const SDL_FRect *src, const SDL_FRect *dst)
{
    if (SDL_GetRenderTarget(r) == g_surfaceLayer && dst) {
        RecordSurface("figure_draw", {dst->x, dst->y, dst->w, dst->h});
    }
    return SDL_RenderTexture(r, t, src, dst);
}

#define _ALLOW_KEYWORD_MACROS
#define private public
#define Layers RecLayers
#define SDL_RenderTexture RecRenderTexture
#include "Scene.cpp"
#undef SDL_RenderTexture
#undef Layers
#undef private

namespace {

constexpr int kW = 640;
constexpr int kH = 360;
constexpr float kTime = 5.0f;
constexpr Uint32 kMediumFog = 4; ///< main.cpp: entry of the media table (CHI_MEDIUM_FOG)

std::vector<chiaro::MediumDraw> g_fogDraws;
CHI_CrtSettings g_crt = chiaro::Crt(CHI_CRT_OFF); ///< the CRT settings of the frame
bool g_graded = false;                           ///< grade_crt: --grade look --crt subtle --dither ///< the ground fog of the frame (t5, cutaway, grade_crt)
demo::PuffSet g_puffs;                      ///< its shapes

std::vector<Uint8> Download(SDL_GPUDevice *device, SDL_GPUTexture *tex, Uint32 w, Uint32 h);
SDL_GPUTexture *GpuTexture(SDL_Texture *t);

/** Index (1-based, 0 = none) of `id` in `ids`. */
Uint32 IndexOf(const std::vector<Uint32> &ids, Uint32 id)
{
    if (id == 0) {
        return 0;
    }
    const auto it = std::find(ids.begin(), ids.end(), id);
    return it == ids.end() ? 0xffffffffu : static_cast<Uint32>(it - ids.begin()) + 1;
}

struct Refs
{
    std::vector<Uint32> profiles, animations, masks;
};

void WriteLight(FILE *f, const char *tag, const CHI_LightDesc &d, const Refs &r)
{
    std::fprintf(f, "%s %d %u", tag, static_cast<int>(d.type), d.flags);
    for (float v : {d.x, d.y, d.height, d.color.r, d.color.g, d.color.b, d.intensity, d.specular, d.haze, d.radius,
                    d.falloff, d.source_size, d.flicker, d.flicker_speed, d.direction.x, d.direction.y, d.direction.z,
                    d.inner_angle, d.outer_angle, d.axis.x, d.axis.y, d.axis.z, d.half_width}) {
        std::fprintf(f, " %.9g", v);
    }
    std::fprintf(f, " %u %.9g %u %.9g %u %d", IndexOf(r.profiles, d.profile.id), d.profile_rotation,
                 IndexOf(r.animations, d.animation.id), d.animation_offset, IndexOf(r.masks, d.mask.id),
                 static_cast<int>(d.mask_mode));
    for (float v : {d.mask_angle, d.mask_rotation, d.mask_center.x, d.mask_center.y, d.mask_center.z, d.mask_u.x,
                    d.mask_u.y, d.mask_u.z, d.mask_v.x, d.mask_v.y, d.mask_v.z}) {
        std::fprintf(f, " %.9g", v);
    }
    std::fprintf(f, "\n");
}

void WriteShape(FILE *f, const CHI_ShapeDesc &d, bool heights, Uint32 pattern)
{
    const bool finite = heights || (d.flags & CHI_OBSTACLE_DISABLED); // ShapeGroup::Apply; the slab keeps its desc
    std::fprintf(f, "shape %d %u %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %u\n", static_cast<int>(d.type),
                 d.flags, d.x, d.y, d.rotation, finite ? d.height : CHI_HEIGHT_INFINITE,
                 finite ? d.base_height : 0.0f, d.opacity, d.radius, d.width, d.depth, pattern);
}

/** The resolved scene for the Knorke test; see crates/knorke-integration/tests/common/demo_scene.rs for the format. */
bool WriteScene(SDL_GPUDevice *device, chiaro::Context &fx, const night::NightScene &s, const std::string &path)
{
    FILE *f = std::fopen(path.c_str(), "w");
    if (!f) {
        return false;
    }
    std::fprintf(f, "# iso_night at t = 5, written by tests/golden/chiaro/iso_night/iso_night_ref.cpp\n");
    Refs refs;
    // Profiles in creation order: hat (posts 4..6), then LED (posts 0..3, 7).
    refs.profiles = {s.m_lampDescs[4].profile.id, s.m_lampDescs[0].profile.id};
    chiaro::ProfileShape hat(chiaro::ProfilePreset::HatLamp);
    hat.cutoff_fade_angle = 20.0f;
    for (const CHI_ProfileShape &p : {static_cast<CHI_ProfileShape>(hat),
                                      static_cast<CHI_ProfileShape>(chiaro::ProfileShape(chiaro::ProfilePreset::LedStreet))}) {
        std::fprintf(f, "profile %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g\n", p.peak_angle, p.peak_angle_c90,
                     p.spread_angle, p.nadir, p.cutoff_angle, p.cutoff_fade_angle, p.gain[0], p.gain[1], p.gain[2],
                     p.gain[3]);
    }
    // Animations in creation order (fire, beacon, blink, roof, flash), as in CreateAnimatedLights.
    std::vector<CHI_LightDesc> animated(s.m_animated.Ids().size());
    for (size_t i = 0; i < animated.size(); ++i) {
        (void)CHI_GetLight(fx.Get(), s.m_animated.Ids()[i], &animated[i]);
    }
    refs.animations = {animated[0].animation.id, animated[1].animation.id, animated[2].animation.id,
                       animated[4].animation.id, animated[5].animation.id};
    chiaro::AnimationDesc fire(chiaro::AnimationType::Fire);
    chiaro::AnimationDesc beacon(chiaro::AnimationType::Beacon);
    beacon.period = 1.2f;
    chiaro::AnimationDesc blink(chiaro::AnimationType::Flash);
    blink.duty = 0.35f;
    blink.minimum = 0.03f;
    chiaro::AnimationDesc roof(chiaro::AnimationType::Flash);
    roof.period = 1.6f;
    roof.duty = 0.05f;
    roof.count = 2;
    chiaro::AnimationDesc flash(chiaro::AnimationType::Envelope);
    flash.start = 3.0;
    flash.period = 9.0f;
    flash.attack = 0.03f;
    flash.hold = 0.06f;
    flash.decay = 1.1f;
    for (const CHI_AnimationDesc &a : {fire, beacon, blink, roof, flash}) {
        std::fprintf(f, "animation %d %.17g %.9g %.9g %.9g %.9g %.9g %.9g %u %.9g %.9g %.9g\n",
                     static_cast<int>(a.type), a.start, a.period, a.minimum, a.low_color[0], a.low_color[1],
                     a.low_color[2], a.duty, a.count, a.attack, a.hold, a.decay);
    }
    refs.masks = {s.m_hallMask.id};
    std::fprintf(f, "mask window_hall\n");

    // Lights in creation order (Create: lamps, windows, boards, fixed, moon, hall; then the animated ones).
    std::vector<CHI_LightID> lights;
    for (const chiaro::Lights *group : {&s.m_lamps, &s.m_windows, &s.m_boards, &s.m_fixed}) {
        lights.insert(lights.end(), group->Ids().begin(), group->Ids().end());
    }
    lights.push_back(s.m_moon.Id());
    lights.push_back(s.m_hall.Id());
    lights.insert(lights.end(), s.m_animated.Ids().begin(), s.m_animated.Ids().end());
    for (CHI_LightID id : lights) {
        CHI_LightDesc d{};
        if (!CHI_GetLight(fx.Get(), id, &d)) {
            std::fclose(f);
            return false;
        }
        WriteLight(f, "light", d, refs);
    }
    for (const chiaro::LightDesc &d : s.m_carLights) {
        WriteLight(f, "frame_light", d, refs);
    }
    // What the library does to the animated lights at t = 5, to check the copied animation descs.
    for (size_t i = 0; i < animated.size(); ++i) {
        const auto a = fx.SampleLightAnimation(s.m_animated.Ids()[i], kTime);
        if (!a) {
            std::fclose(f);
            return false;
        }
        std::fprintf(f, "sample %zu %.9g %.9g %.9g %.9g %.9g\n", i, a->intensity, a->color[0], a->color[1],
                     a->color[2], a->rotation);
    }

    // The pattern sheet (alpha only; its RGB is white) and the two patterns of Create.
    const std::vector<Uint8> sheet = night::PatternSheet();
    std::fprintf(f, "sheet 20 16\n");
    for (int y = 0; y < 16; ++y) {
        std::fprintf(f, "row ");
        for (int x = 0; x < 20; ++x) {
            std::fprintf(f, "%02x", sheet[(static_cast<size_t>(y) * 20 + x) * 4 + 3]);
        }
        std::fprintf(f, "\n");
    }
    std::fprintf(f, "pattern 0 0 4 16 %.9g 0\n", 2.0f * night::kSlat);
    std::fprintf(f, "pattern 4 0 16 16 %.9g %.9g\n", night::kTowerSide, night::kTowerCell);

    // Obstacles in creation order: the hall slab, houses, props, movers, screens (with their patterns).
    const bool heights = s.m_toggles.heights;
    WriteShape(f, chiaro::ShapeDesc(s.m_slabDesc).Disabled(!s.m_toggles.cutaway || !heights), heights, 0);
    for (const night::NightScene::ShapeGroup *g : {&s.m_houses, &s.m_props, &s.m_movers}) {
        for (const chiaro::ShapeDesc &d : g->descs) {
            WriteShape(f, d, heights, 0);
        }
    }
    for (size_t i = 0; i < s.m_screens.descs.size(); ++i) {
        WriteShape(f, s.m_screens.descs[i], heights, s.m_toggles.patterns ? static_cast<Uint32>(i) + 1 : 0);
    }

    const CHI_AmbientZoneDesc &z = s.m_zoneDesc;
    const Uint32 zoneFlags = z.flags | (s.m_toggles.ambientZones ? 0u : CHI_AMBIENT_ZONE_DISABLED);
    std::fprintf(f, "zone %d %u %.9g %.9g %.9g %.9g %d %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g\n",
                 static_cast<int>(z.type), zoneFlags, z.ambient.r, z.ambient.g, z.ambient.b, z.soft_edge, z.priority,
                 z.height, z.base_height, z.x, z.y, z.rotation, z.radius, z.width, z.depth);
    // The surface pass (hard only): the figure sprite's colour (its alpha makes the standing entry), then the draws.
    if (!g_surfaceDraws.empty()) {
        const std::vector<Uint8> figure =
            Download(device, GpuTexture(s.m_figure->GetTexture(chiaro::Layer::Scene).renderer_texture),
                     static_cast<Uint32>(s.m_figureW), static_cast<Uint32>(s.m_figureH));
        std::fprintf(f, "figure %d %d %.9g\n", s.m_figureW, s.m_figureH, night::kFigureHeight);
        for (int y = 0; y < s.m_figureH && !figure.empty(); ++y) {
            std::fprintf(f, "figure_row ");
            for (int x = 0; x < s.m_figureW; ++x) {
                std::fprintf(f, "%02x", figure[(static_cast<size_t>(y) * s.m_figureW + x) * 4 + 3]);
            }
            std::fprintf(f, "\n");
        }
        for (const std::string &line : g_surfaceDraws) {
            std::fprintf(f, "%s\n", line.c_str());
        }
    }
    // The ground fog: one line per medium draw, the shape as the index of its puff (1-based, 0 = none).
    for (const chiaro::MediumDraw &d : g_fogDraws) {
        Uint32 puff = 0;
        for (Uint32 k = 0; k < 8 && puff == 0; ++k) {
            puff = d.texture == g_puffs.Pick(k) ? k + 1 : 0;
        }
        std::fprintf(f, "medium_draw %.9g %.9g %.9g %.9g %.9g %.9g %.9g %u %u\n", d.rect.x, d.rect.y, d.rect.w,
                     d.rect.h, d.density, d.bottom, d.top, d.medium, puff);
    }
    // The output grade (CHI_OutputSettings after ApplyGrade; the tint is CHI_Kelvin's) and the CRT settings, when
    // graded (plain and hard keep their text of before).
    if (!g_graded) {
        return std::fclose(f) == 0;
    }
    const chiaro::Settings st = fx.GetSettings();
    const CHI_OutputSettings &o = st.output;
    std::fprintf(f, "output %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %.9g %u\n", o.saturation, o.contrast,
                 o.tint.r, o.tint.g, o.tint.b, o.vignette, o.vignette_radius, o.vignette_softness, o.vignette_color.r,
                 o.vignette_color.g, o.vignette_color.b, (o.flags & CHI_OUTPUT_DITHER) ? 1u : 0u);
    const CHI_CrtSettings &c = g_crt;
    std::fprintf(f, "crt %u %.9g %.9g %.9g %.9g %.9g %.9g %.9g\n", c.flags, c.curvature, c.corner_radius, c.scanlines,
                 c.scanline_period, c.mask, c.mask_pitch, c.brightness);
    const bool ok = std::fclose(f) == 0;
    return ok;
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

/** Downloads a whole R8G8B8A8 GPU texture and writes its bytes to `path`. */
bool DumpRaw(SDL_GPUDevice *device, SDL_GPUTexture *tex, Uint32 w, Uint32 h, const std::string &path)
{
    const std::vector<Uint8> bytes = Download(device, tex, w, h);
    FILE *f = bytes.empty() ? nullptr : std::fopen(path.c_str(), "wb");
    if (!f) {
        return false;
    }
    const bool ok = std::fwrite(bytes.data(), 1, bytes.size(), f) == bytes.size();
    return std::fclose(f) == 0 && ok;
}

/** The GPU texture behind a renderer texture. */
SDL_GPUTexture *GpuTexture(SDL_Texture *t)
{
    return static_cast<SDL_GPUTexture *>(
        SDL_GetPointerProperty(SDL_GetTextureProperties(t), SDL_PROP_TEXTURE_GPU_TEXTURE_POINTER, nullptr));
}

/** The eight puff shapes one below the other (128 x 1024 RGBA8). */
bool DumpPuffs(SDL_GPUDevice *device, const std::string &path)
{
    std::vector<Uint8> all;
    for (Uint32 k = 0; k < 8; ++k) {
        const std::vector<Uint8> px = Download(device, GpuTexture(g_puffs.Pick(k)->renderer_texture), 128, 128);
        if (px.empty()) {
            return false;
        }
        all.insert(all.end(), px.begin(), px.end());
    }
    FILE *f = std::fopen(path.c_str(), "wb");
    if (!f) {
        return false;
    }
    const bool ok = std::fwrite(all.data(), 1, all.size(), f) == all.size();
    return std::fclose(f) == 0 && ok;
}

/** The GPU texture behind a renderer layer; fails unless it is R8G8B8A8_UNORM (bytes as Knorke's layers). */
SDL_GPUTexture *LayerGpuTexture(const chiaro::Layers &layers, Uint32 kind)
{
    SDL_Texture *t = layers.GetRendererTexture(kind);
    if (!t || t->format != SDL_PIXELFORMAT_ABGR8888) {
        SDL_Log("layer %u: no RGBA32 renderer texture (format 0x%08x)", kind, t ? t->format : 0u);
        return nullptr;
    }
    return GpuTexture(t);
}

} // namespace

int main(int argc, char **argv)
{
    const std::string variant = argc == 3 ? argv[1] : "";
    const bool plain = variant == "plain", hard = variant == "hard";
    g_graded = variant == "grade_crt";
    if (!plain && !hard && !g_graded && variant != "t5" && variant != "cutaway") {
        SDL_Log("usage: %s <plain|hard|t5|cutaway|grade_crt> <out_dir>", argv[0]);
        return 1;
    }
    const std::string out = argv[2];
    // plain and hard run without haze and fog (--no-haze), the others with both.
    const bool haze = !plain && !hard;
    night::Toggles toggles;
    if (plain) {
        toggles.heights = false;
        toggles.surface = false;
        toggles.moonShadows = false;
    } else if (hard) {
        toggles.soft = false;
    }
    toggles.cutaway = variant == "cutaway";
    if (!SDL_Init(SDL_INIT_VIDEO)) {
        return 1;
    }
    SDL_GPUDevice *device = SDL_CreateGPUDevice(
        SDL_GPU_SHADERFORMAT_SPIRV | SDL_GPU_SHADERFORMAT_DXIL | SDL_GPU_SHADERFORMAT_MSL, false, nullptr);
    SDL_Window *window = device ? SDL_CreateWindow("iso_night_ref", 1280, 720, 0) : nullptr;
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
    std::optional<chiaro::Layers> layers =
        fx->CreateLayers(renderer, kW, kH, chiaro::Layer::Surface | chiaro::Layer::Glow | chiaro::Layer::Medium);
    // main.cpp: integer nearest scaling, custom quality (shadow 0.5, light 1, medium the preset's), haze 0.012 with
    // shafts or off (--no-haze) with the demo's haze colour; grade and CRT as ApplyGrade and ApplyCrt set them.
    fx->SetScaling(CHI_SCALE_INTEGER, CHI_FILTER_NEAREST);
    const bool configured = layers && g_puffs.Create(renderer) && fx->Edit([&](chiaro::Settings &s) {
        s.quality.preset = CHI_QUALITY_CUSTOM;
        s.quality.medium_filter = CHI_FILTER_LINEAR;
        s.quality.shadow_resolution = 0.5f;
        s.quality.light_resolution = 1.0f;
        s.haze.density = haze ? 0.012f : 0.0f;
        s.haze.flags = 0u;
        s.haze.color = chiaro::Color(0.8f, 0.85f, 1.0f);
        if (g_graded) { // ApplyGrade(Grade::Look, dither on) on the defaults
            s.output.flags |= CHI_OUTPUT_DITHER;
            s.output.saturation = 0.8f;
            s.output.contrast = 1.15f;
            s.output.tint = chiaro::Color::Kelvin(8000.0f);
            s.output.vignette = 0.6f;
        }
    });
    if (g_graded) { // ApplyCrt(subtle, flat)
        g_crt = chiaro::Crt(CHI_CRT_SUBTLE);
    }
    if (!fx->SetCrt(chiaro::CrtSettings(g_crt))) {
        SDL_Log("crt failed: %s", chiaro::GetError());
        return 1;
    }
    std::optional<night::NightScene> scene(std::in_place);
    if (!configured || !scene->Create(*fx)) {
        SDL_Log("scene failed: %s", chiaro::GetError());
        return 1;
    }
    scene->Apply(toggles);
    scene->Update(*fx, kTime);
    const CHI_Projection projection = chiaro::IsoProjection(18.0f, 14.0f, 10.0f, 1.0f, kW, kH);

    SDL_GPUTextureCreateInfo ti{};
    ti.type = SDL_GPU_TEXTURETYPE_2D;
    ti.format = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
    ti.usage = SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER;
    ti.width = 1280;
    ti.height = 720;
    ti.layer_count_or_depth = 1;
    ti.num_levels = 1;
    SDL_GPUTexture *shotTex = SDL_CreateGPUTexture(device, &ti);
    int failures = 0;
    failures += scene->SetFrameLights(*fx) ? 0 : 1;
    g_surfaceLayer = layers->GetRendererTexture(chiaro::Layer::Surface);
    bool drawn = layers->Begin() && scene->Draw(renderer, static_cast<chiaro::RecLayers &>(*layers), projection);
    if (drawn && haze) {
        // main.cpp: the ground fog in front of the construction site.
        demo::AddFogPatch(g_fogDraws, projection, g_puffs, 38.0f, 18.0f, 4.5f, 150, kTime, kMediumFog);
        drawn = layers->DrawMediums(g_fogDraws);
    }
    drawn = layers->End() && drawn;
    failures += drawn ? 0 : 1;
    SDL_RenderPresent(renderer);
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(device);
    chiaro::Frame lit{.target = chiaro::FromGPU(shotTex, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, 1280, 720),
                      .layers = *layers,
                      .projection = &projection,
                      .time = kTime};
    failures += fx->Render(cmd, lit) ? 0 : 1;
    SDL_SubmitGPUCommandBuffer(cmd);

    const std::pair<Uint32, const char *> dumps[] = {
        {chiaro::Layer::Scene, "scene"},
        {chiaro::Layer::Glow, "glow"},
        {chiaro::Layer::Surface, "surface"},
        {chiaro::Layer::Medium, "medium"}};
    for (const auto &[kind, name] : dumps) {
        SDL_GPUTexture *t = LayerGpuTexture(*layers, kind);
        failures += t && DumpRaw(device, t, kW, kH, out + "/" + name + ".rgba") ? 0 : 1;
    }
    failures += DumpPuffs(device, out + "/puffs.rgba") ? 0 : 1;
    failures += DumpRaw(device, shotTex, 1280, 720, out + "/frame.rgba") ? 0 : 1;
    failures += WriteScene(device, *fx, *scene, out + "/" + argv[1] + ".txt") ? 0 : 1;
    SDL_Log("%d failures", failures);

    scene.reset();
    g_puffs.Reset();
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
