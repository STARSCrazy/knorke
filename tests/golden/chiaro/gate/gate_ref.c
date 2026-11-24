/* Gate spike reference (Knorke Chiaro port): renders the two spike scenes with C++ Chiaro through
 * GPU textures (CHI_FromGPU), saves the frame as BMP and optionally times N frames.
 *   gate_ref <normal|shadow> <out.bmp> [frames]
 * normal: examples/minimal_normal_map (64x64): colour + material from CHI_ConvertNormalMap.
 * shadow: 192x128 scene, one soft point light, sprite occlusion buffer (three shapes).
 * Build (MSVC, against an out-of-tree Chiaro 9bc5b9b build <b>):
 *   cl /O2 /std:c11 /I <chiaro>\include /I <b>\_deps\sdl3-src\include gate_ref.c
 *      /link /LIBPATH:<b>\lib\Release Chiaro.lib SDL3.lib
 * The BMP is converted to the RGB PNG in tests/golden/chiaro. The scene is mirrored in
 * crates/knorke-light2d/tests/gpu_spike.rs; change both together. */
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

int main(int argc, char **argv)
{
    if (argc < 3)
        return 2;
    const bool shadow = SDL_strcmp(argv[1], "shadow") == 0;
    const int frames = argc > 3 ? atoi(argv[3]) : 0;
    const int W = shadow ? 192 : 64, H = shadow ? 128 : 64;
    SDL_SetHint(SDL_HINT_GPU_DRIVER, "vulkan");
    if (!SDL_Init(SDL_INIT_VIDEO))
        return 77;
    SDL_GPUDevice *device = SDL_CreateGPUDevice(SDL_GPU_SHADERFORMAT_SPIRV, false, "vulkan");
    if (!device)
        return 77;
    Uint8 *color = SDL_calloc(W * H, 4), *second = SDL_calloc(W * H, 4);
    CHI_Context *fx = CHI_CreateContext(device, CHI_VERSION);
    CHI_Frame frame = {0};
    if (!shadow) {
        Uint8 *normal = SDL_calloc(W * H, 4);
        for (int i = 0; i < W * H; ++i) {
            color[i * 4 + 0] = color[i * 4 + 1] = color[i * 4 + 2] = 160;
            color[i * 4 + 3] = 255;
        }
        for (int y = 0; y < H; ++y)
            for (int x = 0; x < W; ++x) {
                float dx = (x + 0.5f - W / 2) / 24.0f, dy = (y + 0.5f - H / 2) / 24.0f, d2 = dx * dx + dy * dy;
                float n[3] = {0.0f, 0.0f, 1.0f};
                if (d2 < 1.0f) {
                    n[0] = dx;
                    n[1] = -dy;
                    n[2] = SDL_sqrtf(1.0f - d2);
                }
                Uint8 *p = normal + (y * W + x) * 4;
                for (int i = 0; i < 3; ++i)
                    p[i] = (Uint8)SDL_lroundf((n[i] * 0.5f + 0.5f) * 255.0f);
                p[3] = 255;
            }
        CHI_NormalMapOptions o;
        CHI_InitNormalMapOptions(&o);
        o.material = CHI_GetMaterialPreset(CHI_MATERIAL_SATIN);
        o.alpha_rgba = color;
        o.alpha_pitch = W * 4;
        if (!CHI_ConvertNormalMap(normal, W, H, W * 4, &o, second)) {
            SDL_Log("%s", SDL_GetError());
            return 1;
        }
        CHI_LightDesc l;
        CHI_CreateLight(fx, CHI_PointLight(&l, 4, 32, 96, CHI_WHITE, 1.5f));
        CHI_SetAmbient(fx, (CHI_Color){0.1f, 0.1f, 0.1f});
        CHI_SetScaling(fx, CHI_SCALE_INTEGER, CHI_FILTER_NEAREST);
        frame.buffers.material = CHI_FromGPU(Upload(device, second, W, H, SDL_GPU_TEXTUREUSAGE_SAMPLER),
                                             SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, W, H);
    } else {
        for (int y = 0; y < H; ++y)
            for (int x = 0; x < W; ++x) {
                Uint8 *c = color + (y * W + x) * 4, *o = second + (y * W + x) * 4;
                const int checker = ((x / 8) + (y / 8)) & 1 ? 12 : -12;
                c[0] = (Uint8)(110 + checker);
                c[1] = (Uint8)(120 + checker);
                c[2] = (Uint8)(130 + checker);
                c[3] = 255;
                const int cx = x - 100, cy = y - 100;
                const bool solid = (x >= 90 && x < 110 && y >= 30 && y < 60) ||
                                   (x >= 130 && x < 170 && y >= 85 && y < 95) || cx * cx + cy * cy < 100;
                o[0] = o[1] = o[2] = 0;
                o[3] = solid ? 255 : 0;
            }
        CHI_LightDesc l;
        CHI_PointLight(&l, 40, 64, 170, (CHI_Color){1.0f, 0.85f, 0.6f}, 1.3f);
        l.source_size = 4.0f;
        if (!CHI_CreateLight(fx, &l).id) {
            SDL_Log("%s", SDL_GetError());
            return 1;
        }
        CHI_SetAmbient(fx, (CHI_Color){0.08f, 0.08f, 0.08f});
        frame.buffers.occlusion = CHI_FromGPU(Upload(device, second, W, H, SDL_GPU_TEXTUREUSAGE_SAMPLER),
                                              SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, W, H);
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

    if (frames > 0) {
        double *t = SDL_calloc(frames, sizeof(double));
        const double freq = (double)SDL_GetPerformanceFrequency();
        for (int i = 0; i < frames; ++i) {
            const Uint64 a = SDL_GetPerformanceCounter();
            cmd = SDL_AcquireGPUCommandBuffer(device);
            CHI_Render(fx, cmd, &frame);
            fence = SDL_SubmitGPUCommandBufferAndAcquireFence(cmd);
            SDL_WaitForGPUFences(device, true, &fence, 1);
            SDL_ReleaseGPUFence(device, fence);
            t[i] = (double)(SDL_GetPerformanceCounter() - a) / freq * 1e6;
        }
        for (int i = 1; i < frames; ++i) /* insertion sort, small N */
            for (int j = i; j > 0 && t[j] < t[j - 1]; --j) {
                double s = t[j];
                t[j] = t[j - 1];
                t[j - 1] = s;
            }
        double sum = 0;
        for (int i = 0; i < frames; ++i)
            sum += t[i];
        CHI_FrameStats st;
        CHI_GetFrameStats(fx, &st);
        printf("bench %s %dx%d vulkan (C++), %d frames: mean %.1f us, median %.1f us, p95 %.1f us; passes %u draws %u cpu_record %.3f ms\n",
               argv[1], W, H, frames, sum / frames, t[frames / 2], t[frames * 95 / 100], st.passes, st.draw_calls,
               st.cpu_record_ms);
    }
    CHI_DestroyContext(fx);
    SDL_DestroyGPUDevice(device);
    SDL_Quit();
    return 0;
}
