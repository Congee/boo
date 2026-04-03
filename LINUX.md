# Linux Port Status

## Architecture

On macOS, ghostty renders into an NSView child via Metal/IOSurface. The macOS
compositor composites the NSView on top of the iced window. Zero-copy.

On Linux, ghostty renders into an offscreen EGL context (OpenGL 4.6 on AMD via
Mesa). After each frame, `glReadPixels` copies pixels to shared memory. iced
displays the frame as an image widget.

## What Works

- **Platform abstraction**: `src/platform/mod.rs` with cfg-gated `macos.rs` and
  `linux.rs` backends sharing identical API surface
- **EGL context**: Device platform (`eglQueryDevicesEXT` + `EGL_PLATFORM_DEVICE_EXT`)
  gives desktop OpenGL 4.3+ without display server dependency
- **ghostty initialization**: App + surface created, shell spawns, config loads
- **Renderer thread**: Starts successfully, `eglMakeCurrent` succeeds on renderer
  thread, `gladLoaderLoadGLContext` loads OpenGL 4.6, shaders compile on AMD GPU
- **Frame callback**: `frame_cb` in `ghostty_platform_egl_s` fires after each
  `present()` with the target FBO bound for reading. `glReadPixels` captures
  pixels to `Arc<Mutex<FrameData>>`
- **Frame display**: iced `image` widget shows the captured frame (verified with
  test patterns and actual terminal background)
- **Terminal background**: Renders correctly at 1024x748 with 265 unique colors

## What Doesn't Work

### Text rendering in embedded EGL mode

ghostty's OpenGL renderer draws the terminal background but does NOT render text
glyphs or cursor into the FBO. The shell is running (PTY is active, zsh spawns)
but the rendered frame contains only background color.

**Root cause**: Unknown. Likely one of:

1. **Font atlas upload fails**: ghostty uploads font glyphs to GL textures during
   rendering. On the EGL device platform (no window surface), texture uploads may
   fail silently or the atlas isn't initialized properly.

2. **Text shader rendering fails**: The cell/text rendering passes may depend on
   GL state that's only set up correctly with a real window surface (GTK's
   GtkGLArea sets viewport, scissor, etc.).

3. **Terminal grid is empty**: The PTY output may not reach the terminal grid
   before frames are rendered, though this is unlikely since the shell prompt
   should appear immediately.

4. **sRGB framebuffer issues**: ghostty toggles `GL_FRAMEBUFFER_SRGB` during
   rendering. On a pbuffer without sRGB support, this may cause the text pass
   to produce invisible output.

### Investigation needed

- Check ghostty's font atlas initialization for embedded mode
- Check if the cell rendering pass (`renderCells`) produces any draw calls
- Check GL error state after each rendering pass
- Compare the rendering flow between GTK (working) and embedded EGL (broken)

## Ghostty Zig Changes (in `ghostty/` submodule)

### `include/ghostty.h`
- Added `GHOSTTY_PLATFORM_EGL = 3` to platform enum
- Added `ghostty_platform_egl_s` with display, surface, context, frame_cb,
  frame_cb_userdata
- Added `ghostty_frame_cb` typedef

### `src/apprt/embedded.zig`
- Added `EGL` variant to `Platform` union with frame callback fields
- Added `egl = 3` to `PlatformTag` enum
- Added EGL initialization in `Platform.init()`

### `src/renderer/OpenGL.zig`
- Added EGL type aliases and `eglMakeCurrent`/`eglSwapBuffers` extern declarations
- Added `egl_display`, `egl_surface`, `frame_cb`, `frame_cb_userdata`,
  `surface_width`, `surface_height` fields
- `surfaceInit`: makes EGL context current, calls `prepareContext(null)`
- `finalizeSurfaceInit`: releases EGL context from main thread
- `threadEnter`: claims EGL context on renderer thread, stores handles
- `drawFrameStart`: sets `glViewport` from stored surface size
- `surfaceSize`: returns stored size instead of `GL_VIEWPORT` (pbuffer is 1x1)
- `setSurfaceSize`: called by generic renderer on resize
- `present`: calls `eglSwapBuffers` and `frame_cb` after blit

### `src/renderer/generic.zig`
- `setScreenSize`: calls `api.setSurfaceSize()` if available

### `src/build/SharedDeps.zig`
- Moved glad compilation out of `if (step.kind != .lib)` so library targets
  get glad too
- Added `egl` system library link for library targets on non-Darwin

## NixOS-Specific Issues

### EGL vendor dispatch
- libglvnd's `libEGL.so.1` needs `__EGL_VENDOR_LIBRARY_DIRS` pointing to
  `/run/opengl-driver/share/glvnd/egl_vendor.d` for Mesa vendor discovery
- Mesa's DRI driver (`radeonsi_dri.so`) and its dependencies (`libdrm_amdgpu`)
  must be on `LD_LIBRARY_PATH` — the system mesa at `/run/opengl-driver` has
  them but the nix-store mesa from buildInputs may not match
- `eglGetDisplay(DEFAULT_DISPLAY)` returns EGL 0.0 on Wayland because Mesa's
  Wayland EGL only exposes GLES, not desktop GL
- Solution: `eglQueryDevicesEXT` + `EGL_PLATFORM_DEVICE_EXT` for direct GPU
  access with desktop GL 4.6

### LLVM stdenv
- `pkgsLLVM.llvmPackages_latest.stdenv.mkDerivation` provides libc++ for linking
- Injects `-fmacro-prefix-map` flags in `NIX_CFLAGS_COMPILE` that zig doesn't
  understand — must be stripped in shellHook
- `LD_LIBRARY_PATH` must include system mesa lib path (resolved from
  `/run/opengl-driver/lib/libEGL_mesa.so.0` symlink) for mesa's transitive
  dependencies

## File Layout

```
src/platform/
  mod.rs         — Rect/Point/Size types, ScrollEvent, cfg re-exports
  macos.rs       — AppKit: NSView, CALayer, NSPasteboard, NSEvent monitors
  linux.rs       — EGL context, glReadPixels callback, arboard clipboard

ghostty/         — submodule with Zig patches for EGL platform support
build.rs         — links libghostty.so, libEGL, libGL on Linux
flake.nix        — pkgsLLVM stdenv, LD_LIBRARY_PATH, __EGL_VENDOR_LIBRARY_DIRS
```
