//! FFI boundary to SDL3 for window, events, clock and swapchain: the only module of this
//! crate that uses `unsafe` (ADR 0005). GPU work goes through `knorke-gpu`.

use std::cell::Cell;
use std::ffi::CString;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;

use knorke_gpu::kpak::ShaderFormat;
use knorke_gpu::sys::events::{
    SDL_EVENT_GAMEPAD_ADDED, SDL_EVENT_GAMEPAD_AXIS_MOTION, SDL_EVENT_GAMEPAD_BUTTON_DOWN,
    SDL_EVENT_GAMEPAD_BUTTON_UP, SDL_EVENT_KEY_DOWN, SDL_EVENT_KEY_UP, SDL_EVENT_MOUSE_BUTTON_DOWN,
    SDL_EVENT_MOUSE_BUTTON_UP, SDL_EVENT_MOUSE_MOTION, SDL_EVENT_QUIT,
    SDL_EVENT_WINDOW_CLOSE_REQUESTED, SDL_Event, SDL_EventType, SDL_PollEvent,
};
use knorke_gpu::sys::gamepad::SDL_OpenGamepad;
use knorke_gpu::sys::gpu::{
    SDL_ClaimWindowForGPUDevice, SDL_GPU_PRESENTMODE_IMMEDIATE, SDL_GPU_PRESENTMODE_VSYNC,
    SDL_GPU_SHADERFORMAT_DXIL, SDL_GPU_SHADERFORMAT_MSL, SDL_GPU_SHADERFORMAT_SPIRV,
    SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
    SDL_GetGPUSwapchainTextureFormat, SDL_ReleaseWindowFromGPUDevice,
    SDL_SetGPUSwapchainParameters, SDL_WaitAndAcquireGPUSwapchainTexture,
};
use knorke_gpu::sys::init::{
    SDL_INIT_GAMEPAD, SDL_INIT_VIDEO, SDL_InitSubSystem, SDL_QuitSubSystem,
};
use knorke_gpu::sys::timer::SDL_GetTicksNS;
use knorke_gpu::sys::video::{
    SDL_CreateWindow, SDL_DestroyWindow, SDL_WINDOW_RESIZABLE, SDL_Window,
};
use knorke_gpu::{
    CommandBuffer, Device, Gpu, LiveCounter, TextureDesc, TextureRef, device_formats, init_video,
    preferred_format,
};
use knorke_input::{Button, InputEvent, MouseButton, PadAxis, PadButton, Scancode};
use knorke_types::Diagnostic;

use crate::{
    DEFAULT_MAX_FRAME_NANOS, DEVICE_UNAVAILABLE, ShaderPackSpec, cap_elapsed, load_shader_pack,
    shader_dir,
};

/// What [`Host::new`] opens.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// Window title.
    pub title: String,
    /// Window size in pixels.
    pub width: u32,
    /// Window size in pixels.
    pub height: u32,
    /// Wait for the display (default on); off presents immediately.
    pub vsync: bool,
    /// Cap for one frame's `elapsed_nanos` ([`DEFAULT_MAX_FRAME_NANOS`]).
    pub max_frame_nanos: u64,
    /// The game's shader pack, loaded from [`shader_dir`] and checked; `None` for none.
    pub shader_pack: Option<ShaderPackSpec>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            title: "Knorke".into(),
            width: 1280,
            height: 720,
            vsync: true,
            max_frame_nanos: DEFAULT_MAX_FRAME_NANOS,
            shader_pack: None,
        }
    }
}

/// Window, clock, GPU device and swapchain. Owns the frame: modules record into the
/// command buffer of a [`Frame`], only [`Host::end_frame`] submits and presents (ADR 0002).
pub struct Host {
    // Dropped by hand in `Drop`: the window must be released from the device first, and
    // SDL must still be initialised when the device goes.
    device: ManuallyDrop<Device>,
    counter: LiveCounter,
    window: NonNull<SDL_Window>,
    pack: Option<Vec<u8>>,
    max_frame_nanos: u64,
    // `&self` state, so a `Frame` can borrow the device while the next round's state moves.
    last_ns: Cell<u64>,
    // The event buffer travels into the frame and back: no allocation per frame.
    events: Cell<Vec<InputEvent>>,
}

