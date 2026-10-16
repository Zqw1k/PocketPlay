/* ABI glue for upstream native codec and graphics libraries. Scheduling stays in Rust. */
#define WIN32_LEAN_AND_MEAN
#define COBJMACROS
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <libavcodec/avcodec.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_d3d11va.h>
#include <d3d11_4.h>
#include <dxgi1_2.h>
#include <libavutil/error.h>
#include <SDL3/SDL.h>
#include "symbols.h"

static _Thread_local char error_text[512];
const char *pp_error(void) { return error_text; }
static int fail(const char *message) { snprintf(error_text, sizeof(error_text), "%s", message); return -1; }
static int av_fail(int code, const char *operation) {
    char detail[256]; av_strerror(code, detail, sizeof(detail));
    snprintf(error_text, sizeof(error_text), "%s: %s", operation, detail); return -1;
}
static HMODULE load(const wchar_t *dir, const wchar_t *name) {
    wchar_t path[32768];
    if (wcslen(dir) + wcslen(name) + 2 >= 32768) return NULL;
    wcscpy(path, dir); wcscat(path, L"\\"); wcscat(path, name);
    return LoadLibraryExW(path, NULL, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
}
int pp_init(const wchar_t *dir) {
    HMODULE util = load(dir, L"avutil-61.dll");
    HMODULE resample = load(dir, L"swresample-7.dll");
    HMODULE codec = load(dir, L"avcodec-63.dll");
    HMODULE sdl = load(dir, L"SDL3.dll");
    if (!util || !resample || !codec || !sdl) return fail("Cannot load native media DLLs");
    if (!load_symbols(codec, util, sdl)) return fail("Native media DLL API mismatch");
    if (avcodec_version() >> 16 != 63) return fail("Unsupported FFmpeg codec ABI");
    return 0;
}

typedef struct Decoder {
    AVCodecContext *context;
    AVFrame *frame;
    AVFrame *download;
    AVPacket *packet;
    AVBufferRef *hardware;
    int actual_hardware;
} Decoder;

static enum AVPixelFormat choose_format(AVCodecContext *context, const enum AVPixelFormat *formats) {
    Decoder *d = context->opaque;
    if (d->hardware) {
        for (const enum AVPixelFormat *p = formats; *p != AV_PIX_FMT_NONE; p++)
            if (*p == AV_PIX_FMT_D3D11) return *p;
    }
    for (const enum AVPixelFormat *p = formats; *p != AV_PIX_FMT_NONE; p++)
        if (*p == AV_PIX_FMT_YUV420P || *p == AV_PIX_FMT_NV12) return *p;
    return formats[0];
}
void pp_decoder_free(void *handle) {
    Decoder *d = handle;
    if (!d) return;
    avcodec_free_context(&d->context); av_frame_free(&d->frame);
    av_frame_free(&d->download); av_packet_free(&d->packet);
    av_buffer_unref(&d->hardware); free(d);
}
void *pp_decoder_new(const char *name, int hardware, void *render_device) {
    if (!render_device && hardware == 2) { fail("D3D11 decoding requires a D3D11 renderer"); return NULL; }
    if (!render_device) hardware=0;
    const AVCodec *codec = avcodec_find_decoder_by_name(name);
    if (!codec) { fail("Video decoder unavailable"); return NULL; }
    Decoder *d = calloc(1, sizeof(*d));
    if (!d) { fail("Decoder allocation failed"); return NULL; }
    d->context = avcodec_alloc_context3(codec); d->frame = av_frame_alloc();
    d->download = av_frame_alloc(); d->packet = av_packet_alloc();
    if (!d->context || !d->frame || !d->download || !d->packet) { pp_decoder_free(d); fail("Decoder allocation failed"); return NULL; }
    d->context->flags |= AV_CODEC_FLAG_LOW_DELAY;
    d->context->thread_count = 1;
    d->context->thread_type = 0;
    d->context->opaque = d;
    d->context->get_format = choose_format;
    if (hardware) {
        int code;
        if (render_device) {
            d->hardware = av_hwdevice_ctx_alloc(AV_HWDEVICE_TYPE_D3D11VA);
            if (!d->hardware) { pp_decoder_free(d); fail("Hardware context allocation failed"); return NULL; }
            AVHWDeviceContext *context=(AVHWDeviceContext*)d->hardware->data;
            AVD3D11VADeviceContext *device=context->hwctx;
            device->device=render_device;ID3D11Device_AddRef(device->device);
            code=av_hwdevice_ctx_init(d->hardware);
            if (code < 0) av_buffer_unref(&d->hardware);
        } else code = av_hwdevice_ctx_create(&d->hardware, AV_HWDEVICE_TYPE_D3D11VA, NULL, NULL, 0);
        if (code < 0 && hardware == 2) { av_fail(code, "Required D3D11 decoder"); pp_decoder_free(d); return NULL; }
        if (d->hardware) d->context->hw_device_ctx = av_buffer_ref(d->hardware);
    }
    int code = avcodec_open2(d->context, codec, NULL);
    if (code < 0) { av_fail(code, "Open decoder"); pp_decoder_free(d); return NULL; }
    return d;
}
void pp_frame_free(void *handle) { AVFrame *frame = handle; av_frame_free(&frame); }
int pp_frame_width(void *handle) { return ((AVFrame*)handle)->width; }
int pp_frame_height(void *handle) { return ((AVFrame*)handle)->height; }
#ifdef POCKETPLAY_MEDIA_QA
int64_t pp_frame_pts(void *handle) { return ((AVFrame*)handle)->pts; }
int64_t pp_clock_us(void) {
    LARGE_INTEGER ticks,frequency;
    if (!QueryPerformanceCounter(&ticks) || !QueryPerformanceFrequency(&frequency) || frequency.QuadPart<=0) return -1;
    return ticks.QuadPart/frequency.QuadPart*1000000 + ticks.QuadPart%frequency.QuadPart*1000000/frequency.QuadPart;
}
#endif
int pp_decoder_hardware(void *handle) { return ((Decoder*)handle)->actual_hardware; }

static int receive_frames(Decoder *d, AVFrame **latest) {
    for (;;) {
        int code = avcodec_receive_frame(d->context, d->frame);
        if (code == AVERROR(EAGAIN) || code == AVERROR_EOF) return 0;
        if (code < 0) return av_fail(code, "Decode frame");
        AVFrame *source = d->frame;
        if (source->format == AV_PIX_FMT_D3D11) {
            d->actual_hardware = 1;
        }
        if (source->width <= 0 || source->height <= 0 || source->width > 8192 || source->height > 8192 ||
            (int64_t)source->width * source->height > 16 * 1024 * 1024) { av_frame_unref(d->frame); return fail("Decoded dimensions exceed limits"); }
        AVFrame *copy = av_frame_clone(source);
        av_frame_unref(d->frame);
        if (!copy) return fail("Frame reference allocation failed");
        if (*latest) av_frame_free(latest);
        *latest = copy;
    }
}
int pp_decoder_push(void *handle, const uint8_t *bytes, int length, int64_t pts, void **output) {
    Decoder *d = handle; AVFrame *latest = NULL;
    *output = NULL;
    if (length <= 0 || length > 16 * 1024 * 1024) return fail("Invalid encoded packet size");
    av_packet_unref(d->packet);
    int code = av_new_packet(d->packet, length);
    if (code < 0) return av_fail(code, "Allocate packet");
    memcpy(d->packet->data, bytes, length); d->packet->pts = pts; d->packet->dts = pts;
    code = avcodec_send_packet(d->context, d->packet);
    if (code == AVERROR(EAGAIN)) {
        if (receive_frames(d, &latest) < 0) { av_frame_free(&latest); return -1; }
        code = avcodec_send_packet(d->context, d->packet);
    }
    av_packet_unref(d->packet);
    if (code < 0) { av_frame_free(&latest); return av_fail(code, "Submit packet"); }
    if (receive_frames(d, &latest) < 0) { av_frame_free(&latest); return -1; }
    *output = latest; return latest ? 1 : 0;
}

typedef struct Viewer {
    SDL_Window *window; SDL_Renderer *renderer; SDL_Texture *texture;
    ID3D11Device *device; ID3D11DeviceContext *context; ID3D11Texture2D *gpu_texture;
    int w, h, format, display_w, display_h, placement_pending, vsync; SDL_Colorspace colorspace; int captured;
#ifdef POCKETPLAY_MEDIA_QA
    struct { UINT id; int64_t pts, submitted; } presents[256];
    UINT feedback_id;
#endif
} Viewer;
#ifdef POCKETPLAY_MEDIA_QA
int pp_viewer_feedback(void *handle, int64_t *sample) {
    Viewer *v=handle;
    IDXGISwapChain1 *chain=SDL_GetPointerProperty(SDL_GetRendererProperties(v->renderer),SDL_PROP_RENDERER_D3D11_SWAPCHAIN_POINTER,NULL);
    if (!chain) return 0;
    DXGI_FRAME_STATISTICS stats;
    if (FAILED(IDXGISwapChain1_GetFrameStatistics(chain,&stats)) || !stats.PresentCount || stats.PresentCount==v->feedback_id) return 0;
    v->feedback_id=stats.PresentCount;
    UINT slot=stats.PresentCount%256;
    if (v->presents[slot].id!=stats.PresentCount) return 0;
    LARGE_INTEGER frequency;
    if (!QueryPerformanceFrequency(&frequency) || frequency.QuadPart<=0) return 0;
    sample[0]=v->presents[slot].pts;
    sample[1]=stats.SyncQPCTime.QuadPart/frequency.QuadPart*1000000 + stats.SyncQPCTime.QuadPart%frequency.QuadPart*1000000/frequency.QuadPart;
    sample[2]=v->presents[slot].submitted;
    sample[3]=(int64_t)stats.PresentRefreshCount-(int64_t)stats.SyncRefreshCount;
    sample[4]=stats.PresentCount;
    sample[5]=stats.PresentRefreshCount;
    return 1;
}
#endif
void pp_viewer_free(void *handle) {
    Viewer *v = handle; if (!v) return;
    if (v->texture) SDL_DestroyTexture(v->texture);
    if (v->gpu_texture) ID3D11Texture2D_Release(v->gpu_texture);
    if (v->context) ID3D11DeviceContext_Release(v->context);
    if (v->renderer) SDL_DestroyRenderer(v->renderer);
    if (v->window) SDL_DestroyWindow(v->window);
    free(v); SDL_Quit();
}
void *pp_viewer_new(int w, int h, int fullscreen, int smooth) {
    SDL_SetHint("SDL_WINDOWS_DPI_AWARENESS", "permonitorv2");
    SDL_SetHint(SDL_HINT_RENDER_DIRECT3D_THREADSAFE, "1");
    if (!SDL_Init(SDL_INIT_VIDEO)) { fail(SDL_GetError()); return NULL; }
    Viewer *v = calloc(1, sizeof(*v));
    if (!v) { fail("Viewer allocation failed"); SDL_Quit(); return NULL; }
    SDL_Rect bounds = {0, 0, 1280, 720};
    SDL_GetDisplayUsableBounds(SDL_GetPrimaryDisplay(), &bounds);
    double scale = fmin(1.0, fmin((bounds.w - 80.0) / w, (bounds.h - 100.0) / h));
    int ww = (int)(w * scale), wh = (int)(h * scale);
    SDL_WindowFlags flags=SDL_WINDOW_RESIZABLE | SDL_WINDOW_HIGH_PIXEL_DENSITY;
#ifdef POCKETPLAY_MEDIA_QA
    const char *qa_fullscreen=getenv("POCKETPLAY_QA_FULLSCREEN");
    if (qa_fullscreen) fullscreen=strcmp(qa_fullscreen,"1")==0;
#endif
    if (fullscreen) flags|=SDL_WINDOW_FULLSCREEN;
    v->window = SDL_CreateWindow("PocketPlay Android", ww, wh, flags);
    if (!v->window) { fail(SDL_GetError()); pp_viewer_free(v); return NULL; }
    v->display_w=w;v->display_h=h;
    float aspect=(float)w/h;
    if (!SDL_SetWindowAspectRatio(v->window,aspect,aspect)) { fail(SDL_GetError());pp_viewer_free(v);return NULL; }
    v->renderer = SDL_CreateRenderer(v->window, "direct3d11");
    if (!v->renderer) v->renderer = SDL_CreateRenderer(v->window, NULL);
    if (!v->renderer) { fail(SDL_GetError()); pp_viewer_free(v); return NULL; }
    int vsync=-1;
    int wanted_vsync=smooth ? 1 : 0;
#ifdef POCKETPLAY_MEDIA_QA
    const char *qa_vsync=getenv("POCKETPLAY_QA_VSYNC");
    if (qa_vsync) wanted_vsync=atoi(qa_vsync);
#endif
    if (!SDL_SetRenderVSync(v->renderer, wanted_vsync) || !SDL_GetRenderVSync(v->renderer,&vsync) || vsync != wanted_vsync) {
        fail("The renderer could not apply the selected VSync mode");pp_viewer_free(v);return NULL;
    }
    v->vsync=vsync;
    v->device=SDL_GetPointerProperty(SDL_GetRendererProperties(v->renderer), SDL_PROP_RENDERER_D3D11_DEVICE_POINTER, NULL);
    if (v->device) {
        ID3D11Device_GetImmediateContext(v->device,&v->context);
        const GUID multithread_id={0x9b7e4e00,0x342c,0x4106,{0xa1,0x9f,0x4f,0x27,0x04,0xf6,0x89,0xf0}};
        ID3D11Multithread *multithread=NULL;
        if (SUCCEEDED(ID3D11DeviceContext_QueryInterface(v->context,&multithread_id,(void**)&multithread))) {
            ID3D11Multithread_SetMultithreadProtected(multithread,TRUE);ID3D11Multithread_Release(multithread);
        } else v->device=NULL;
    }
    return v;
}
void *pp_viewer_device(void *handle) { return ((Viewer*)handle)->device; }
void pp_viewer_icon(void *handle, const uint8_t *pixels, int w, int h) {
    if (w<=0 || h<=0 || w>512 || h>512) return;
    SDL_Surface *surface=SDL_CreateSurfaceFrom(w,h,SDL_PIXELFORMAT_RGBA32,(void*)pixels,w*4);
    if (surface) {SDL_SetWindowIcon(((Viewer*)handle)->window,surface);SDL_DestroySurface(surface);}
}
const char *pp_viewer_renderer(void *handle) { return SDL_GetRendererName(((Viewer*)handle)->renderer); }
int pp_viewer_vsync(void *handle) { return ((Viewer*)handle)->vsync; }
int pp_viewer_geometry(void *handle, int w, int h) {
    Viewer *v=handle;
    if (w<=0 || h<=0 || w>8192 || h>8192) return fail("Invalid content aspect ratio");
    int changed=v->display_w!=w || v->display_h!=h;
    if (!changed && !v->placement_pending) return 0;
    // Capture the old center and monitor before the aspect constraint can resize
    // the window. Keeping the old top-left would push a wide rotation off-screen.
    int old_x,old_y,old_w,old_h;
    SDL_Rect bounds;
    if (!SDL_GetWindowPosition(v->window,&old_x,&old_y) ||
        !SDL_GetWindowSize(v->window,&old_w,&old_h) ||
        !SDL_GetDisplayUsableBounds(SDL_GetDisplayForWindow(v->window),&bounds)) return fail(SDL_GetError());
    if (changed) {
        v->display_w=w;v->display_h=h;v->placement_pending=1;
    }
    // Apply a deferred fit after restore, including when no video frame arrives.
    if (SDL_GetWindowFlags(v->window)&(SDL_WINDOW_MAXIMIZED|SDL_WINDOW_MINIMIZED|SDL_WINDOW_FULLSCREEN)) return 0;
    // Setting the aspect constraint itself may resize the native window. Keep
    // that operation out of minimize/maximize transitions too.
    float aspect=(float)w/h;
    if (!SDL_SetWindowAspectRatio(v->window,aspect,aspect)) return fail(SDL_GetError());
    int top,left,bottom,right;
    if (!SDL_GetWindowBordersSize(v->window,&top,&left,&bottom,&right)) return fail(SDL_GetError());
    int available_w=bounds.w-left-right-24,available_h=bounds.h-top-bottom-24;
    if (available_w<1 || available_h<1) return fail("Monitor work area is too small");
    double fit=fmin(1.0,fmin((double)available_w/w,(double)available_h/h));
    int ww=(int)fmax(1.0,w*fit),wh=(int)fmax(1.0,h*fit);
    int x=(int)round(old_x+(old_w-ww)*0.5),y=(int)round(old_y+(old_h-wh)*0.5);
    int min_x=bounds.x+left+12,min_y=bounds.y+top+12;
    int max_x=bounds.x+bounds.w-right-ww-12,max_y=bounds.y+bounds.h-bottom-wh-12;
    x=x<min_x?min_x:(x>max_x?max_x:x);
    y=y<min_y?min_y:(y>max_y?max_y:y);
    if (!SDL_SetWindowSize(v->window,ww,wh) || !SDL_SetWindowPosition(v->window,x,y)) return fail(SDL_GetError());
    v->placement_pending=0;
    return 0;
}
int pp_viewer_output_size(void *handle,int *w,int *h) {
    if (SDL_GetWindowFlags(((Viewer*)handle)->window)&SDL_WINDOW_MINIMIZED) return 1;
    return SDL_GetRenderOutputSize(((Viewer*)handle)->renderer,w,h) ? 0 : fail(SDL_GetError());
}
int pp_viewer_poll(void *handle) {
    Viewer *v=handle;
    SDL_Event e;
    while (SDL_PollEvent(&e)) {
        if (e.type == SDL_EVENT_QUIT || e.type == SDL_EVENT_WINDOW_CLOSE_REQUESTED) return 0;
        if (e.type==SDL_EVENT_KEY_DOWN && e.key.key==SDLK_F11 && !e.key.repeat) {
            bool fullscreen=!!(SDL_GetWindowFlags(v->window)&SDL_WINDOW_FULLSCREEN);
            if (!SDL_SetWindowFullscreen(v->window,!fullscreen)) return fail(SDL_GetError());
            v->placement_pending=1;
        }
        if (e.type==SDL_EVENT_WINDOW_RESIZED &&
            !(SDL_GetWindowFlags(v->window)&(SDL_WINDOW_MAXIMIZED|SDL_WINDOW_MINIMIZED|SDL_WINDOW_FULLSCREEN))) {
            int w,h;
            if (!SDL_GetWindowSize(v->window,&w,&h)) return fail(SDL_GetError());
            // A native restore animation can apply its old rectangle after our
            // first fit. Repair that late resize, while allowing pixel rounding
            // and ordinary resizing that already satisfies the aspect ratio.
            double mismatch=fabs((double)w*v->display_h-(double)h*v->display_w);
            if (mismatch>fmax(v->display_w,v->display_h)*1.25) v->placement_pending=1;
        }
    }
    if (v->placement_pending && pp_viewer_geometry(v,v->display_w,v->display_h)!=0) return -1;
    return 1;
}
int pp_viewer_present(void *handle, void *frame_handle, const double *rectangles) {
    Viewer *v = handle; AVFrame *f = frame_handle;
    if (SDL_GetWindowFlags(v->window)&SDL_WINDOW_MINIMIZED) return 1;
    int gpu=f->format==AV_PIX_FMT_D3D11;
    int format = f->format == AV_PIX_FMT_NV12 || gpu ? SDL_PIXELFORMAT_NV12 : SDL_PIXELFORMAT_IYUV;
    bool full=f->color_range==AVCOL_RANGE_JPEG;
    SDL_Colorspace colorspace=(f->colorspace==AVCOL_SPC_BT709 || f->height>576)
        ? (full ? SDL_COLORSPACE_BT709_FULL : SDL_COLORSPACE_BT709_LIMITED)
        : (full ? SDL_COLORSPACE_BT601_FULL : SDL_COLORSPACE_BT601_LIMITED);
    if (!gpu && f->format != AV_PIX_FMT_NV12 && f->format != AV_PIX_FMT_YUV420P && f->format != AV_PIX_FMT_YUVJ420P)
        return fail("Unsupported decoded pixel format; use an 8-bit video stream");
    if (!v->texture || v->w != f->width || v->h != f->height || v->format != format || v->colorspace!=colorspace) {
        if (v->texture) SDL_DestroyTexture(v->texture);
        v->texture=NULL;
        if (v->gpu_texture) { ID3D11Texture2D_Release(v->gpu_texture);v->gpu_texture=NULL; }
        if (gpu) {
            if (!v->device) return fail("GPU renderer and decoder do not share a device");
            D3D11_TEXTURE2D_DESC desc={0};desc.Width=f->width;desc.Height=f->height;desc.MipLevels=1;desc.ArraySize=1;
            desc.Format=DXGI_FORMAT_NV12;desc.SampleDesc.Count=1;desc.Usage=D3D11_USAGE_DEFAULT;desc.BindFlags=D3D11_BIND_SHADER_RESOURCE;
            if (FAILED(ID3D11Device_CreateTexture2D(v->device,&desc,NULL,&v->gpu_texture))) return fail("Cannot create NV12 GPU presentation texture");
            SDL_PropertiesID props=SDL_CreateProperties();
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_FORMAT_NUMBER,format);
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_ACCESS_NUMBER,SDL_TEXTUREACCESS_STATIC);
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_WIDTH_NUMBER,f->width);
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_HEIGHT_NUMBER,f->height);
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_COLORSPACE_NUMBER,colorspace);
            SDL_SetPointerProperty(props,SDL_PROP_TEXTURE_CREATE_D3D11_TEXTURE_POINTER,v->gpu_texture);
            v->texture=SDL_CreateTextureWithProperties(v->renderer,props);SDL_DestroyProperties(props);
        } else {
            SDL_PropertiesID props=SDL_CreateProperties();
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_FORMAT_NUMBER,format);
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_ACCESS_NUMBER,SDL_TEXTUREACCESS_STREAMING);
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_WIDTH_NUMBER,f->width);
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_HEIGHT_NUMBER,f->height);
            SDL_SetNumberProperty(props,SDL_PROP_TEXTURE_CREATE_COLORSPACE_NUMBER,colorspace);
            v->texture=SDL_CreateTextureWithProperties(v->renderer,props);SDL_DestroyProperties(props);
        }
        if (!v->texture) return fail(SDL_GetError());
        SDL_SetTextureScaleMode(v->texture, SDL_SCALEMODE_LINEAR);
        v->w = f->width; v->h = f->height; v->format = format;v->colorspace=colorspace;
    }
    bool uploaded=true;
    if (gpu) {
        D3D11_TEXTURE2D_DESC source;ID3D11Texture2D_GetDesc((ID3D11Texture2D*)f->data[0],&source);
        if (source.Format!=DXGI_FORMAT_NV12 || source.Width<(UINT)f->width || source.Height<(UINT)f->height || (uintptr_t)f->data[1]>=source.ArraySize)
            return fail("Unsupported hardware surface; use an 8-bit SDR stream");
        D3D11_BOX box={0,0,0,(UINT)f->width,(UINT)f->height,1};
        ID3D11DeviceContext_CopySubresourceRegion(v->context,(ID3D11Resource*)v->gpu_texture,0,0,0,0,(ID3D11Resource*)f->data[0],(UINT)(uintptr_t)f->data[1],&box);
    } else uploaded = format == SDL_PIXELFORMAT_NV12
        ? SDL_UpdateNVTexture(v->texture, NULL, f->data[0], f->linesize[0], f->data[1], f->linesize[1])
        : SDL_UpdateYUVTexture(v->texture, NULL, f->data[0], f->linesize[0], f->data[1], f->linesize[1], f->data[2], f->linesize[2]);
    if (!uploaded) return fail(SDL_GetError());
    SDL_FRect source_rect={(float)rectangles[0],(float)rectangles[1],(float)rectangles[2],(float)rectangles[3]};
    SDL_FRect destination={(float)rectangles[4],(float)rectangles[5],(float)rectangles[6],(float)rectangles[7]};
    if (!SDL_RenderClear(v->renderer) || !SDL_RenderTexture(v->renderer, v->texture, &source_rect, &destination)) return fail(SDL_GetError());
