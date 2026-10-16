/* Generated declarations; the engine source below is maintained separately. */
static __typeof__(avcodec_version) *p_avcodec_version;
static __typeof__(avcodec_find_decoder_by_name) *p_avcodec_find_decoder_by_name;
static __typeof__(avcodec_alloc_context3) *p_avcodec_alloc_context3;
static __typeof__(avcodec_open2) *p_avcodec_open2;
static __typeof__(avcodec_free_context) *p_avcodec_free_context;
static __typeof__(avcodec_send_packet) *p_avcodec_send_packet;
static __typeof__(avcodec_receive_frame) *p_avcodec_receive_frame;
static __typeof__(av_packet_alloc) *p_av_packet_alloc;
static __typeof__(av_packet_free) *p_av_packet_free;
static __typeof__(av_new_packet) *p_av_new_packet;
static __typeof__(av_packet_unref) *p_av_packet_unref;
static __typeof__(av_frame_alloc) *p_av_frame_alloc;
static __typeof__(av_frame_clone) *p_av_frame_clone;
static __typeof__(av_frame_free) *p_av_frame_free;
static __typeof__(av_frame_unref) *p_av_frame_unref;
static __typeof__(av_hwdevice_ctx_create) *p_av_hwdevice_ctx_create;
static __typeof__(av_hwdevice_ctx_alloc) *p_av_hwdevice_ctx_alloc;
static __typeof__(av_hwdevice_ctx_init) *p_av_hwdevice_ctx_init;
static __typeof__(av_hwframe_transfer_data) *p_av_hwframe_transfer_data;
static __typeof__(av_buffer_ref) *p_av_buffer_ref;
static __typeof__(av_buffer_unref) *p_av_buffer_unref;
static __typeof__(av_strerror) *p_av_strerror;
static __typeof__(SDL_Init) *p_SDL_Init;
static __typeof__(SDL_InitSubSystem) *p_SDL_InitSubSystem;
static __typeof__(SDL_Quit) *p_SDL_Quit;
static __typeof__(SDL_GetError) *p_SDL_GetError;
static __typeof__(SDL_SetHint) *p_SDL_SetHint;
static __typeof__(SDL_CreateWindow) *p_SDL_CreateWindow;
static __typeof__(SDL_DestroyWindow) *p_SDL_DestroyWindow;
static __typeof__(SDL_CreateRenderer) *p_SDL_CreateRenderer;
static __typeof__(SDL_DestroyRenderer) *p_SDL_DestroyRenderer;
static __typeof__(SDL_GetRendererName) *p_SDL_GetRendererName;
static __typeof__(SDL_SetRenderVSync) *p_SDL_SetRenderVSync;
static __typeof__(SDL_CreateTexture) *p_SDL_CreateTexture;
static __typeof__(SDL_DestroyTexture) *p_SDL_DestroyTexture;
static __typeof__(SDL_SetTextureScaleMode) *p_SDL_SetTextureScaleMode;
static __typeof__(SDL_UpdateNVTexture) *p_SDL_UpdateNVTexture;
static __typeof__(SDL_UpdateYUVTexture) *p_SDL_UpdateYUVTexture;
static __typeof__(SDL_GetWindowSizeInPixels) *p_SDL_GetWindowSizeInPixels;
static __typeof__(SDL_RenderClear) *p_SDL_RenderClear;
static __typeof__(SDL_RenderTexture) *p_SDL_RenderTexture;
static __typeof__(SDL_RenderPresent) *p_SDL_RenderPresent;
static __typeof__(SDL_PollEvent) *p_SDL_PollEvent;
static __typeof__(SDL_GetDisplayUsableBounds) *p_SDL_GetDisplayUsableBounds;
static __typeof__(SDL_GetPrimaryDisplay) *p_SDL_GetPrimaryDisplay;
static __typeof__(SDL_SetWindowSize) *p_SDL_SetWindowSize;
static __typeof__(SDL_OpenAudioDeviceStream) *p_SDL_OpenAudioDeviceStream;
static __typeof__(SDL_ResumeAudioStreamDevice) *p_SDL_ResumeAudioStreamDevice;
static __typeof__(SDL_GetAudioStreamQueued) *p_SDL_GetAudioStreamQueued;
static __typeof__(SDL_ClearAudioStream) *p_SDL_ClearAudioStream;
static __typeof__(SDL_PutAudioStreamData) *p_SDL_PutAudioStreamData;
static __typeof__(SDL_DestroyAudioStream) *p_SDL_DestroyAudioStream;
static __typeof__(SDL_GetAudioStreamDevice) *p_SDL_GetAudioStreamDevice;
static __typeof__(SDL_GetAudioDeviceFormat) *p_SDL_GetAudioDeviceFormat;
static __typeof__(SDL_GetRendererProperties) *p_SDL_GetRendererProperties;
static __typeof__(SDL_GetPointerProperty) *p_SDL_GetPointerProperty;
static __typeof__(SDL_CreateProperties) *p_SDL_CreateProperties;
static __typeof__(SDL_SetNumberProperty) *p_SDL_SetNumberProperty;
static __typeof__(SDL_SetPointerProperty) *p_SDL_SetPointerProperty;
static __typeof__(SDL_DestroyProperties) *p_SDL_DestroyProperties;
static __typeof__(SDL_CreateTextureWithProperties) *p_SDL_CreateTextureWithProperties;
static __typeof__(SDL_RenderReadPixels) *p_SDL_RenderReadPixels;
static __typeof__(SDL_SaveBMP) *p_SDL_SaveBMP;
static __typeof__(SDL_DestroySurface) *p_SDL_DestroySurface;
static __typeof__(SDL_CreateSurfaceFrom) *p_SDL_CreateSurfaceFrom;
static __typeof__(SDL_SetWindowIcon) *p_SDL_SetWindowIcon;
static __typeof__(SDL_GetWindowFlags) *p_SDL_GetWindowFlags;
static __typeof__(SDL_SetWindowAspectRatio) *p_SDL_SetWindowAspectRatio;
static __typeof__(SDL_GetRenderOutputSize) *p_SDL_GetRenderOutputSize;
static __typeof__(SDL_GetRenderVSync) *p_SDL_GetRenderVSync;
static __typeof__(SDL_GetDisplayForWindow) *p_SDL_GetDisplayForWindow;
static __typeof__(SDL_GetWindowPosition) *p_SDL_GetWindowPosition;
static __typeof__(SDL_GetWindowSize) *p_SDL_GetWindowSize;
static __typeof__(SDL_GetWindowBordersSize) *p_SDL_GetWindowBordersSize;
static __typeof__(SDL_SetWindowPosition) *p_SDL_SetWindowPosition;
static __typeof__(SDL_SetWindowFullscreen) *p_SDL_SetWindowFullscreen;
static int load_symbols(HMODULE codec, HMODULE util, HMODULE sdl) {
    p_SDL_GetAudioStreamDevice = (__typeof__(SDL_GetAudioStreamDevice)*) GetProcAddress(sdl, "SDL_GetAudioStreamDevice"); if (!p_SDL_GetAudioStreamDevice) return 0;
    p_SDL_GetAudioDeviceFormat = (__typeof__(SDL_GetAudioDeviceFormat)*) GetProcAddress(sdl, "SDL_GetAudioDeviceFormat"); if (!p_SDL_GetAudioDeviceFormat) return 0;
    p_avcodec_version = (__typeof__(avcodec_version)*) GetProcAddress(codec, "avcodec_version"); if (!p_avcodec_version) return 0;
    p_avcodec_find_decoder_by_name = (__typeof__(avcodec_find_decoder_by_name)*) GetProcAddress(codec, "avcodec_find_decoder_by_name"); if (!p_avcodec_find_decoder_by_name) return 0;
    p_avcodec_alloc_context3 = (__typeof__(avcodec_alloc_context3)*) GetProcAddress(codec, "avcodec_alloc_context3"); if (!p_avcodec_alloc_context3) return 0;
    p_avcodec_open2 = (__typeof__(avcodec_open2)*) GetProcAddress(codec, "avcodec_open2"); if (!p_avcodec_open2) return 0;
    p_avcodec_free_context = (__typeof__(avcodec_free_context)*) GetProcAddress(codec, "avcodec_free_context"); if (!p_avcodec_free_context) return 0;
    p_avcodec_send_packet = (__typeof__(avcodec_send_packet)*) GetProcAddress(codec, "avcodec_send_packet"); if (!p_avcodec_send_packet) return 0;
    p_avcodec_receive_frame = (__typeof__(avcodec_receive_frame)*) GetProcAddress(codec, "avcodec_receive_frame"); if (!p_avcodec_receive_frame) return 0;
    p_av_packet_alloc = (__typeof__(av_packet_alloc)*) GetProcAddress(codec, "av_packet_alloc"); if (!p_av_packet_alloc) return 0;
    p_av_packet_free = (__typeof__(av_packet_free)*) GetProcAddress(codec, "av_packet_free"); if (!p_av_packet_free) return 0;
    p_av_new_packet = (__typeof__(av_new_packet)*) GetProcAddress(codec, "av_new_packet"); if (!p_av_new_packet) return 0;
    p_av_packet_unref = (__typeof__(av_packet_unref)*) GetProcAddress(codec, "av_packet_unref"); if (!p_av_packet_unref) return 0;
    p_av_frame_alloc = (__typeof__(av_frame_alloc)*) GetProcAddress(util, "av_frame_alloc"); if (!p_av_frame_alloc) return 0;
    p_av_frame_clone = (__typeof__(av_frame_clone)*) GetProcAddress(util, "av_frame_clone"); if (!p_av_frame_clone) return 0;
    p_av_frame_free = (__typeof__(av_frame_free)*) GetProcAddress(util, "av_frame_free"); if (!p_av_frame_free) return 0;
    p_av_frame_unref = (__typeof__(av_frame_unref)*) GetProcAddress(util, "av_frame_unref"); if (!p_av_frame_unref) return 0;
    p_av_hwdevice_ctx_create = (__typeof__(av_hwdevice_ctx_create)*) GetProcAddress(util, "av_hwdevice_ctx_create"); if (!p_av_hwdevice_ctx_create) return 0;
    p_av_hwdevice_ctx_alloc = (__typeof__(av_hwdevice_ctx_alloc)*) GetProcAddress(util, "av_hwdevice_ctx_alloc"); if (!p_av_hwdevice_ctx_alloc) return 0;
    p_av_hwdevice_ctx_init = (__typeof__(av_hwdevice_ctx_init)*) GetProcAddress(util, "av_hwdevice_ctx_init"); if (!p_av_hwdevice_ctx_init) return 0;
    p_av_hwframe_transfer_data = (__typeof__(av_hwframe_transfer_data)*) GetProcAddress(util, "av_hwframe_transfer_data"); if (!p_av_hwframe_transfer_data) return 0;
    p_av_buffer_ref = (__typeof__(av_buffer_ref)*) GetProcAddress(util, "av_buffer_ref"); if (!p_av_buffer_ref) return 0;
    p_av_buffer_unref = (__typeof__(av_buffer_unref)*) GetProcAddress(util, "av_buffer_unref"); if (!p_av_buffer_unref) return 0;
    p_av_strerror = (__typeof__(av_strerror)*) GetProcAddress(util, "av_strerror"); if (!p_av_strerror) return 0;
    p_SDL_Init = (__typeof__(SDL_Init)*) GetProcAddress(sdl, "SDL_Init"); if (!p_SDL_Init) return 0;
    p_SDL_InitSubSystem = (__typeof__(SDL_InitSubSystem)*) GetProcAddress(sdl, "SDL_InitSubSystem"); if (!p_SDL_InitSubSystem) return 0;
    p_SDL_Quit = (__typeof__(SDL_Quit)*) GetProcAddress(sdl, "SDL_Quit"); if (!p_SDL_Quit) return 0;
    p_SDL_GetError = (__typeof__(SDL_GetError)*) GetProcAddress(sdl, "SDL_GetError"); if (!p_SDL_GetError) return 0;
    p_SDL_SetHint = (__typeof__(SDL_SetHint)*) GetProcAddress(sdl, "SDL_SetHint"); if (!p_SDL_SetHint) return 0;
    p_SDL_CreateWindow = (__typeof__(SDL_CreateWindow)*) GetProcAddress(sdl, "SDL_CreateWindow"); if (!p_SDL_CreateWindow) return 0;
    p_SDL_DestroyWindow = (__typeof__(SDL_DestroyWindow)*) GetProcAddress(sdl, "SDL_DestroyWindow"); if (!p_SDL_DestroyWindow) return 0;
    p_SDL_CreateRenderer = (__typeof__(SDL_CreateRenderer)*) GetProcAddress(sdl, "SDL_CreateRenderer"); if (!p_SDL_CreateRenderer) return 0;
    p_SDL_DestroyRenderer = (__typeof__(SDL_DestroyRenderer)*) GetProcAddress(sdl, "SDL_DestroyRenderer"); if (!p_SDL_DestroyRenderer) return 0;
    p_SDL_GetRendererName = (__typeof__(SDL_GetRendererName)*) GetProcAddress(sdl, "SDL_GetRendererName"); if (!p_SDL_GetRendererName) return 0;
    p_SDL_SetRenderVSync = (__typeof__(SDL_SetRenderVSync)*) GetProcAddress(sdl, "SDL_SetRenderVSync"); if (!p_SDL_SetRenderVSync) return 0;
    p_SDL_CreateTexture = (__typeof__(SDL_CreateTexture)*) GetProcAddress(sdl, "SDL_CreateTexture"); if (!p_SDL_CreateTexture) return 0;
    p_SDL_DestroyTexture = (__typeof__(SDL_DestroyTexture)*) GetProcAddress(sdl, "SDL_DestroyTexture"); if (!p_SDL_DestroyTexture) return 0;
    p_SDL_SetTextureScaleMode = (__typeof__(SDL_SetTextureScaleMode)*) GetProcAddress(sdl, "SDL_SetTextureScaleMode"); if (!p_SDL_SetTextureScaleMode) return 0;
    p_SDL_UpdateNVTexture = (__typeof__(SDL_UpdateNVTexture)*) GetProcAddress(sdl, "SDL_UpdateNVTexture"); if (!p_SDL_UpdateNVTexture) return 0;
    p_SDL_UpdateYUVTexture = (__typeof__(SDL_UpdateYUVTexture)*) GetProcAddress(sdl, "SDL_UpdateYUVTexture"); if (!p_SDL_UpdateYUVTexture) return 0;
    p_SDL_GetWindowSizeInPixels = (__typeof__(SDL_GetWindowSizeInPixels)*) GetProcAddress(sdl, "SDL_GetWindowSizeInPixels"); if (!p_SDL_GetWindowSizeInPixels) return 0;
    p_SDL_RenderClear = (__typeof__(SDL_RenderClear)*) GetProcAddress(sdl, "SDL_RenderClear"); if (!p_SDL_RenderClear) return 0;
    p_SDL_RenderTexture = (__typeof__(SDL_RenderTexture)*) GetProcAddress(sdl, "SDL_RenderTexture"); if (!p_SDL_RenderTexture) return 0;
    p_SDL_RenderPresent = (__typeof__(SDL_RenderPresent)*) GetProcAddress(sdl, "SDL_RenderPresent"); if (!p_SDL_RenderPresent) return 0;
    p_SDL_PollEvent = (__typeof__(SDL_PollEvent)*) GetProcAddress(sdl, "SDL_PollEvent"); if (!p_SDL_PollEvent) return 0;
    p_SDL_GetDisplayUsableBounds = (__typeof__(SDL_GetDisplayUsableBounds)*) GetProcAddress(sdl, "SDL_GetDisplayUsableBounds"); if (!p_SDL_GetDisplayUsableBounds) return 0;
    p_SDL_GetPrimaryDisplay = (__typeof__(SDL_GetPrimaryDisplay)*) GetProcAddress(sdl, "SDL_GetPrimaryDisplay"); if (!p_SDL_GetPrimaryDisplay) return 0;
    p_SDL_SetWindowSize = (__typeof__(SDL_SetWindowSize)*) GetProcAddress(sdl, "SDL_SetWindowSize"); if (!p_SDL_SetWindowSize) return 0;
    p_SDL_OpenAudioDeviceStream = (__typeof__(SDL_OpenAudioDeviceStream)*) GetProcAddress(sdl, "SDL_OpenAudioDeviceStream"); if (!p_SDL_OpenAudioDeviceStream) return 0;
    p_SDL_ResumeAudioStreamDevice = (__typeof__(SDL_ResumeAudioStreamDevice)*) GetProcAddress(sdl, "SDL_ResumeAudioStreamDevice"); if (!p_SDL_ResumeAudioStreamDevice) return 0;
    p_SDL_GetAudioStreamQueued = (__typeof__(SDL_GetAudioStreamQueued)*) GetProcAddress(sdl, "SDL_GetAudioStreamQueued"); if (!p_SDL_GetAudioStreamQueued) return 0;
    p_SDL_ClearAudioStream = (__typeof__(SDL_ClearAudioStream)*) GetProcAddress(sdl, "SDL_ClearAudioStream"); if (!p_SDL_ClearAudioStream) return 0;
    p_SDL_PutAudioStreamData = (__typeof__(SDL_PutAudioStreamData)*) GetProcAddress(sdl, "SDL_PutAudioStreamData"); if (!p_SDL_PutAudioStreamData) return 0;
    p_SDL_DestroyAudioStream = (__typeof__(SDL_DestroyAudioStream)*) GetProcAddress(sdl, "SDL_DestroyAudioStream"); if (!p_SDL_DestroyAudioStream) return 0;
    p_SDL_GetRendererProperties = (__typeof__(SDL_GetRendererProperties)*) GetProcAddress(sdl, "SDL_GetRendererProperties"); if (!p_SDL_GetRendererProperties) return 0;
    p_SDL_GetPointerProperty = (__typeof__(SDL_GetPointerProperty)*) GetProcAddress(sdl, "SDL_GetPointerProperty"); if (!p_SDL_GetPointerProperty) return 0;
    p_SDL_CreateProperties = (__typeof__(SDL_CreateProperties)*) GetProcAddress(sdl, "SDL_CreateProperties"); if (!p_SDL_CreateProperties) return 0;
    p_SDL_SetNumberProperty = (__typeof__(SDL_SetNumberProperty)*) GetProcAddress(sdl, "SDL_SetNumberProperty"); if (!p_SDL_SetNumberProperty) return 0;
    p_SDL_SetPointerProperty = (__typeof__(SDL_SetPointerProperty)*) GetProcAddress(sdl, "SDL_SetPointerProperty"); if (!p_SDL_SetPointerProperty) return 0;
    p_SDL_DestroyProperties = (__typeof__(SDL_DestroyProperties)*) GetProcAddress(sdl, "SDL_DestroyProperties"); if (!p_SDL_DestroyProperties) return 0;
    p_SDL_CreateTextureWithProperties = (__typeof__(SDL_CreateTextureWithProperties)*) GetProcAddress(sdl, "SDL_CreateTextureWithProperties"); if (!p_SDL_CreateTextureWithProperties) return 0;
    p_SDL_RenderReadPixels = (__typeof__(SDL_RenderReadPixels)*) GetProcAddress(sdl, "SDL_RenderReadPixels"); if (!p_SDL_RenderReadPixels) return 0;
    p_SDL_SaveBMP = (__typeof__(SDL_SaveBMP)*) GetProcAddress(sdl, "SDL_SaveBMP"); if (!p_SDL_SaveBMP) return 0;
    p_SDL_DestroySurface = (__typeof__(SDL_DestroySurface)*) GetProcAddress(sdl, "SDL_DestroySurface"); if (!p_SDL_DestroySurface) return 0;
    p_SDL_CreateSurfaceFrom = (__typeof__(SDL_CreateSurfaceFrom)*) GetProcAddress(sdl, "SDL_CreateSurfaceFrom"); if (!p_SDL_CreateSurfaceFrom) return 0;
    p_SDL_SetWindowIcon = (__typeof__(SDL_SetWindowIcon)*) GetProcAddress(sdl, "SDL_SetWindowIcon"); if (!p_SDL_SetWindowIcon) return 0;
    p_SDL_GetWindowFlags = (__typeof__(SDL_GetWindowFlags)*) GetProcAddress(sdl, "SDL_GetWindowFlags"); if (!p_SDL_GetWindowFlags) return 0;
    p_SDL_SetWindowAspectRatio = (__typeof__(SDL_SetWindowAspectRatio)*) GetProcAddress(sdl, "SDL_SetWindowAspectRatio"); if (!p_SDL_SetWindowAspectRatio) return 0;
    p_SDL_GetRenderOutputSize = (__typeof__(SDL_GetRenderOutputSize)*) GetProcAddress(sdl, "SDL_GetRenderOutputSize"); if (!p_SDL_GetRenderOutputSize) return 0;
    p_SDL_GetRenderVSync = (__typeof__(SDL_GetRenderVSync)*) GetProcAddress(sdl, "SDL_GetRenderVSync"); if (!p_SDL_GetRenderVSync) return 0;
    p_SDL_GetDisplayForWindow = (__typeof__(SDL_GetDisplayForWindow)*) GetProcAddress(sdl, "SDL_GetDisplayForWindow"); if (!p_SDL_GetDisplayForWindow) return 0;
    p_SDL_GetWindowPosition = (__typeof__(SDL_GetWindowPosition)*) GetProcAddress(sdl, "SDL_GetWindowPosition"); if (!p_SDL_GetWindowPosition) return 0;
    p_SDL_GetWindowSize = (__typeof__(SDL_GetWindowSize)*) GetProcAddress(sdl, "SDL_GetWindowSize"); if (!p_SDL_GetWindowSize) return 0;
    p_SDL_GetWindowBordersSize = (__typeof__(SDL_GetWindowBordersSize)*) GetProcAddress(sdl, "SDL_GetWindowBordersSize"); if (!p_SDL_GetWindowBordersSize) return 0;
    p_SDL_SetWindowPosition = (__typeof__(SDL_SetWindowPosition)*) GetProcAddress(sdl, "SDL_SetWindowPosition"); if (!p_SDL_SetWindowPosition) return 0;
    p_SDL_SetWindowFullscreen = (__typeof__(SDL_SetWindowFullscreen)*) GetProcAddress(sdl, "SDL_SetWindowFullscreen"); if (!p_SDL_SetWindowFullscreen) return 0;
    return 1;
}
#define avcodec_version p_avcodec_version
#define avcodec_find_decoder_by_name p_avcodec_find_decoder_by_name
#define avcodec_alloc_context3 p_avcodec_alloc_context3
#define avcodec_open2 p_avcodec_open2
#define avcodec_free_context p_avcodec_free_context
#define avcodec_send_packet p_avcodec_send_packet
#define avcodec_receive_frame p_avcodec_receive_frame
#define av_packet_alloc p_av_packet_alloc
#define av_packet_free p_av_packet_free
#define av_new_packet p_av_new_packet
#define av_packet_unref p_av_packet_unref
#define av_frame_alloc p_av_frame_alloc
#define av_frame_clone p_av_frame_clone
#define av_frame_free p_av_frame_free
#define av_frame_unref p_av_frame_unref
#define av_hwdevice_ctx_create p_av_hwdevice_ctx_create
#define av_hwdevice_ctx_alloc p_av_hwdevice_ctx_alloc
#define av_hwdevice_ctx_init p_av_hwdevice_ctx_init
#define av_hwframe_transfer_data p_av_hwframe_transfer_data
#define av_buffer_ref p_av_buffer_ref
#define av_buffer_unref p_av_buffer_unref
#define av_strerror p_av_strerror
#define SDL_Init p_SDL_Init
#define SDL_InitSubSystem p_SDL_InitSubSystem
#define SDL_Quit p_SDL_Quit
#define SDL_GetError p_SDL_GetError
#define SDL_SetHint p_SDL_SetHint
#define SDL_CreateWindow p_SDL_CreateWindow
#define SDL_DestroyWindow p_SDL_DestroyWindow
#define SDL_CreateRenderer p_SDL_CreateRenderer
#define SDL_DestroyRenderer p_SDL_DestroyRenderer
#define SDL_GetRendererName p_SDL_GetRendererName
#define SDL_SetRenderVSync p_SDL_SetRenderVSync
#define SDL_CreateTexture p_SDL_CreateTexture
#define SDL_DestroyTexture p_SDL_DestroyTexture
#define SDL_SetTextureScaleMode p_SDL_SetTextureScaleMode
#define SDL_UpdateNVTexture p_SDL_UpdateNVTexture
#define SDL_UpdateYUVTexture p_SDL_UpdateYUVTexture
#define SDL_GetWindowSizeInPixels p_SDL_GetWindowSizeInPixels
#define SDL_RenderClear p_SDL_RenderClear
#define SDL_RenderTexture p_SDL_RenderTexture
#define SDL_RenderPresent p_SDL_RenderPresent
#define SDL_PollEvent p_SDL_PollEvent
#define SDL_GetDisplayUsableBounds p_SDL_GetDisplayUsableBounds
#define SDL_GetPrimaryDisplay p_SDL_GetPrimaryDisplay
#define SDL_SetWindowSize p_SDL_SetWindowSize
#define SDL_OpenAudioDeviceStream p_SDL_OpenAudioDeviceStream
#define SDL_ResumeAudioStreamDevice p_SDL_ResumeAudioStreamDevice
#define SDL_GetAudioStreamQueued p_SDL_GetAudioStreamQueued
#define SDL_ClearAudioStream p_SDL_ClearAudioStream
#define SDL_PutAudioStreamData p_SDL_PutAudioStreamData
#define SDL_DestroyAudioStream p_SDL_DestroyAudioStream
#define SDL_GetAudioStreamDevice p_SDL_GetAudioStreamDevice
#define SDL_GetAudioDeviceFormat p_SDL_GetAudioDeviceFormat
#define SDL_GetRendererProperties p_SDL_GetRendererProperties
#define SDL_GetPointerProperty p_SDL_GetPointerProperty
#define SDL_CreateProperties p_SDL_CreateProperties
#define SDL_SetNumberProperty p_SDL_SetNumberProperty
#define SDL_SetPointerProperty p_SDL_SetPointerProperty
#define SDL_DestroyProperties p_SDL_DestroyProperties
#define SDL_CreateTextureWithProperties p_SDL_CreateTextureWithProperties
#define SDL_RenderReadPixels p_SDL_RenderReadPixels
#define SDL_SaveBMP p_SDL_SaveBMP
#define SDL_DestroySurface p_SDL_DestroySurface
#define SDL_CreateSurfaceFrom p_SDL_CreateSurfaceFrom
#define SDL_SetWindowIcon p_SDL_SetWindowIcon
#define SDL_GetWindowFlags p_SDL_GetWindowFlags
#define SDL_SetWindowAspectRatio p_SDL_SetWindowAspectRatio
#define SDL_GetRenderOutputSize p_SDL_GetRenderOutputSize
#define SDL_GetRenderVSync p_SDL_GetRenderVSync
#define SDL_GetDisplayForWindow p_SDL_GetDisplayForWindow
#define SDL_GetWindowPosition p_SDL_GetWindowPosition
#define SDL_GetWindowSize p_SDL_GetWindowSize
#define SDL_GetWindowBordersSize p_SDL_GetWindowBordersSize
#define SDL_SetWindowPosition p_SDL_SetWindowPosition
#define SDL_SetWindowFullscreen p_SDL_SetWindowFullscreen
