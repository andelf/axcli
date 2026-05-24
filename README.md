# axcli

[![CI](https://github.com/andelf/axcli/actions/workflows/ci.yml/badge.svg)](https://github.com/andelf/axcli/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/axcli)](https://crates.io/crates/axcli)
[![License](https://img.shields.io/crates/l/axcli)](https://github.com/andelf/axcli#license)
![macOS](https://img.shields.io/badge/platform-macOS-lightgrey)

**A Playwright/Puppeteer-style CLI for native macOS apps via the Accessibility API.**

Explore, interact with, and automate any macOS application from the command line — snapshot accessibility trees, click buttons, type text, press keys, scroll, take screenshots, and more. Built on Apple's Accessibility and ScreenCaptureKit frameworks.

Incubated from [picc](https://github.com/andelf/picc).

## Highlights

- **Background-safe input.** `click`, `dblclick`, and `scroll` deliver events through `CGEventPostToPid` by default, so the target app stays in the background — no focus steal, no cursor movement. Confirmed working on AppKit *and* Chromium/Electron apps (Lark, VSCode, Chrome).
- **Playwright-style locators.** CSS-like selectors with chaining (`>>`), pseudo-classes (`:has-text`, `:has`, `:visible`, `:nth-child`), and regex text matchers.
- **Occlusion-proof screenshots.** ScreenCaptureKit captures the target window even when it's behind other windows; optional `--ocr` via Vision framework.
- **Visual cursor overlay.** A small crosshair animates toward each click target for visual feedback; disable with `--no-visual-cursor`.
- **Escape hatches.** Global `mouse` / `keyboard` subcommands for raw HID-level delivery, plus `--strategy` flags on `click`, `scroll`, and `press` to force a specific dispatch path.

## macOS Permissions

axcli uses the macOS Accessibility API and ScreenCaptureKit, which require explicit user consent:

- **Accessibility**: The terminal app (e.g. Terminal.app, iTerm2, Alacritty) running `axcli` must be granted Accessibility access. Go to **System Settings → Privacy & Security → Accessibility** and add your terminal app.
- **Screen Recording**: Required for `screenshot` commands. Go to **System Settings → Privacy & Security → Screen Recording** and add your terminal app.

You may need to restart your terminal after granting permissions.

## Installation

### From crates.io

```sh
cargo install axcli
```

### From source

```sh
git clone https://github.com/andelf/axcli.git
cd axcli
cargo install --path .
```

## Usage

App-scoped commands require `--app <name>` or `--pid <pid>` to target an application. The `mouse`, `keyboard`, and `list-apps` subcommands are global and ignore `--app`/`--pid`.

```
$ axcli --help
macOS Accessibility CLI tool — automate any app via the Accessibility API.

Workflow: snapshot (explore) → get text (read) → click/input (act) → screenshot (verify).
Run `axcli <command> --help` for per-command tips.

Usage: axcli [OPTIONS] <COMMAND>

Commands:
  snapshot    Print accessibility tree (shows first match by default, use --all for all)
  click       Click element (background-safe, no focus steal)
  dblclick    Double-click element (background-safe via cg-pid)
  click-xy    Click at screen coordinates (background-safe, no AX selector needed)
  dblclick-xy Double-click at screen coordinates (background-safe, no AX selector)
  input       Focus element and type text (appends to existing content)
  fill        Clear field then type text (Cmd+A, Delete, type)
  press       Press key combo (Enter, Control+a, Command+Shift+v)
  hover       Move mouse to element center
  focus       Focus element (AXFocused + click fallback)
  scroll-to   Scroll element into view (AXScrollToVisible)
  scroll      Scroll within an element (up/down/left/right)
  screenshot  Capture screenshot (background, no need to activate app)
  activate    Activate (bring to foreground) the target application
  wait        Wait for element or milliseconds
  get         Get element attribute value
  watch       Watch for accessibility notifications (daemon mode)
  list-apps   List running applications visible to accessibility
  mouse       Global mouse control — ignores --app/--pid
  keyboard    Global keyboard input — ignores --app/--pid

Options:
      --app <APP>           Application name
      --pid <PID>           Process ID
      --no-visual-cursor    Disable the software cursor overlay during click/hover
  -h, --help                Print help (see a summary with '-h')
  -V, --version             Print version
```

### Examples

**Snapshot the accessibility tree:**

```sh
axcli --app Safari snapshot
axcli --app Safari snapshot --depth 5
axcli --app Safari snapshot '.toolbar' --depth 8
```

**Click an element (background-safe by default):**

```sh
axcli --app Safari click 'AXButton[title="Reload"]'
axcli --app Lark   click '.SearchButton'

# Explicit strategies:
axcli --app Safari click '.Reload' --strategy ax             # AXPress (no event posting)
axcli --app Safari click '.Reload' --strategy cg --activate  # global click; brings app to front

# Pre-move the cursor to trigger hover-gated UI:
axcli --app Lark click '.menu-item' --hover
```

**Double-click:**

```sh
axcli --app Finder dblclick '.file-cell'
```

**Click at screen coordinates (for self-drawn / non-AX UIs):**

```sh
# Background-safe click at logical screen (X, Y). Useful when the target UI
# isn't exposed via the Accessibility API (custom-rendered controls, canvas-
# based widgets, proprietary financial / trading clients, ...).  No focus
# steal, no cursor movement.
axcli --app 东方财富 click-xy 440 150

# Pin to a specific window if the app has multiple:
axcli --pid 92666 click-xy 440 150 --window 57484

# Foreground equivalent (activates the app and goes via HID event tap):
axcli --app 东方财富 click-xy 440 150 --strategy cg --activate

# Double-click variant:
axcli --app Finder dblclick-xy 200 300
```

**Input and fill text:**

```sh
axcli --app Safari input '.SearchInput' 'hello world'
axcli --app Safari fill  '.SearchInput' 'replace all text'
```

**Press keys:**

```sh
axcli --app Safari press Enter
axcli --app Safari press 'Command+a'

# Deliver to a background app without stealing focus:
axcli --app Calculator press '5' --strategy pid
```

**Scroll (background-safe via cg-pid):**

```sh
axcli --app Lark scroll-to '.item'
axcli --app Lark scroll    '.chat-list' down 300

# Legacy global path; auto-activates if the target window is occluded:
axcli --app Lark scroll '.chat-list' down 300 --strategy cg
```

**Screenshot:**

```sh
axcli --app Safari screenshot -o /tmp/safari.png
axcli --app Safari screenshot '.toolbar' -o /tmp/toolbar.png
axcli --app Safari screenshot --ocr
```

**Get element attributes:**

```sh
axcli --app Safari get text    '.content'
axcli --app Safari get value   '.SearchInput'
axcli --app Safari get classes '.item'
```

**Wait:**

```sh
axcli --app Safari wait '.loading'     # poll until element appears
axcli --app Safari wait 500            # sleep 500ms
```

**Watch for UI changes (daemon):**

```sh
axcli --app Lark watch
axcli --app Lark watch --format json
```

**Global mouse / keyboard (ignores --app/--pid):**

```sh
axcli mouse pos                     # print current cursor position
axcli mouse move 400 300
axcli mouse click 400 300
axcli mouse scroll 0 -120           # scroll down 120px at current cursor
axcli keyboard type 'hello world'
axcli keyboard press 'Command+Shift+4'
```

**List running apps:**

```sh
axcli list-apps
```

## Commands

| Command | Description |
|---|---|
| `snapshot` | Print the accessibility tree of an app or element |
| `click` | Click an element — background-safe via `CGEventPostToPid` by default. Flags: `--strategy auto/ax/cg/cg-pid`, `--hover`, `--activate` |
| `dblclick` | Double-click an element (background-safe via cg-pid) |
| `click-xy` | Click at screen coordinates (X, Y) — for self-drawn / non-AX UIs. Background-safe via cg-pid; `--strategy cg/hid` for global. `--window <ID>` to pin to a specific window |
| `dblclick-xy` | Double-click at screen coordinates (X, Y) — same model as `click-xy` |
| `input` | Focus element and type text (appends) |
| `fill` | Clear field then type text (Cmd+A, Delete, type) |
| `press` | Press a key combination. `--strategy hid` (default, activates) or `pid` (background) |
| `hover` | Move mouse to element center |
| `focus` | Focus an element |
| `scroll-to` | Scroll an element into view (`AXScrollToVisible`) |
| `scroll` | Scroll within an element — background-safe via cg-pid by default. `--strategy auto/cg-pid/cg` |
| `screenshot` | Capture a screenshot via ScreenCaptureKit (occlusion-proof). `--ocr`, `--legacy` |
| `activate` | Bring the target application to foreground |
| `wait` | Wait for an element to appear, or sleep N milliseconds |
| `get` | Get an element attribute (text, value, title, classes, position, size, ...) |
| `watch` | Watch accessibility notifications in the target app. `--format text/json` |
| `list-apps` | List running applications visible to accessibility |
| `mouse` | Global mouse control: `pos`, `move`, `click`, `dblclick`, `scroll` |
| `keyboard` | Global keyboard input: `type`, `press` |

## Background delivery

By default, `click`, `dblclick`, and `scroll` use `CGEventPostToPid` (the `cg-pid` strategy) to deliver events directly to the target process — without activating it and without moving the real cursor:

- **Click** applies the SWaveAX recipe: an `NSEvent` factory event with `CGEventSetWindowLocation` and a Command-flag signal, then posted to the target pid.
- **Scroll** pre-sends a `MouseMoved` event to update the process's "window under cursor" tracking state, then posts the scroll wheel event with the target window tags.

Tested on AppKit (Calculator, TextEdit, Finder) and Chromium/Electron apps (Lark, VSCode, Chrome). If a control only responds to real hover state (menus, tooltips), add `--hover` to pre-move the cursor. If a target exposes no accessible click surface, fall back to `--strategy cg --activate` to send a global click at the element's screen coordinates.

For UIs that aren't exposed to the Accessibility API at all — custom-rendered controls, canvas widgets, proprietary financial / trading clients — use `click-xy X Y` (or `dblclick-xy`) to bypass the AX selector entirely. It applies the same SWaveAX recipe at raw screen coordinates and auto-picks the target window (or use `--window <CGWindowID>` to override). Confirmed working on AppKit apps with custom-drawn buttons that have zero AX elements (verified on 东方财富 self-drawn trade panel buttons).

`press` defaults to the global HID path (which activates the app). Use `press <key> --strategy pid` to deliver to a background app's first responder.

## Locator Syntax

axcli uses a CSS-like selector syntax to target elements in the accessibility tree:

| Pattern | Meaning | Example |
|---|---|---|
| `#id` | DOM ID | `#root`, `#modal` |
| `.class` | DOM class | `.SearchButton`, `.msg-item` |
| `.class1.class2` | Multiple classes | `.message-item.message-self` |
| `Role` | AX role | `AXButton`, `button`, `textarea` |
| `Role.class` | Role + class | `AXGroup.feed-card` |
| `Role[attr="val"]` | Exact match | `AXButton[title="Send"]` |
| `Role[attr*="val"]` | Contains | `radiobutton[name*="Tab Title"]` |
| `Role[attr^="val"]` | Starts with | `AXWindow[title^="Chat"]` |
| `Role[attr$="val"]` | Ends with | `text[desc$="ago"]` |
| `text="VALUE"` | Exact text | `text="Hello"` |
| `text~="VALUE"` | Contains text | `text~="partial"` |
| `text=/regex/` | Regex text | `text=/\d+ unread/`, `text=/Log\s*in/i` |
| `L >> R` | Chain (scope) | `.sidebar >> AXButton` |
| `L > R` | Direct child | `AXWindow > AXGroup` |
| `>> nth=N` | Nth match | `.item >> nth=0` |
| `>> first / last` | First/last match | `.item >> last` |
| `:has-text("…")` | Subtree text match | `.card:has-text("meeting")` |
| `:has(sel)` | Has descendant | `.item:has(.reaction)` |
| `:visible` | Non-zero size | `AXButton:visible` |
| `:nth-child(N)` | Nth child (0-based) | `AXGroup:nth-child(0)` |

Bracket attributes `title` / `desc` / `text` map to `AXTitle` / `AXDescription` / `AXValue`. Regex flags after the trailing `/` follow Rust's `regex` crate (`i`, `m`, `s`, `x`).

## License

MIT OR Apache-2.0