#ifdef POCKETPLAY_MEDIA_QA
    const char *capture=getenv("POCKETPLAY_VIEWER_SNAPSHOT");
    if (capture && !v->captured) {
        SDL_Surface *surface=SDL_RenderReadPixels(v->renderer,NULL);
        if (surface) {SDL_SaveBMP(surface,capture);SDL_DestroySurface(surface);v->captured=1;}
    }
#endif
    if (!SDL_RenderPresent(v->renderer)) return fail(SDL_GetError());
#ifdef POCKETPLAY_MEDIA_QA
    IDXGISwapChain1 *chain=SDL_GetPointerProperty(SDL_GetRendererProperties(v->renderer),SDL_PROP_RENDERER_D3D11_SWAPCHAIN_POINTER,NULL);
    UINT count=0;
    if (chain && SUCCEEDED(IDXGISwapChain1_GetLastPresentCount(chain,&count))) {
        UINT slot=count%256;
        v->presents[slot].id=count;v->presents[slot].pts=f->pts;v->presents[slot].submitted=pp_clock_us();
    }
#endif
    return 0;
}

typedef struct Audio {
    SDL_AudioStream *stream;
    int target_bytes;
    int resumed;
    double device_buffer_ms;
} Audio;
void *pp_audio_new(int buffer_ms) {
    SDL_SetHint(SDL_HINT_AUDIO_DEVICE_SAMPLE_FRAMES, "128");
    if (!SDL_InitSubSystem(SDL_INIT_AUDIO)) { fail(SDL_GetError()); return NULL; }
    Audio *audio = calloc(1, sizeof(*audio));
    if (!audio) { fail("Audio allocation failed"); return NULL; }
    SDL_AudioSpec spec = {SDL_AUDIO_S16LE, 2, 48000};
    audio->stream = SDL_OpenAudioDeviceStream(SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, &spec, NULL, NULL);
    if (!audio->stream) { fail(SDL_GetError()); free(audio); return NULL; }
    audio->target_bytes = (buffer_ms < 0 ? 0 : buffer_ms > 200 ? 200 : buffer_ms) * 192;
    SDL_AudioSpec actual;
    int frames = 0;
    if (SDL_GetAudioDeviceFormat(SDL_GetAudioStreamDevice(audio->stream), &actual, &frames) && actual.freq > 0)
        audio->device_buffer_ms = frames * 1000.0 / actual.freq;
    return audio;
}
void pp_audio_free(void *handle) {
    Audio *audio=handle;if (!audio) return;
    SDL_DestroyAudioStream(audio->stream);free(audio);
}
int pp_audio_push(void *handle, const uint8_t *bytes, int length) {
    Audio *audio=handle;
    if (length <= 0 || length > 65536 || length % 4) return fail("Invalid PCM packet");
    int queued=SDL_GetAudioStreamQueued(audio->stream);
    if (queued < 0) return fail(SDL_GetError());
    // Allow normal Android mixer bursts; discard stale audio after a real stall.
    int limit = audio->target_bytes * 2;
    if (limit < 192 * 30) limit = 192 * 30;
    if (length > limit) { bytes += length - limit; length = limit; }
    if (queued + length > limit) {
        if (!SDL_ClearAudioStream(audio->stream)) return fail(SDL_GetError());
        queued = 0;
    }
    if (!SDL_PutAudioStreamData(audio->stream, bytes, length)) return fail(SDL_GetError());
    if (!audio->resumed && queued + length >= audio->target_bytes) {
        if (!SDL_ResumeAudioStreamDevice(audio->stream)) return fail(SDL_GetError());
        audio->resumed=1;
    }
    return 0;
}
int pp_audio_stats(void *handle, double values[2]) {
    Audio *audio = handle;
    int queued = SDL_GetAudioStreamQueued(audio->stream);
    if (queued < 0) return fail(SDL_GetError());
    values[0] = queued / 192.0;
    values[1] = audio->device_buffer_ms;
    return 0;
}
