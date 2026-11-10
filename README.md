# Berick

Berick is a web browser written in Rust, built on the [Servo](https://servo.org/) engine.
The goal is a daily-driver browser in the same league as Chrome, Brave and Firefox.

This is the first milestone: a single-window browser you can surf with.

## What works

- Address bar: type a URL, a bare domain (`example.com`), a local address
  (`localhost:8080`), or anything else to search DuckDuckGo
- Back, forward and reload buttons, a loading indicator, and the page title in the window title
- Pop-ups (`target=_blank`, `window.open`) open on top of the current page until tabs land
- Keyboard shortcuts:

| Shortcut | Action |
| --- | --- |
| Ctrl+L, Alt+D, F6 | Focus the address bar |
| Enter / Esc in the address bar | Go / undo the edit |
| Ctrl+R, F5 | Reload |
| Alt+Left, Alt+Right (Cmd+[ / Cmd+] on macOS) | Back, forward |
| Mouse back/forward buttons | Back, forward |
| Ctrl+= / Ctrl+- / Ctrl+0 | Zoom in, out, reset |

On macOS, Ctrl is Cmd.

## Not yet

Tabs, bookmarks, history, settings, downloads, ad blocking and a stop button come next.
See the roadmap in the project notes.

## Building

You need Rust (stable, 1.88 or newer) plus a C/C++ toolchain. The first build compiles all of
Servo and takes 30 to 60 minutes. Later builds take a minute or two.
SpiderMonkey is downloaded prebuilt, so you don't need to build it.

### Windows

1. Install [Rust](https://rustup.rs/) with the MSVC toolchain.
2. Install Visual Studio 2022 (or the Build Tools) with the "Desktop development with C++" workload.
3. Install LLVM/clang: `winget install LLVM.LLVM`. Then make sure `clang` is on your `PATH`
   or set `LIBCLANG_PATH=C:\Program Files\LLVM\bin`.
4. Install CMake and NASM: `winget install Kitware.CMake NASM.NASM`.

### Linux (Ubuntu/Debian)

```sh
sudo apt-get install clang cmake nasm libx11-dev libxcb1-dev libxkbcommon-dev \
  libfontconfig1-dev libfreetype-dev libharfbuzz-dev libegl1-mesa-dev \
  libgles2-mesa-dev libudev-dev libwayland-dev
```

### macOS

```sh
xcode-select --install
brew install cmake nasm llvm
```

### Run

```sh
cargo run --release
cargo run --release -- github.com      # open a site
cargo run --release -- rust servo      # search
```

Set `RUST_LOG=info` to see more logging.

### Audio and video (optional)

By default Berick is built without a media backend, so `<video>` and `<audio>` never play.
To turn playback on, install GStreamer and build with the `media` feature:

```sh
cargo run --release --features media
```

- **Windows:** install both the *runtime* and *development* MSVC 64-bit installers (1.24 or newer)
  from <https://gstreamer.freedesktop.org/download/>, choosing the "Complete" install. Then add
  `C:\gstreamer\1.0\msvc_x86_64\bin` to your `PATH` and set
  `PKG_CONFIG_PATH=C:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig`.
- **Linux:** `sudo apt-get install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
  libgstreamer-plugins-bad1.0-dev gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav`
- **macOS:** `brew install gstreamer`

## Sites that need things Servo doesn't have yet

Berick turns on the web features Servo offers, but some sites need parts of the web platform
that Servo hasn't built yet. Those can't be fixed in Berick itself.

- **YouTube, Twitch and most video sites:** these stream video through Media Source Extensions
  (MSE), which Servo doesn't implement. The page may load, but videos won't play, even with the
  `media` feature. Plain video files (an `.mp4` or `.webm` link, or a simple `<video src>`) do play
  with the `media` feature.
- **Netflix, Spotify, Disney+ and other DRM services:** these need Encrypted Media Extensions and a
  licensed DRM module, which Servo doesn't have.
- **Heavy web apps** (Google Docs, Figma and the like) may load partly or look broken where Servo's
  layout or JavaScript APIs are still incomplete.

## Layout

| File | What it does |
| --- | --- |
| `src/main.rs` | Starts the event loop and wakes Servo when it has work |
| `src/browser.rs` | The window, Servo, the page, and input routing |
| `src/gui.rs` | The toolbar, drawn with egui |
| `src/location.rs` | Address bar logic: URL or search (with unit tests) |
| `src/keyutils.rs` | Keyboard event conversion, vendored from Servo's servoshell |

The toolbar and rendering approach follow Servo's own demo browser,
[servoshell](https://github.com/servo/servo/tree/main/ports/servoshell).

## License

[MPL-2.0](LICENSE), the same license as Servo.