impl Host {
    /// Opens window, device and swapchain and loads the shader pack. Fails with
    /// [`DEVICE_UNAVAILABLE`] or a shader pack diagnostic, never with a fallback.
    pub fn new(config: &HostConfig) -> Result<Self, Diagnostic> {
        let unavailable =
            |what: &str| Diagnostic::error(DEVICE_UNAVAILABLE, format!("{what}: {}", sdl_error()));
        if !init_video(None) {
            return Err(unavailable("initialising SDL video"));
        }
        // SAFETY: plain FFI call; balanced by the quit in `Drop` (SDL counts subsystem refs).
        if !unsafe { SDL_InitSubSystem(SDL_INIT_GAMEPAD) } {
            return Err(unavailable("initialising SDL gamepads"));
        }
        let device = Device::create(
            SDL_GPU_SHADERFORMAT_SPIRV | SDL_GPU_SHADERFORMAT_DXIL | SDL_GPU_SHADERFORMAT_MSL,
            false,
        )
        .map_err(|e| Diagnostic::error(DEVICE_UNAVAILABLE, e.message))?;
        let title = CString::new(config.title.replace('\0', "")).unwrap_or_default();
        // SAFETY: `title` is NUL-terminated and alive for the call; SDL copies it.
        let window = unsafe {
            SDL_CreateWindow(
                title.as_ptr(),
                config.width as i32,
                config.height as i32,
                SDL_WINDOW_RESIZABLE,
            )
        };
        let window = NonNull::new(window).ok_or_else(|| unavailable("creating the window"))?;
        let present = if config.vsync {
            SDL_GPU_PRESENTMODE_VSYNC
        } else {
            SDL_GPU_PRESENTMODE_IMMEDIATE
        };
        // SAFETY: device and window are live. SDR_LINEAR gives an sRGB swapchain: modules
        // write linear light and the conversion to sRGB happens once, on store (ADR 0002).
        let claimed = unsafe {
            SDL_ClaimWindowForGPUDevice(device.as_ptr(), window.as_ptr())
                && SDL_SetGPUSwapchainParameters(
                    device.as_ptr(),
                    window.as_ptr(),
                    SDL_GPU_SWAPCHAINCOMPOSITION_SDR_LINEAR,
                    present,
                )
        };
        let mut host = Self {
            device: ManuallyDrop::new(device),
            counter: LiveCounter::new(),
            window,
            pack: None,
            max_frame_nanos: config.max_frame_nanos,
            // SAFETY: plain FFI call on the monotonic clock.
            last_ns: Cell::new(unsafe { SDL_GetTicksNS() }),
            events: Cell::new(Vec::with_capacity(64)),
        };
        if !claimed {
            // `host` drops here and releases what was claimed.
            return Err(unavailable("linear sRGB swapchain on the window"));
        }
        let Some(spec) = config.shader_pack else {
            return Ok(host);
        };
        let format = preferred_format(device_formats(host.gpu())).ok_or_else(|| {
            Diagnostic::error(DEVICE_UNAVAILABLE, "device takes no SPIR-V, MSL or DXIL")
        })?;
        host.pack = Some(load_shader_pack(&shader_dir(), &spec, format)?);
        Ok(host)
    }

    /// The device with the host's leak counter, for modules to create GPU objects.
    pub fn gpu(&self) -> Gpu<'_> {
        Gpu::new(&self.device, &self.counter)
    }

    /// The checked shader pack bytes (for `ShaderLibrary::new`), if one was configured.
    pub fn shader_pack(&self) -> Option<&[u8]> {
        self.pack.as_deref()
    }

    /// The pack format the device prefers (what [`Host::shader_pack`] holds).
    pub fn shader_format(&self) -> Option<ShaderFormat> {
        preferred_format(device_formats(self.gpu()))
    }

    /// Drains SDL's events, measures the time since the last frame and acquires a command
    /// buffer and the swapchain texture. The game must hand the frame back to
    /// [`Host::end_frame`].
    pub fn begin_frame(&self) -> Result<Frame<'_>, Diagnostic> {
        let mut events = self.events.take();
        events.clear();
        let quit = poll_events(&mut events);
        // SAFETY: plain FFI call on the monotonic clock.
        let now = unsafe { SDL_GetTicksNS() };
        let elapsed_nanos = cap_elapsed(self.last_ns.replace(now), now, self.max_frame_nanos);
        let commands = CommandBuffer::acquire(&self.device)?;
        let (mut raw, mut w, mut h) = (core::ptr::null_mut(), 0, 0);
        // SAFETY: the command buffer is acquired and unsubmitted, the window is claimed by
        // this device; the out pointers are valid locals.
        let ok = unsafe {
            SDL_WaitAndAcquireGPUSwapchainTexture(
                commands.as_ptr(),
                self.window.as_ptr(),
                &mut raw,
                &mut w,
                &mut h,
            )
        };
        if !ok {
            self.events.set(events);
            return Err(Diagnostic::error(
                DEVICE_UNAVAILABLE,
                format!("acquiring the swapchain texture failed: {}", sdl_error()),
            ));
        }
        // SAFETY: device and window are live.
        let format =
            unsafe { SDL_GetGPUSwapchainTextureFormat(self.device.as_ptr(), self.window.as_ptr()) };
        let desc = TextureDesc::new(w, h, format, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET);
        // SAFETY: SDL hands out a live 2D swapchain texture of this device with this size and
        // format; it stays valid until the command buffer is submitted, which only
        // `end_frame` does, and the frame (holding the ref) is consumed there. Null while the
        // window is minimised: then there is nothing to draw into.
        let target =
            NonNull::new(raw).map(|raw| unsafe { TextureRef::from_raw(&self.device, raw, desc) });
        Ok(Frame {
            events,
            quit,
            elapsed_nanos,
            commands,
            target,
        })
    }

    /// Submits the frame's commands; this presents the swapchain texture.
    pub fn end_frame(&self, frame: Frame<'_>) -> Result<(), Diagnostic> {
        let Frame {
            events, commands, ..
        } = frame;
        self.events.set(events);
        commands.submit()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // SAFETY: the window is ours and claimed (or the claim failed, which SDL tolerates);
        // no frame is alive (frames borrow `self`), so no texture of the swapchain is used.
        // The device is dropped once, here, before SDL's subsystems go.
        unsafe {
            SDL_ReleaseWindowFromGPUDevice(self.device.as_ptr(), self.window.as_ptr());
            SDL_DestroyWindow(self.window.as_ptr());
            ManuallyDrop::drop(&mut self.device);
            SDL_QuitSubSystem(SDL_INIT_GAMEPAD | SDL_INIT_VIDEO);
        }
    }
}

