/* Post-pass reference (Knorke Chiaro port, package P20): renders three scenes through the output and bloom
 * stages of C++ Chiaro 9bc5b9b and saves the target as PNG.
 *   post_ref <bloom_aces|soft_grade_crt|srgb_stretch> <out.png>
 * The scene is a synthetic 192x128 RGBA8 picture with bright patches; the ambient light of the context lifts
 * it into HDR (no lights). Each config exercises other branches of the output shader: scaling and letterbox,
 * bloom (level chain, snap, coarsest weight), tonemap, grade, vignette, CRT, dither, shockwave, overlay,
 * target encoding. The scenes and settings are mirrored in crates/knorke-post/tests/gpu_parity.rs; change
 * both together.
 * Build (MSVC, against an out-of-tree Chiaro 9bc5b9b build <b>):
 *   cl /O2 /std:c11 /I <chiaro>\include /I <b>\_deps\sdl3-src\include post_ref.c
 *      /link /LIBPATH:<b>\lib\Release Chiaro.lib SDL3.lib
 * Run with Chiaro.dll and SDL3.dll next to the exe. The PNG holds the raw bytes of the target (for an sRGB
 * target the encoded values), alpha 255. */
#include <Chiaro/Chiaro.h>
#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>
#include <stdio.h>
#include <stdlib.h>

#define SW 192
#define SH 128

static SDL_GPUTexture *Upload(SDL_GPUDevice *d, const Uint8 *px, int w, int h, SDL_GPUTextureFormat format,
                              SDL_GPUTextureUsageFlags usage)
{
    SDL_GPUTextureCreateInfo ti = {.type = SDL_GPU_TEXTURETYPE_2D, .format = format, .usage = usage, .width = w,
                                   .height = h, .layer_count_or_depth = 1, .num_levels = 1};
    SDL_GPUTexture *t = SDL_CreateGPUTexture(d, &ti);
    if (!px)
        return t;
    SDL_GPUTransferBufferCreateInfo bi = {.usage = SDL_GPU_TRANSFERBUFFERUSAGE_UPLOAD, .size = w * h * 4};
    SDL_GPUTransferBuffer *tb = SDL_CreateGPUTransferBuffer(d, &bi);
    SDL_memcpy(SDL_MapGPUTransferBuffer(d, tb, false), px, w * h * 4);
    SDL_UnmapGPUTransferBuffer(d, tb);
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(d);
    SDL_GPUCopyPass *copy = SDL_BeginGPUCopyPass(cmd);
    SDL_GPUTextureTransferInfo src = {.transfer_buffer = tb};
    SDL_GPUTextureRegion dst = {.texture = t, .w = w, .h = h, .d = 1};
    SDL_UploadToGPUTexture(copy, &src, &dst, false);
    SDL_EndGPUCopyPass(copy);
    SDL_SubmitGPUCommandBuffer(cmd);
    SDL_WaitForGPUIdle(d);
    SDL_ReleaseGPUTransferBuffer(d, tb);
    return t;
}

