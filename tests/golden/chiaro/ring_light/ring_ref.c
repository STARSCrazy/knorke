/* Ring light reference (Knorke Chiaro port, P21): renders a scene with shockwave ring lights
 * (CHI_SetFrameShockwaves, E143) with C++ Chiaro through GPU textures and saves the frame as BMP.
 *   ring_ref <light|only> <out.bmp>
 * Both: 128x96 checker scene, ambient 0.1, default settings and output (linear filter).
 * light: identity projection, one point light at (30, 48), radius 80, intensity 1; two waves,
 *        the first lights only, the second lights and distorts (strength 2).
 * only:  no light, an isometric projection, one wave that only lights (no light pass: the ring
 *        pass clears the light buffer itself).
 * Build (MSVC, against an out-of-tree Chiaro 9bc5b9b build <b>):
 *   cl /O2 /std:c11 /I <chiaro>\include /I <b>\_deps\sdl3-src\include ring_ref.c
 *      /link /LIBPATH:<b>\lib\Release Chiaro.lib SDL3.lib
 * Run next to Chiaro.dll and SDL3.dll. The BMP is converted to the RGB PNGs ring_light.png and
 * ring_only.png here. The scene is mirrored in crates/knorke-integration/tests/gpu_ring_light.rs;
 * change both together. */
#include <Chiaro/Chiaro.h>
#include <SDL3/SDL.h>
#include <SDL3/SDL_main.h>
#include <stdio.h>
#include <stdlib.h>

static SDL_GPUTexture *Upload(SDL_GPUDevice *d, const Uint8 *px, int w, int h, SDL_GPUTextureUsageFlags usage)
{
    SDL_GPUTextureCreateInfo ti = {.type = SDL_GPU_TEXTURETYPE_2D, .format = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                                   .usage = usage, .width = w, .height = h, .layer_count_or_depth = 1, .num_levels = 1};
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

static CHI_ShockwaveDesc Wave(float x, float y, float radius, float width, float strength, CHI_Color light,
                              float intensity)
{
    CHI_ShockwaveDesc w;
    CHI_InitShockwaveDesc(&w);
    w.x = x;
    w.y = y;
    w.radius = radius;
    w.width = width;
    w.strength = strength;
    w.light = light;
    w.light_intensity = intensity;
    return w;
}

int main(int argc, char **argv)
{
    if (argc < 3)
        return 2;
    const bool only = SDL_strcmp(argv[1], "only") == 0;
    const int W = 128, H = 96;
    SDL_SetHint(SDL_HINT_GPU_DRIVER, "vulkan");
    if (!SDL_Init(SDL_INIT_VIDEO))
        return 77;
    SDL_GPUDevice *device = SDL_CreateGPUDevice(SDL_GPU_SHADERFORMAT_SPIRV, false, "vulkan");
    if (!device)
        return 77;
    Uint8 *color = SDL_calloc(W * H, 4);
    for (int y = 0; y < H; ++y)
        for (int x = 0; x < W; ++x) {
            Uint8 *c = color + (y * W + x) * 4;
            const int checker = ((x / 8) + (y / 8)) & 1 ? 12 : -12;
            c[0] = (Uint8)(110 + checker);
            c[1] = (Uint8)(120 + checker);
            c[2] = (Uint8)(130 + checker);
            c[3] = 255;
        }
    CHI_Context *fx = CHI_CreateContext(device, CHI_VERSION);
    CHI_SetAmbient(fx, (CHI_Color){0.1f, 0.1f, 0.1f});
    CHI_Frame frame = {0};
    const CHI_Projection iso = {{64.0f, 20.0f}, {2.0f, 1.0f}, {-2.0f, 1.0f}, {0.0f, -2.0f}};
    CHI_ShockwaveDesc waves[2];
    Uint32 count;
    if (!only) {
        CHI_LightDesc l;
        if (!CHI_CreateLight(fx, CHI_PointLight(&l, 30, 48, 80, CHI_WHITE, 1.0f)).id) {
            SDL_Log("%s", SDL_GetError());
            return 1;
        }
        waves[0] = Wave(64.0f, 48.0f, 20.0f, 10.0f, 0.0f, (CHI_Color){1.0f, 0.5f, 0.2f}, 2.0f);
        waves[1] = Wave(100.0f, 70.0f, 12.0f, 6.0f, 2.0f, (CHI_Color){0.3f, 0.6f, 1.0f}, 1.5f);
        count = 2;
    } else {
        frame.projection = &iso;
        waves[0] = Wave(20.0f, 18.0f, 9.0f, 5.0f, 0.0f, (CHI_Color){1.0f, 0.7f, 0.3f}, 3.0f);
        count = 1;
    }
    if (!CHI_SetFrameShockwaves(fx, waves, count)) {
        SDL_Log("%s", SDL_GetError());
        return 1;
    }
    frame.scene = CHI_FromGPU(Upload(device, color, W, H, SDL_GPU_TEXTUREUSAGE_SAMPLER),
                              SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, W, H);
    SDL_GPUTexture *target = Upload(device, NULL, W, H, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET);
    frame.target = CHI_FromGPU(target, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, W, H);

    SDL_GPUTransferBufferCreateInfo bi = {.usage = SDL_GPU_TRANSFERBUFFERUSAGE_DOWNLOAD, .size = W * H * 4};
    SDL_GPUTransferBuffer *readback = SDL_CreateGPUTransferBuffer(device, &bi);
    SDL_GPUCommandBuffer *cmd = SDL_AcquireGPUCommandBuffer(device);
    if (!CHI_Render(fx, cmd, &frame)) {
        SDL_Log("render: %s", SDL_GetError());
        return 1;
    }
    SDL_GPUCopyPass *copy = SDL_BeginGPUCopyPass(cmd);
    SDL_GPUTextureRegion src = {.texture = target, .w = W, .h = H, .d = 1};
    SDL_GPUTextureTransferInfo dst = {.transfer_buffer = readback};
    SDL_DownloadFromGPUTexture(copy, &src, &dst);
    SDL_EndGPUCopyPass(copy);
    SDL_GPUFence *fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
    SDL_WaitForGPUFences(device, true, &fence, 1);
    SDL_ReleaseGPUFence(device, fence);
    Uint8 *px = SDL_MapGPUTransferBuffer(device, readback, false);
    SDL_Surface *out = SDL_CreateSurfaceFrom(W, H, SDL_PIXELFORMAT_RGBA32, px, W * 4);
    SDL_SaveBMP(out, argv[2]);
    SDL_DestroySurface(out);
    SDL_UnmapGPUTransferBuffer(device, readback);
    return 0;
}
