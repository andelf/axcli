# axcli — macOS Accessibility CLI

A Playwright/Puppeteer-style CLI for native macOS apps via the Accessibility API.

Incubated from [picc](https://github.com/andelf/picc).

## Features

- **Explore** — snapshot the accessibility tree of any running app
- **Interact** — click, double-click, input, fill, press keys, hover, focus, scroll
- **Capture** — screenshot windows/elements via ScreenCaptureKit (background, no activation needed)
- **Read** — get element attributes (text, value, title, classes, position, size, …)
- **Wait** — poll for elements or sleep for milliseconds
- **Locators** — CSS-like selectors with role, class, ID, attribute, text matching, chaining, and pseudo-classes

## Install

```sh
cargo install --path .
```

You must grant **Accessibility** permission to the terminal app running `axcli` (System Settings → Privacy & Security → Accessibility).

## Usage

All commands require `--app <name>` or `--pid <pid>` to target an application.

### Snapshot the accessibility tree

```sh
axcli --app Safari snapshot
axcli --app Safari snapshot --depth 5
axcli --app Safari snapshot '.toolbar' --depth 8
```

### Click an element

```sh
axcli --app Safari click 'AXButton[title="Reload"]'
axcli --app Lark click '.SearchButton'
```

### Input text

```sh
axcli --app Safari input '.SearchInput' 'hello world'
axcli --app Safari fill '.SearchInput' 'replace all text'
```

### Press keys

```sh
axcli --app Safari press Enter
axcli --app Safari press 'Command+a'
axcli --app Safari press 'Control+Shift+v'
```

### Scroll

```sh
axcli --app Lark scroll-to '.item'
axcli --app Lark scroll '.chat-list' down 300
```

### Screenshot

```sh
axcli --app Safari screenshot -o /tmp/safari.png
axcli --app Safari screenshot '.toolbar' -o /tmp/toolbar.png
axcli --app Safari screenshot --ocr
```

### Get element attributes

```sh
axcli --app Safari get text '.content'
axcli --app Safari get value '.SearchInput'
axcli --app Safari get classes '.item'
```

### Wait

```sh
axcli --app Safari wait '.loading'     # poll until element appears
axcli --app Safari wait 500            # sleep 500ms
```

### List running apps

```sh
axcli list-apps
```

## Locator Syntax

| Pattern | Meaning | Example |
|---|---|---|
| `#id` | DOM ID | `#root`, `#modal` |
| `.class` | DOM class | `.SearchButton`, `.msg-item` |
| `Role` | AX role | `AXButton`, `button`, `textarea` |
| `Role.class` | Role + class | `AXGroup.feed-card` |
| `Role[attr="val"]` | Exact match | `AXButton[title="Send"]` |
| `Role[attr*="val"]` | Contains | `radiobutton[name*="Tab Title"]` |
| `text="VALUE"` | Exact text | `text="Hello"` |
| `text~="VALUE"` | Contains text | `text~="partial"` |
| `text=/regex/` | Regex text | `text=/\d+条新消息/` |
| `L >> R` | Chain (scope) | `.sidebar >> AXButton` |
| `L > R` | Direct child | `AXWindow > AXGroup` |
| `:has-text("…")` | Subtree text | `.card:has-text("meeting")` |
| `:has(sel)` | Has descendant | `.item:has(.reaction)` |
| `:visible` | Non-zero size | `AXButton:visible` |
| `>> nth=N` | Nth match | `.item >> nth=0` |

## License

MIT OR Apache-2.0