int main(int argc, char **argv)
{
    if (argc < 3)
        return 2;
    const int cfg = SDL_strcmp(argv[1], "bloom_aces") == 0 ? 0 : SDL_strcmp(argv[1], "soft_grade_crt") == 0 ? 1 : 2;
    const int TW = cfg == 0 ? 320 : cfg == 1 ? 400 : 256;
    const int TH = cfg == 0 ? 200 : cfg == 1 ? 300 : 256;
    const SDL_GPUTextureFormat tformat =
        cfg == 2 ? SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB : SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
    SDL_SetHint(SDL_HINT_GPU_DRIVER, "vulkan");
    if (!SDL_Init(SDL_INIT_VIDEO))
        return 77;
    SDL_GPUDevice *device = SDL_CreateGPUDevice(SDL_GPU_SHADERFORMAT_SPIRV, false, "vulkan");
    if (!device)
        return 77;

    Uint8 *color = SDL_calloc(SW * SH, 4);
    for (int y = 0; y < SH; ++y)
        for (int x = 0; x < SW; ++x) {
            Uint8 *c = color + (y * SW + x) * 4;
            c[0] = (Uint8)(x * 255 / 191);
            c[1] = (Uint8)(y * 255 / 127);
            c[2] = ((x / 16 + y / 16) & 1) ? 200 : 40;
            c[3] = 255;
            if (x >= 20 && x < 36 && y >= 20 && y < 36) {
                c[0] = c[1] = c[2] = 255;
            }
            const int dx = x - 150, dy = y - 40;
            if (dx * dx + dy * dy < 100) {
                c[0] = 255;
                c[1] = 200;
                c[2] = 120;
            }
        }
    Uint8 *overlay = SDL_calloc(TW * TH, 4);
    for (int y = 0; y < TH; ++y)
        for (int x = 0; x < TW; ++x) {
            Uint8 *o = overlay + (y * TW + x) * 4;
            if (x >= 10 && x < 110 && y >= 10 && y < 40) {
                o[0] = 102;
                o[1] = 26;
                o[2] = 26;
                o[3] = 128;
            } else if (x >= TW - 60 && x < TW - 20 && y >= TH - 50 && y < TH - 20) {
                o[1] = 255;
                o[3] = 255;
            }
        }

    CHI_Context *fx = CHI_CreateContext(device, CHI_VERSION);
    CHI_Settings s;
    CHI_GetSettings(fx, &s);
    CHI_OutputSettings *o = &s.output;
    if (cfg == 0) {
        s.ambient = (CHI_Color){3.0f, 3.0f, 3.0f};
        o->scale_mode = CHI_SCALE_FIT;
        o->filter = CHI_FILTER_LINEAR;
        o->letterbox = (CHI_Color){0.1f, 0.2f, 0.3f};
        o->exposure_ev = 0.25f;
        o->tonemap = CHI_TONEMAP_ACES;
        s.bloom = (CHI_BloomSettings){1, 1.0f, 0.5f, 0.8f, 24.0f};
    } else if (cfg == 1) {
        s.ambient = (CHI_Color){2.5f, 2.5f, 2.5f};
        o->scale_mode = CHI_SCALE_INTEGER;
        o->filter = CHI_FILTER_NEAREST;
        o->letterbox = (CHI_Color){0.05f, 0.05f, 0.1f};
        o->exposure_ev = -0.2f;
        o->tonemap = CHI_TONEMAP_SOFT;
        o->tonemap_knee = 0.7f;
        o->tint = (CHI_Color){1.1f, 1.0f, 0.9f};
        o->saturation = 1.3f;
        o->contrast = 1.15f;
        o->vignette = 0.5f;
        o->vignette_radius = 0.6f;
        o->vignette_softness = 1.2f;
        o->vignette_color = (CHI_Color){0.2f, 0.0f, 0.0f};
        o->flags = CHI_OUTPUT_DITHER;
        s.bloom = (CHI_BloomSettings){1, 1.5f, 0.5f, 0.5f, 12.0f};
    } else {
        s.ambient = (CHI_Color){1.5f, 1.5f, 1.5f};
        o->scale_mode = CHI_SCALE_STRETCH;
        o->filter = CHI_FILTER_LINEAR;
        o->scene_color_space = CHI_COLORSPACE_LINEAR;
        o->saturation = 0.8f;
        s.bloom = (CHI_BloomSettings){1, 0.5f, 0.5f, 2.0f, 100.0f};
    }
    if (!CHI_SetSettings(fx, &s)) {
        SDL_Log("settings: %s", SDL_GetError());
        return 1;
    }
    if (cfg == 1) {
        CHI_CrtSettings crt;
        CHI_InitCrtSettings(&crt);
        crt.flags = CHI_CRT_OVERLAY;
        crt.curvature = 0.3f;
        crt.corner_radius = 0.15f;
        crt.scanlines = 0.4f;
        crt.scanline_period = 1.0f;
        crt.mask = 0.25f;
        crt.mask_pitch = 3.0f;
        crt.brightness = 1.3f;
        if (!CHI_SetCrt(fx, &crt)) {
            SDL_Log("crt: %s", SDL_GetError());
            return 1;
        }
        CHI_ShockwaveDesc wave;
        CHI_InitShockwaveDesc(&wave);
        wave.x = 96.0f;
        wave.y = 64.0f;
        wave.radius = 30.0f;
        wave.width = 24.0f;
        wave.strength = 5.0f;
        if (!CHI_SetFrameShockwaves(fx, &wave, 1)) {
            SDL_Log("shockwave: %s", SDL_GetError());
            return 1;
        }
    }

    CHI_Frame frame = {0};
    frame.scene = CHI_FromGPU(Upload(device, color, SW, SH, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                                     SDL_GPU_TEXTUREUSAGE_SAMPLER),
                              SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SW, SH);
    const SDL_GPUTextureFormat oformat =
        cfg == 2 ? SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB : SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
    frame.buffers.overlay =
        CHI_FromGPU(Upload(device, overlay, TW, TH, oformat, SDL_GPU_TEXTUREUSAGE_SAMPLER), oformat, TW, TH);
    SDL_GPUTexture *target = Upload(device, NULL, TW, TH, tformat, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET);
    frame.target = CHI_FromGPU(target, tformat, TW, TH);

    SDL_GPUTransferBufferCreateInfo bi = {.usage = SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD, .size = TW * TH * 4};
    SDL_GPUTransferBuffer *readback = SDL_CreateGPUTransferBuffer(device, &bi);
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(device);
    if (!CHI_Render(fx, cmd, &frame)) {
        SDL_Log("render: %s", SDL_GetError());
        return 1;
    }
    SDL_GPUCopyPass *copy = SDL_BeginGPUCopyPass(cmd);
    SDL_GPUTextureRegion src = {.texture = target, .w = TW, .h = TH, .d = 1};
    SDL_GPUTextureTransferInfo dst = {.transfer_buffer = readback};
    SDL_DownloadFromGPUTexture(copy, &src, &dst);
    SDL_EndGPUCopyPass(copy);
    SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
    SDL_WaitForGPUFences(device, true, &fence, 1);
    SDL_ReleaseGPUFence(device, fence);
    Uint8 *px = SDL_MapGPUTransferBuffer(device, readback, false);
    SDL_Surface *out = SDL_CreateSurfaceFrom(TW, TH, SDL_PIXELFORMAT_RGBA32, px, TW * 4);
    if (!SDL_SavePNG(out, argv[2]))
        SDL_Log("save: %s", SDL_GetError());
    SDL_DestroySurface(out);
    SDL_UnmapGPUTransferBuffer(device, readback);
    CHI_FrameStats st;
    CHI_GetFrameStats(fx, &st);
    printf("%s %dx%d: passes %u draws %u bloom_levels %u\n", argv[1], TW, TH, st.passes, st.draw_calls,
           st.bloom_levels);
    CHI_DestroyContext(fx);
    SDL_DestroyGPUDevice(device);
    SDL_Quit();
    return 0;
}