/// One round between [`Host::begin_frame`] and [`Host::end_frame`].
pub struct Frame<'h> {
    events: Vec<InputEvent>,
    quit: bool,
    elapsed_nanos: u64,
    commands: CommandBuffer<'h>,
    target: Option<TextureRef<'h>>,
}

impl<'h> Frame<'h> {
    /// Input since the last frame, in SDL's order.
    pub fn input(&self) -> &[InputEvent] {
        &self.events
    }

    /// The window was closed or the program asked to quit.
    pub fn quit(&self) -> bool {
        self.quit
    }

    /// Real time since the last frame, capped; the game passes it to `Runtime::frame`.
    pub fn elapsed_nanos(&self) -> u64 {
        self.elapsed_nanos
    }

    /// The command buffer modules record into; they never submit it.
    pub fn commands(&mut self) -> &mut CommandBuffer<'h> {
        &mut self.commands
    }

    /// The swapchain texture (sRGB, write linear colour); `None` while minimised.
    pub fn target(&self) -> Option<TextureRef<'h>> {
        self.target
    }
}

/// Moves SDL's pending events into `out` as [`InputEvent`]s; true if quit was asked for.
fn poll_events(out: &mut Vec<InputEvent>) -> bool {
    let mut quit = false;
    let mut ev = SDL_Event::default();
    // SAFETY: `ev` is a valid event to write into.
    while unsafe { SDL_PollEvent(&mut ev) } {
        // SAFETY: `r#type` is the common first field of every event variant.
        let kind = SDL_EventType(unsafe { ev.r#type });
        if kind == SDL_EVENT_QUIT || kind == SDL_EVENT_WINDOW_CLOSE_REQUESTED {
            quit = true;
        } else if kind == SDL_EVENT_GAMEPAD_ADDED {
            // Gamepads report buttons only once opened; SDL closes them on quit.
            // SAFETY: the event type says `gdevice` is the active field.
            unsafe { SDL_OpenGamepad(ev.gdevice.which) };
        } else if let Some(input) = translate(&ev) {
            out.push(input);
        }
    }
    quit
}

/// One SDL event as an [`InputEvent`], without meaning; `None` for everything else.
fn translate(ev: &SDL_Event) -> Option<InputEvent> {
    // SAFETY (all union reads): each arm reads the field its event type names; `r#type` is
    // the common first field of every variant.
    let kind = SDL_EventType(unsafe { ev.r#type });
    if kind == SDL_EVENT_KEY_DOWN || kind == SDL_EVENT_KEY_UP {
        let key = unsafe { ev.key };
        let code = u16::try_from(key.scancode.0).ok()?;
        return Some(InputEvent::Button {
            button: Button::Key(Scancode(code)),
            down: key.down,
        });
    }
    if kind == SDL_EVENT_MOUSE_BUTTON_DOWN || kind == SDL_EVENT_MOUSE_BUTTON_UP {
        let b = unsafe { ev.button };
        const MOUSE: [MouseButton; 5] = [
            MouseButton::Left,
            MouseButton::Middle,
            MouseButton::Right,
            MouseButton::X1,
            MouseButton::X2,
        ];
        // SDL numbers mouse buttons from 1.
        let button = *MOUSE.get(usize::from(b.button).checked_sub(1)?)?;
        return Some(InputEvent::Button {
            button: Button::Mouse(button),
            down: b.down,
        });
    }
    if kind == SDL_EVENT_MOUSE_MOTION {
        let m = unsafe { ev.motion };
        return Some(InputEvent::MouseMotion { x: m.x, y: m.y });
    }
    if kind == SDL_EVENT_GAMEPAD_BUTTON_DOWN || kind == SDL_EVENT_GAMEPAD_BUTTON_UP {
        let b = unsafe { ev.gbutton };
        // Same order as SDL_GamepadButton (South … DpadRight).
        use PadButton::*;
        const PAD: [PadButton; 15] = [
            South,
            East,
            West,
            North,
            Back,
            Guide,
            Start,
            LeftStick,
            RightStick,
            LeftShoulder,
            RightShoulder,
            DpadUp,
            DpadDown,
            DpadLeft,
            DpadRight,
        ];
        return Some(InputEvent::Button {
            button: Button::Pad(*PAD.get(usize::from(b.button))?),
            down: b.down,
        });
    }
    if kind == SDL_EVENT_GAMEPAD_AXIS_MOTION {
        let a = unsafe { ev.gaxis };
        // Same order as SDL_GamepadAxis.
        use PadAxis::*;
        const AXES: [PadAxis; 6] = [LeftX, LeftY, RightX, RightY, LeftTrigger, RightTrigger];
        let axis = *AXES.get(usize::from(a.axis))?;
        // -32768 would be just below -1; clamp to keep sticks in -1..=1.
        let value = (f32::from(a.value) / 32767.0).clamp(-1.0, 1.0);
        return Some(InputEvent::Axis { axis, value });
    }
    None
}

fn sdl_error() -> String {
    let p = knorke_gpu::sys::error::SDL_GetError();
    if p.is_null() {
        return String::new();
    }
    // SAFETY: SDL_GetError returns a NUL-terminated string owned by SDL, valid until the
    // next SDL call on this thread; copied out at once.
    unsafe { std::ffi::CStr::from_ptr(p) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_gpu::sys::events::{SDL_KeyboardEvent, SDL_MouseButtonEvent, SDL_PushEvent};
    use knorke_gpu::sys::scancode::SDL_SCANCODE_UP;

    /// Translation through SDL's real queue on the dummy video driver: no window, no GPU.
    #[test]
    fn sdl_events_become_input_events() {
        // SAFETY: hint name and value are NUL-terminated; SDL copies them. This is the only
        // test in the binary that touches SDL.
        unsafe {
            knorke_gpu::sys::hints::SDL_SetHint(
                knorke_gpu::sys::hints::SDL_HINT_VIDEO_DRIVER,
                c"dummy".as_ptr(),
            );
            assert!(SDL_InitSubSystem(SDL_INIT_VIDEO), "{}", sdl_error());
        }
        let mut key = SDL_Event {
            key: SDL_KeyboardEvent {
                r#type: SDL_EVENT_KEY_DOWN,
                scancode: SDL_SCANCODE_UP,
                down: true,
                ..Default::default()
            },
        };
        let mut click = SDL_Event {
            button: SDL_MouseButtonEvent {
                r#type: SDL_EVENT_MOUSE_BUTTON_UP,
                button: 3,
                down: false,
                ..Default::default()
            },
        };
        let mut quit = SDL_Event::default();
        quit.r#type = SDL_EVENT_QUIT.0;
        let mut out = Vec::new();
        // SAFETY: the events are fully initialised; SDL copies them into its queue.
        unsafe {
            poll_events(&mut out); // whatever the driver queued at init
            assert!(SDL_PushEvent(&mut key));
            assert!(SDL_PushEvent(&mut click));
            assert!(SDL_PushEvent(&mut quit));
        }
        out.clear();
        assert!(poll_events(&mut out));
        assert_eq!(
            out,
            [
                InputEvent::Button {
                    button: Button::Key(Scancode::UP),
                    down: true
                },
                InputEvent::Button {
                    button: Button::Mouse(MouseButton::Right),
                    down: false
                },
            ]
        );
        // SAFETY: balances the init above.
        unsafe { SDL_QuitSubSystem(SDL_INIT_VIDEO) };
    }
}
