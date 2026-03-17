# axcli

[![CI](https://github.com/andelf/axcli/actions/workflows/ci.yml/badge.svg)](https://github.com/andelf/axcli/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/axcli)](https://crates.io/crates/axcli)
[![License](https://img.shields.io/crates/l/axcli)](https://github.com/andelf/axcli#license)
![macOS](https://img.shields.io/badge/platform-macOS-lightgrey)

**A Playwright/Puppeteer-style CLI for native macOS apps via the Accessibility API.**

Explore, interact with, and automate any macOS application from the command line — snapshot accessibility trees, click buttons, type text, press keys, scroll, take screenshots, and more. Built on Apple's Accessibility and ScreenCaptureKit frameworks.

Incubated from [picc](https://github.com/andelf/picc).

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

All commands require `--app <name>` or `--pid <pid>` to target an application.

```
$ axcli --help
macOS Accessibility CLI tool — automate any app via the Accessibility API.

Workflow: snapshot (explore) → get text (read) → click/input (act) → screenshot (verify).
Run `axcli <command> --help` for per-command tips.

Usage: axcli [OPTIONS] <COMMAND>

Commands:
  snapshot    Print accessibility tree (shows first match by default, use --all for all)
  click       Click element
  dblclick    Double-click element
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
  list-apps   List running applications visible to accessibility
  help        Print this message or the help of the given subcommand(s)

Options:
      --app <APP>  Application name
      --pid <PID>  Process ID
  -h, --help       Print help (see a summary with '-h')
  -V, --version    Print version
```

### Examples

**Snapshot the accessibility tree:**

```sh
axcli --app Safari snapshot
axcli --app Safari snapshot --depth 5
axcli --app Safari snapshot '.toolbar' --depth 8
```

**Click an element:**

```sh
axcli --app Safari click 'AXButton[title="Reload"]'
axcli --app Lark click '.SearchButton'
```

**Input and fill text:**

```sh
axcli --app Safari input '.SearchInput' 'hello world'
axcli --app Safari fill '.SearchInput' 'replace all text'
```

**Press keys:**

```sh
axcli --app Safari press Enter
axcli --app Safari press 'Command+a'
axcli --app Safari press 'Control+Shift+v'
```

**Scroll:**

```sh
axcli --app Lark scroll-to '.item'
axcli --app Lark scroll '.chat-list' down 300
```

**Screenshot:**

```sh
axcli --app Safari screenshot -o /tmp/safari.png
axcli --app Safari screenshot '.toolbar' -o /tmp/toolbar.png
axcli --app Safari screenshot --ocr
```

**Get element attributes:**

```sh
axcli --app Safari get text '.content'
axcli --app Safari get value '.SearchInput'
axcli --app Safari get classes '.item'
```

**Wait:**

```sh
axcli --app Safari wait '.loading'     # poll until element appears
axcli --app Safari wait 500            # sleep 500ms
```

**List running apps:**

```sh
axcli list-apps
```

## Commands

| Command | Description |
|---|---|
| `snapshot` | Print the accessibility tree of an app or element |
| `click` | Click an element (AXPress action or mouse click) |
| `dblclick` | Double-click an element |
| `input` | Focus element and type text (appends) |
| `fill` | Clear field then type text (Cmd+A, Delete, type) |
| `press` | Press a key combination |
| `hover` | Move mouse to element center |
| `focus` | Focus an element |
| `scroll-to` | Scroll an element into view |
| `scroll` | Scroll within an element (up/down/left/right) |
| `screenshot` | Capture a screenshot via ScreenCaptureKit |
| `activate` | Bring the target application to foreground |
| `wait` | Wait for an element to appear or sleep N milliseconds |
| `get` | Get an element attribute (text, value, title, classes, position, size, ...) |
| `list-apps` | List running applications visible to accessibility |

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
| `text=/regex/` | Regex text | `text=/\d+条新消息/` |
| `L >> R` | Chain (scope) | `.sidebar >> AXButton` |
| `L > R` | Direct child | `AXWindow > AXGroup` |
| `>> nth=N` | Nth match | `.item >> nth=0` |
| `>> first / last` | First/last match | `.item >> last` |
| `:has-text("…")` | Subtree text match | `.card:has-text("meeting")` |
| `:has(sel)` | Has descendant | `.item:has(.reaction)` |
| `:visible` | Non-zero size | `AXButton:visible` |
| `:nth-child(N)` | Nth child (0-based) | `AXGroup:nth-child(0)` |

## License

MIT OR Apache-2.0
