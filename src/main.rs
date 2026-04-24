//! axcli — macOS Accessibility CLI tool.
//!
//! Usage:
//!   axcli --app Lark snapshot
//!   axcli --app Lark snapshot --depth 5
//!   axcli --app Lark click '.SearchButton'
//!   axcli --app Lark dblclick '.SearchButton'
//!   axcli --app Lark input '.SearchInput' 'hello'
//!   axcli --app Lark fill '.SearchInput' 'hello'
//!   axcli --app Lark press Enter
//!   axcli --app Lark press 'Control+a'
//!   axcli --app Lark hover '.SearchButton'
//!   axcli --app Lark focus '.SearchInput'
//!   axcli --app Lark scroll-to '.item'
//!   axcli --app Lark scroll '.chat-list' down 300
//!   axcli --app Lark screenshot -o /tmp/shot.png
//!   axcli --app Lark screenshot '.SearchButton' -o /tmp/btn.png
//!   axcli --app Lark wait '.loading'
//!   axcli --app Lark wait 500
//!   axcli --app Lark get AXValue '.SearchInput'

use std::ptr::NonNull;
use std::ffi::c_void;

use clap::{Parser, Subcommand, ValueEnum};
use objc2_core_foundation::{CGPoint, CGSize, CGRect, CFString, CFRunLoop, kCFRunLoopDefaultMode};
use objc2_core_graphics::CGImage;
use objc2_application_services::{AXObserver, AXObserverCallback, AXUIElement, AXError as AXErr};
use axcli::accessibility::{self, AXNode, attr_string};
use axcli::actions::ExecutionContext;
use axcli::error::{AxError, exit_code};
use axcli::{input, overlay, screenshot, tree_fmt};

#[derive(Parser)]
#[command(name = "axcli", version, about = "macOS Accessibility CLI tool", long_about = "\
macOS Accessibility CLI tool — automate any app via the Accessibility API.

Workflow: snapshot (explore) → get text (read) → click/input (act) → screenshot (verify).
Run `axcli <command> --help` for per-command tips.", after_help = "\
Locator syntax:
  #id                       DOM ID           e.g. #root, #modal
  .class                    DOM class        e.g. .SearchButton, .msg-item
  .class1.class2            Multiple classes  e.g. .message-item.message-self
  Role                      AX role          e.g. AXButton, button, textarea
  Role.class                Role + class     e.g. AXGroup.feed-card
  Role[attr=\"val\"]          Exact match      e.g. AXButton[title=\"Send\"]
  Role[attr*=\"val\"]         Contains          e.g. radiobutton[name*=\"Tab Title\"]
  Role[attr^=\"val\"]         Starts with       e.g. AXWindow[title^=\"Chat\"]
  Role[attr$=\"val\"]         Ends with         e.g. text[desc$=\"ago\"]
  Bracket attrs: title (AXTitle, alias: name), desc (AXDescription), text (AXValue)
  text=VALUE                Exact text       e.g. text=\"Hello\"
  text~=VALUE               Contains text    e.g. text~=\"partial\"
  text=/regex/flags         Regex text       e.g. text=/\\d+ unread/, text=/Log\\s*in/i
  L >> R                    Chain (scope)    e.g. .sidebar >> AXButton
  L > R                     Direct child     e.g. AXWindow > AXGroup
  L >> nth=N                Pick Nth match   e.g. .item >> nth=0, nth=-1
  L >> first / last         Pick first/last  e.g. .item >> last

Pseudo-classes:
  :has-text(\"text\")         Subtree text     e.g. .card:has-text(\"Meeting\")
  :has(selector)            Has descendant   e.g. .item:has(.reaction)
  :visible                  Non-zero size    e.g. AXButton:visible
  :nth-child(N)             Nth child (0-based) e.g. AXGroup:nth-child(0)
")]
struct Cli {
    /// Application name
    #[arg(long, global = true)]
    app: Option<String>,

    /// Process ID
    #[arg(long, global = true)]
    pid: Option<i32>,

    /// Disable the software cursor overlay during click/hover operations.
    /// Also disabled by env var AXCLI_NO_VISUAL_CURSOR=1.
    #[arg(long, global = true)]
    no_visual_cursor: bool,

    #[command(subcommand)]
    command: Command,
}

/// Known attribute names for `get` command. Also accepts raw AX* attribute names.
#[derive(Clone, Debug)]
enum GetAttr {
    /// Subtree text (with newlines at block boundaries)
    Text,
    /// AXRole
    Role,
    /// AXTitle
    Title,
    /// AXDescription
    Description,
    /// AXValue
    Value,
    /// AXDOMIdentifier
    DomId,
    /// AXDOMClassList
    Classes,
    /// Available actions
    Actions,
    /// Screen position (x, y)
    Position,
    /// Element size (w, h)
    Size,
    /// Number of children
    ChildCount,
    /// Raw AX attribute (e.g. AXHelp, AXURL)
    Raw(String),
}

impl std::fmt::Display for GetAttr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text => write!(f, "text"),
            Self::Role => write!(f, "role"),
            Self::Title => write!(f, "title"),
            Self::Description => write!(f, "description"),
            Self::Value => write!(f, "value"),
            Self::DomId => write!(f, "domid"),
            Self::Classes => write!(f, "classes"),
            Self::Actions => write!(f, "actions"),
            Self::Position => write!(f, "position"),
            Self::Size => write!(f, "size"),
            Self::ChildCount => write!(f, "child-count"),
            Self::Raw(s) => write!(f, "{s}"),
        }
    }
}

impl std::str::FromStr for GetAttr {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "text" => Self::Text,
            "role" => Self::Role,
            "title" | "axtitle" => Self::Title,
            "description" | "desc" | "axdescription" => Self::Description,
            "value" | "axvalue" => Self::Value,
            "domid" | "dom-id" | "axdomidentifier" => Self::DomId,
            "classes" | "class" | "axdomclasslist" => Self::Classes,
            "actions" => Self::Actions,
            "position" | "pos" => Self::Position,
            "size" => Self::Size,
            "children" | "child-count" | "childcount" => Self::ChildCount,
            _ => Self::Raw(s.to_string()),
        })
    }
}

/// Keyboard post strategy.
#[derive(Clone, Debug, ValueEnum)]
enum PressStrategy {
    /// Global `CGEventPost(HIDEventTap)`. Activates the app first. Default.
    Hid,
    /// `CGEventPostToPid` — delivers to the target process's first responder
    /// without activation or focus steal.  Empirically works on AppKit apps
    /// (Calculator, TextEdit).  Recommended for background input.
    Pid,
}

/// Scroll dispatch strategy.
#[derive(Clone, Debug, ValueEnum)]
enum ScrollStrategy {
    /// Default. Currently resolves to `cg-pid`.
    Auto,
    /// `CGEventPost(HIDEventTap)` — global scroll at element center.
    /// Moves the real cursor to the target.  If the target window is
    /// occluded, auto-activates the app to bring it to front first.
    Cg,
    /// `CGEventPostToPid` with MouseMoved pre-send to establish window
    /// tracking state.  Background-safe, no focus steal, no cursor movement.
    /// Confirmed working on Chromium/Electron apps (Lark).
    CgPid,
}

/// Click dispatch strategy.
#[derive(Clone, Debug, ValueEnum)]
enum ClickStrategy {
    /// Default: cg-pid (CGEventPostToPid).
    /// Background-safe: no focus steal, no activation.
    Auto,
    /// AXPress via the Accessibility API.  Background-safe, no focus steal.
    /// Fails if the element doesn't expose AXPress (e.g. custom canvases).
    Ax,
    /// `CGEventPost(HIDEventTap)` at element center.  Universally works for
    /// whatever is on screen but the click goes to the topmost window — pair
    /// with `--activate` to bring the target to the front first.
    Cg,
    /// `CGEventPostToPid` with the SWaveAX recipe (NSEvent factory +
    /// `CGEventSetWindowLocation` + Command-flag signal).  Delivers the click
    /// to the target window without activation or focus steal.  Works on
    /// native AppKit apps and Chromium/Electron apps (VSCode, Chrome, Lark).
    /// Note: some Electron apps (e.g. Lark) may self-activate after receiving
    /// the event — that is app behavior, not a delivery failure.
    CgPid,
}

#[derive(Subcommand)]
enum Command {
    /// Print accessibility tree (shows first match by default, use --all for all)
    ///
    /// Works on any element regardless of viewport position.
    /// Use --depth 4-6 for overview, 10+ for full content extraction.
    /// Use --max-text-len 0 to show full text without truncation.
    /// Tip: if you only need text content, `get text` is faster and lighter.
    Snapshot {
        /// Locator selector to focus on
        locator: Option<String>,
        /// Max tree depth
        #[arg(long, default_value = "10")]
        depth: usize,
        /// Show all matches instead of just the first
        #[arg(long)]
        all: bool,
        /// Max text/title display length (0 = no truncation)
        #[arg(long, default_value = "80")]
        max_text_len: usize,
        /// Simplify output: hide DOM IDs/classes, prune empty subtrees
        #[arg(long)]
        simplify: bool,
    },
    /// Click element (background-safe, no focus steal)
    ///
    /// Default: cg-pid (CGEventPostToPid), background-safe.
    /// Use `--strategy ax` to force AXPress instead.
    /// For off-screen elements, call `scroll-to` first.
    Click {
        locator: String,
        /// Click strategy.  `auto` (default) uses `cg-pid` — background-safe,
        /// no focus steal.  Override with `ax` / `cg` / `cg-pid` to force a
        /// specific path.
        #[arg(long, value_enum, default_value_t = ClickStrategy::Auto)]
        strategy: ClickStrategy,
        /// Move the cursor to the element center before clicking (hover
        /// effect).  By default the cursor stays where the user left it —
        /// fine for plain buttons but may miss controls that depend on
        /// mouseEntered:/`:hover` state (some menus, tooltips, custom views).
        /// Applies to all strategies.
        #[arg(long)]
        hover: bool,
        /// Bring the target app to the foreground before clicking.  Required
        /// for `cg` to reliably hit the target (otherwise the click goes to
        /// whatever window is currently on top at that screen point).
        /// `ax` and `cg-pid` ignore this flag (they don't need activation).
        #[arg(long)]
        activate: bool,
    },
    /// Double-click element (background-safe via cg-pid)
    Dblclick {
        locator: String,
    },
    /// Focus element and type text (appends to existing content)
    Input {
        /// Target element
        locator: String,
        /// Text to type
        text: String,
    },
    /// Clear field then type text (Cmd+A, Delete, type)
    Fill {
        /// Target element
        locator: String,
        /// Text to type
        text: String,
    },
    /// Press key combo (Enter, Control+a, Command+Shift+v)
    ///
    /// `--strategy pid` delivers the key to the target app's first responder
    /// via CGEventPostToPid — no activation, no focus steal.  Confirmed
    /// working on AppKit apps (Calculator, TextEdit).  See
    /// docs/research/background-click.md appendix A.3.
    Press {
        key: String,
        /// Post strategy.  `hid` (default): global `CGEventPost(HIDEventTap)`,
        /// activates the target first.  `pid`: `CGEventPostToPid`, delivers
        /// to the target process in the background without focus steal.
        #[arg(long, value_enum, default_value_t = PressStrategy::Hid)]
        strategy: PressStrategy,
    },
    /// Move mouse to element center
    ///
    /// Useful for triggering hover-only UI (e.g. toolbars, tooltips).
    /// The hover state is lost when the mouse moves away.
    /// For off-screen elements, call `scroll-to` first.
    Hover {
        locator: String,
    },
    /// Focus element (AXFocused + click fallback)
    Focus {
        locator: String,
    },
    /// Scroll element into view (AXScrollToVisible)
    ///
    /// Call before hover/click if the element may be off-screen.
    /// Not needed for snapshot/get — they work regardless of viewport.
    ScrollTo {
        locator: String,
    },
    /// Scroll within an element (up/down/left/right)
    ///
    /// Default: cg-pid (background-safe, no focus steal, no cursor movement).
    /// Use `--strategy cg` for the legacy global path.
    /// After scrolling, lazy-loaded lists may reindex elements.
    /// Use :has-text() instead of nth= to relocate targets.
    Scroll {
        /// Locator of the scrollable element
        locator: String,
        /// Direction: up, down, left, right
        direction: String,
        /// Pixels to scroll (default 300)
        #[arg(default_value = "300")]
        pixels: i32,
        /// Scroll strategy.  `auto` (default) resolves to `cg-pid` —
        /// background-safe, no focus steal, no cursor movement via
        /// CGEventPostToPid.  `cg-pid` forces the same path explicitly.
        /// `cg` uses global mouse_move + scroll_wheel (moves the real
        /// cursor, auto-activates the app if the target window is occluded).
        #[arg(long, value_enum, default_value_t = ScrollStrategy::Auto)]
        strategy: ScrollStrategy,
    },
    /// Capture screenshot (background, no need to activate app)
    ///
    /// Uses ScreenCaptureKit to capture the window in the background — the target
    /// app does NOT need to be in the foreground, and occluded windows are captured
    /// correctly. Falls back to legacy CGWindowListCreateImage if SCK is unavailable.
    /// Saves PNG to file. Use --ocr to also extract text via Vision framework.
    /// Prefer `snapshot` for structured exploration (faster, no file I/O).
    /// Use screenshot when you need visual context or multimodal analysis.
    Screenshot {
        /// Locator selector (optional, for element screenshot)
        locator: Option<String>,
        /// Output file path
        #[arg(short, long)]
        output: Option<String>,
        /// Run OCR on the captured image (Vision framework, zh-Hans + en-US).
        /// Note: OCR results may be inaccurate — consider using a multimodal model
        /// to read the screenshot directly if OCR output is unsatisfactory.
        #[arg(long)]
        ocr: bool,
        /// Force legacy capture: activate app to foreground + CGWindowListCreateImage.
        /// Useful when you need to verify what's actually visible on screen (e.g.
        /// before a click), since SCK captures the window's own content regardless
        /// of occlusion.
        #[arg(long)]
        legacy: bool,
    },
    /// Activate (bring to foreground) the target application
    Activate,
    /// Wait for element or milliseconds
    ///
    /// Pass a number (e.g. 500) to sleep, or a locator to poll until found.
    /// Useful after click/scroll to wait for UI transitions or lazy loading.
    Wait {
        /// Milliseconds (number) or locator string
        target: String,
    },
    /// Get element attribute value
    ///
    /// Lightest way to read element data. Most useful attributes:
    ///   text     — subtree plain text with newlines (most common)
    ///   classes  — CSS class list (for building locators)
    ///   value    — form field value (input/textarea)
    #[command(after_help = "\
Known attributes:
  text         Subtree text (with newlines at block boundaries)
  role         AXRole (e.g. AXButton, AXStaticText)
  title        AXTitle
  desc         AXDescription (alias: description)
  value        AXValue
  domid        AXDOMIdentifier (alias: dom-id)
  classes      AXDOMClassList (alias: class)
  actions      Available AX actions
  position     Screen position as x,y (alias: pos)
  size         Element size as w,h
  child-count  Number of children (alias: children)
  AX*          Any raw AX attribute (e.g. AXHelp, AXURL)
")]
    Get {
        /// Attribute to read
        #[arg(value_name = "ATTR")]
        attr: GetAttr,
        locator: String,
        /// Show all matches instead of just the first
        #[arg(long)]
        all: bool,
        /// Accepted for compatibility with snapshot, but ignored
        #[arg(long, hide = true, default_value_t = 0)]
        max_text_len: usize,
    },
    /// Watch for accessibility notifications (daemon mode)
    ///
    /// Monitors UI changes in the target app: element creation, destruction,
    /// layout changes, value changes, focus changes, and more.
    /// Useful for detecting when element refs become stale.
    /// Runs until interrupted (Ctrl+C).
    Watch {
        /// Output format: text (default) or json
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// List running applications visible to accessibility
    ListApps,
    /// Global mouse control — ignores --app/--pid.
    ///
    /// Events are posted via `CGEventPost(HIDEventTap)`, so they land on
    /// whichever window is topmost at the given screen coordinates.  Use
    /// `click <LOCATOR>` instead for targeted delivery to a specific app
    /// (defaults to background-safe cg-pid).
    ///
    /// Note: posting a mouse event at (X, Y) also moves the visible cursor
    /// to (X, Y) — that's a macOS-level behavior, not an axcli choice.
    Mouse {
        #[command(subcommand)]
        action: MouseAction,
    },
    /// Global keyboard input — ignores --app/--pid.
    ///
    /// Events are posted via `CGEventPost(HIDEventTap)` and delivered to
    /// whichever process currently holds keyboard focus (first responder).
    /// To target a specific background app use `press <KEY> --strategy pid`.
    Keyboard {
        #[command(subcommand)]
        action: KeyboardAction,
    },
    /// Debug: print current mouse position and screen info
    #[command(hide = true, name = "debug")]
    Debug {
        /// Debug subcommand (mouse)
        what: String,
    },
}

/// `axcli mouse ...` actions.  X/Y are screen coordinates in CG space
/// (origin top-left, y increases downward).  Negative coordinates are
/// common on multi-display setups (secondary monitor to the left/above).
#[derive(Subcommand)]
enum MouseAction {
    /// Print the current cursor position as `x,y`.
    Pos,
    /// Warp the cursor to (X, Y).
    Move {
        #[arg(allow_hyphen_values = true)] x: f64,
        #[arg(allow_hyphen_values = true)] y: f64,
    },
    /// Left click.  Omit X Y to click at the current cursor position.
    Click {
        #[arg(allow_hyphen_values = true)] x: Option<f64>,
        #[arg(allow_hyphen_values = true)] y: Option<f64>,
    },
    /// Left double-click.  Omit X Y to click at the current cursor position.
    Dblclick {
        #[arg(allow_hyphen_values = true)] x: Option<f64>,
        #[arg(allow_hyphen_values = true)] y: Option<f64>,
    },
    /// Scroll by DX/DY pixels.  DY > 0 scrolls up, DX > 0 scrolls left
    /// (same sign convention as `scroll` subcommand's delta).  Omit X Y to
    /// scroll at the current cursor position (most natural behavior — the
    /// window under the cursor is what receives the scroll).
    Scroll {
        #[arg(allow_hyphen_values = true)] dx: i32,
        #[arg(allow_hyphen_values = true)] dy: i32,
        #[arg(allow_hyphen_values = true)] x: Option<f64>,
        #[arg(allow_hyphen_values = true)] y: Option<f64>,
    },
}

/// `axcli keyboard ...` actions.
#[derive(Subcommand)]
enum KeyboardAction {
    /// Type literal text (Unicode via CGEventKeyboardSetUnicodeString) to
    /// whatever app currently has keyboard focus.
    Type { text: String },
    /// Press a key or key combo.  Examples: `Enter`, `Command+a`,
    /// `Control+Shift+v`, `F5`, `Escape`.  Sent to the current first
    /// responder.
    Press { key: String },
}

fn main() {
    let cli = Cli::parse();

    if cli.no_visual_cursor {
        unsafe { std::env::set_var("AXCLI_NO_VISUAL_CURSOR", "1") };
    }

    // list-apps doesn't need --app/--pid
    if matches!(cli.command, Command::ListApps) {
        if let Err(e) = cmd_list_apps() {
            eprintln!("error: {e}");
            std::process::exit(exit_code(&e));
        }
        return;
    }

    // debug doesn't need --app/--pid
    if let Command::Debug { ref what } = cli.command {
        if let Err(e) = cmd_debug(what) {
            eprintln!("error: {e}");
            std::process::exit(exit_code(&e));
        }
        return;
    }

    // mouse / keyboard are global — ignore --app/--pid.
    if let Command::Mouse { ref action } = cli.command {
        if let Err(e) = cmd_mouse(action) {
            eprintln!("error: {e}");
            std::process::exit(exit_code(&e));
        }
        return;
    }
    if let Command::Keyboard { ref action } = cli.command {
        if let Err(e) = cmd_keyboard(action) {
            eprintln!("error: {e}");
            std::process::exit(exit_code(&e));
        }
        return;
    }

    if let Err(e) = run(cli) {
        eprintln!("error: {e}");
        std::process::exit(exit_code(&e));
    }
}

fn run(cli: Cli) -> Result<(), AxError> {
    let (pid, app) = resolve_app(&cli)?;

    if !accessibility::is_trusted() {
        return Err(AxError::AccessDenied);
    }

    let ctx = ExecutionContext::new(pid, app);

    match cli.command {
        Command::ListApps | Command::Mouse { .. } | Command::Keyboard { .. } => unreachable!(),
        Command::Snapshot { locator, depth, all, max_text_len, simplify } => {
            cmd_snapshot(&ctx, locator.as_deref(), depth, all, max_text_len, simplify)
        }
        Command::Click { locator, strategy, hover, activate } => {
            cmd_click(&ctx, &locator, &strategy, hover, activate)
        }
        Command::Dblclick { locator } => cmd_dblclick(&ctx, &locator),
        Command::Input { locator, text } => cmd_input(&ctx, &locator, &text),
        Command::Fill { locator, text } => cmd_fill(&ctx, &locator, &text),
        Command::Press { key, strategy } => cmd_press(&ctx, &key, &strategy),
        Command::Hover { locator } => cmd_hover(&ctx, &locator),
        Command::Focus { locator } => cmd_focus(&ctx, &locator),
        Command::ScrollTo { locator } => cmd_scroll_to(&ctx, &locator),
        Command::Scroll { locator, direction, pixels, strategy } => {
            cmd_scroll(&ctx, &locator, &direction, pixels, &strategy)
        }
        Command::Screenshot { locator, output, ocr, legacy } => {
            cmd_screenshot(&ctx, locator.as_deref(), output.as_deref(), ocr, legacy)
        }
        Command::Activate => {
            ctx.activate();
            Ok(())
        }
        Command::Wait { target } => cmd_wait(&ctx, &target),
        Command::Get { attr, locator, all, max_text_len: _ } => cmd_get(&ctx, &attr, &locator, all),
        Command::Watch { format } => cmd_watch(pid, &ctx.app, &format),
        Command::Debug { .. } => unreachable!(),
    }
}

// --- App resolution ---

fn resolve_app(cli: &Cli) -> Result<(i32, AXNode), AxError> {
    if let Some(pid) = cli.pid {
        return Ok((pid, AXNode::app(pid)));
    }
    if let Some(ref name) = cli.app {
        let mtm = objc2::MainThreadMarker::new()
            .ok_or_else(|| AxError::InvalidArgument("must run on main thread".to_string()))?;
        match accessibility::find_app_by_name(mtm, name) {
            Some((pid, localized)) => {
                eprintln!("Found app: {localized} (pid={pid})");
                return Ok((pid, AXNode::app(pid)));
            }
            None => return Err(AxError::AppNotFound(name.clone())),
        }
    }
    Err(AxError::InvalidArgument("--app or --pid is required".to_string()))
}

/// Strip known pseudo-class suffixes from a segment for validation.
fn strip_pseudo_classes(s: &str) -> &str {
    let mut base = s;
    loop {
        if let Some(stripped) = base.strip_suffix(":visible") {
            base = stripped;
            continue;
        }
        if let Some(pos) = base.rfind(":nth-child(") {
            if base.ends_with(')') {
                base = &base[..pos];
                continue;
            }
        }
        if let Some(pos) = base.rfind(":has-text(") {
            if base.ends_with(')') {
                base = &base[..pos];
                continue;
            }
        }
        if let Some(pos) = base.rfind(":has(") {
            if base.ends_with(')') {
                base = &base[..pos];
                continue;
            }
        }
        break;
    }
    base
}

/// Validate a single locator segment (between `>>` or `>`).
fn validate_segment(seg: &str) -> Result<(), String> {
    let s = seg.trim();
    if s.is_empty() {
        return Err("empty segment (double `>>` or trailing `>>`)".into());
    }
    let base = strip_pseudo_classes(s);
    if base.is_empty() {
        return Ok(());
    }
    let s = base;
    if s == "first" || s == "last" || s.starts_with("nth=") {
        return Ok(());
    }
    if s.starts_with('#') {
        return if s.len() > 1 { Ok(()) } else { Err("empty DOM ID after `#`".into()) };
    }
    if s.starts_with("text=") || s.starts_with("text~=") {
        return Ok(());
    }
    if s.contains('[') {
        if !s.ends_with(']') {
            return Err(format!("unclosed bracket in `{s}`"));
        }
        let inner = &s[s.find('[').unwrap() + 1..s.len() - 1];
        if !inner.contains('=') {
            return Err(format!("bracket selector missing `=` in `{s}`"));
        }
        return Ok(());
    }
    if s.contains('#') {
        // role#id — role part is optional, id must be non-empty
        let (_, id) = s.split_once('#').unwrap();
        return if !id.is_empty() { Ok(()) } else { Err(format!("empty DOM ID after `#` in `{s}`")) };
    }
    if s.contains('.') {
        let without_not = s.split(":not(").next().unwrap_or(s);
        let has_class = without_not.split('.').skip(1).any(|c| !c.is_empty());
        return if has_class { Ok(()) } else { Err(format!("empty class name in `{s}`")) };
    }
    if s.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Ok(());
    }
    Err(format!("unrecognized locator syntax: `{s}`"))
}

/// Validate an entire locator string.
fn validate_locator(locator: &str) -> Result<(), AxError> {
    for desc_part in locator.split(" >> ") {
        for seg in desc_part.split(" > ") {
            if let Err(msg) = validate_segment(seg) {
                return Err(AxError::LocatorInvalid(format!("{msg}\n  locator: {locator}")));
            }
        }
    }
    Ok(())
}

/// Resolve a locator to a single node.
fn resolve_one(ctx: &ExecutionContext, locator: &str) -> Result<AXNode, AxError> {
    validate_locator(locator)?;
    let node = ctx.resolve_one(locator)?;
    eprintln!(
        "Resolved → role=\"{}\" title=\"{}\"",
        node.role().unwrap_or_default(),
        node.title().unwrap_or_default(),
    );
    Ok(node)
}

fn is_menu_role(role: &str) -> bool {
    role == "AXMenuItem" || role == "AXMenuBarItem"
}

/// Try AXFocused, fall back to clicking element center.
fn ensure_focused(ctx: &ExecutionContext, node: &AXNode) -> Result<(), AxError> {
    if !node.set_focused(true) {
        let (cx, cy) = ctx.element_center(node, false)?;
        eprintln!("AXFocused failed, clicking to focus...");
        input::mouse_move(cx, cy);
        std::thread::sleep(std::time::Duration::from_millis(50));
        input::mouse_click(cx, cy);
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    Ok(())
}

// --- Commands ---

fn cmd_list_apps() -> Result<(), AxError> {
    use objc2_app_kit::NSRunningApplication;

    let mtm = objc2::MainThreadMarker::new()
        .ok_or_else(|| AxError::InvalidArgument("must run on main thread".to_string()))?;
    let _ = mtm;
    let workspace_cls = objc2::runtime::AnyClass::get(c"NSWorkspace")
        .ok_or_else(|| AxError::InvalidArgument("NSWorkspace class not found".to_string()))?;
    let workspace: objc2::rc::Retained<objc2::runtime::NSObject> =
        unsafe { objc2::msg_send![workspace_cls, sharedWorkspace] };
    let apps: objc2::rc::Retained<objc2_foundation::NSArray<NSRunningApplication>> =
        unsafe { objc2::msg_send![&workspace, runningApplications] };

    let mut entries: Vec<(i32, String, String)> = Vec::new();
    for app in apps.iter() {
        let pid = app.processIdentifier();
        let bundle = app
            .bundleIdentifier()
            .map(|b| b.to_string())
            .unwrap_or_default();
        let name = app
            .localizedName()
            .map(|n| n.to_string())
            .unwrap_or_default();
        if !bundle.is_empty() && !name.is_empty() {
            entries.push((pid, name, bundle));
        }
    }
    entries.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    for (pid, name, bundle) in &entries {
        println!("{pid:>6}  {name:<30} {bundle}");
    }
    eprintln!("\n({} apps)", entries.len());
    Ok(())
}

fn cmd_debug(what: &str) -> Result<(), AxError> {
    match what {
        "mouse" => {
            let (mx, my) = input::get_mouse_position();
            println!("Mouse position: ({mx:.1}, {my:.1})");

            // Show all screens and which one the cursor is on
            use objc2_app_kit::NSScreen;
            let mtm = objc2::MainThreadMarker::new()
                .ok_or_else(|| AxError::InvalidArgument("must run on main thread".to_string()))?;
            let _ = mtm;
            // NSScreen uses Cocoa coordinates (origin bottom-left, y up).
            // CGEvent uses CG coordinates (origin top-left, y down).
            // Convert: cg_y = primary_height - ns_y - ns_height
            let screens = NSScreen::screens(mtm);
            let primary_h = screens.iter().next()
                .map(|s| NSScreen::frame(&s).size.height)
                .unwrap_or(0.0);
            for (i, screen) in screens.iter().enumerate() {
                let ns_frame = NSScreen::frame(&screen);
                let name = screen.localizedName();
                // Convert to CG coordinates
                let cg_x = ns_frame.origin.x;
                let cg_y = primary_h - ns_frame.origin.y - ns_frame.size.height;
                let w = ns_frame.size.width;
                let h = ns_frame.size.height;
                let on_screen = mx >= cg_x
                    && mx < cg_x + w
                    && my >= cg_y
                    && my < cg_y + h;
                println!(
                    "Screen {i}: \"{name}\" origin=({cg_x:.0},{cg_y:.0}) size={w:.0}x{h:.0}{mark}",
                    mark = if on_screen { " ← cursor here" } else { "" },
                );
            }
            Ok(())
        }
        _ => Err(AxError::InvalidArgument(format!("unknown debug command: {what} (available: mouse)"))),
    }
}

/// Resolve optional (X, Y) — default to current cursor position when either
/// is unspecified.  Used by `mouse click` / `dblclick` / `scroll`.
fn mouse_point_or_cursor(x: Option<f64>, y: Option<f64>) -> (f64, f64) {
    match (x, y) {
        (Some(x), Some(y)) => (x, y),
        _ => input::get_mouse_position(),
    }
}

fn cmd_mouse(action: &MouseAction) -> Result<(), AxError> {
    // CGEventPost requires Accessibility permission, same as other event-
    // posting paths in axcli.
    if !accessibility::is_trusted() {
        return Err(AxError::AccessDenied);
    }
    match action {
        MouseAction::Pos => {
            let (x, y) = input::get_mouse_position();
            println!("{x:.0},{y:.0}");
        }
        MouseAction::Move { x, y } => {
            input::mouse_move(*x, *y);
        }
        MouseAction::Click { x, y } => {
            let (cx, cy) = mouse_point_or_cursor(*x, *y);
            input::mouse_click(cx, cy);
        }
        MouseAction::Dblclick { x, y } => {
            let (cx, cy) = mouse_point_or_cursor(*x, *y);
            input::mouse_dblclick(cx, cy);
        }
        MouseAction::Scroll { dx, dy, x, y } => {
            let (cx, cy) = mouse_point_or_cursor(*x, *y);
            input::scroll_wheel(cx, cy, *dx, *dy);
        }
    }
    Ok(())
}

fn cmd_keyboard(action: &KeyboardAction) -> Result<(), AxError> {
    if !accessibility::is_trusted() {
        return Err(AxError::AccessDenied);
    }
    match action {
        KeyboardAction::Type { text } => {
            eprintln!("Typing: {text:?}");
            input::type_text(text);
        }
        KeyboardAction::Press { key } => {
            let (keycode, flags) = input::parse_key_combo(key);
            eprintln!("Pressing: {key} (keycode={keycode}, flags=0x{flags:x})");
            input::press_key_combo(keycode, flags);
        }
    }
    Ok(())
}

fn cmd_snapshot(ctx: &ExecutionContext, locator: Option<&str>, depth: usize, all: bool, max_text_len: usize, simplify: bool) -> Result<(), AxError> {
    let mut printer = tree_fmt::TreePrinter::new();
    printer.max_text_len = max_text_len;
    printer.simplify = simplify;

    if let Some(loc) = locator {
        validate_locator(loc)?;
        let nodes = ctx.app.locate_all(loc);
        if nodes.is_empty() {
            return Err(AxError::LocatorNotFound(loc.to_string()));
        }
        if all {
            eprintln!("Found {} matches for {loc}", nodes.len());
            for (i, node) in nodes.iter().enumerate() {
                if i > 0 { println!(); }
                eprintln!("--- match {}/{} ---", i + 1, nodes.len());
                printer.print_with_ancestors(node, depth);
            }
        } else {
            let node = &nodes[0];
            if nodes.len() > 1 {
                eprintln!(
                    "Matched {} elements, showing first. Use --all to see all.",
                    nodes.len()
                );
            }
            eprintln!(
                "Resolved → role=\"{}\" title=\"{}\" children={}",
                node.role().unwrap_or_default(),
                node.title().unwrap_or_default(),
                node.child_count()
            );
            printer.print_with_ancestors(node, depth);
        }
    } else {
        printer.print_tree(&ctx.app, 0, depth);
    }

    eprintln!("\n({} interactive elements)", printer.interactive_count());
    Ok(())
}

fn cmd_click(
    ctx: &ExecutionContext,
    locator: &str,
    strategy: &ClickStrategy,
    hover: bool,
    activate: bool,
) -> Result<(), AxError> {
    let node = resolve_one(ctx, locator)?;
    let role = node.role().unwrap_or_default();

    // Software cursor overlay: animate to target before clicking.
    if overlay::is_enabled() {
        if let Ok((cx, cy)) = ctx.element_center(&node, false) {
            overlay::animate_to_and_click(cx, cy);
        }
    }

    // --hover applies to all strategies: pre-move the cursor for visual
    // hover state.  Useful for controls that gate on mouseEntered:/`:hover`.
    if hover {
        let (cx, cy) = ctx.element_center(&node, false)?;
        input::mouse_move(cx, cy);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Auto: pick the best path based on element + app type.
    let resolved_strategy = match strategy {
        ClickStrategy::Auto => choose_auto_strategy(ctx, &node),
        s => s.clone(),
    };

    match resolved_strategy {
        ClickStrategy::Auto => unreachable!("auto resolved above"),
        ClickStrategy::Ax => {
            if activate {
                eprintln!("warning: --activate is ignored for --strategy ax (target self-decides)");
            }
            eprintln!("Performing AXPress on {role}");
            if !accessibility::perform_action(&node.0, "AXPress") {
                return Err(AxError::ActionFailed("AXPress".to_string()));
            }
            Ok(())
        }
        ClickStrategy::CgPid => {
            if activate {
                eprintln!("warning: --activate is ignored for --strategy cg-pid (background by design)");
            }
            let wid = match node.window_id() {
                Some(w) => w,
                None => {
                    eprintln!("debug: _AXUIElementGetWindow returned no CGWindowID for this element");
                    return Err(AxError::ActionFailed(
                        "could not find the window that owns this element".to_string(),
                    ));
                }
            };
            let (cx, cy) = ctx.element_center(&node, false)?;
            let (wx, wy) = find_owning_window(&node)
                .and_then(|w| w.position())
                .unwrap_or((0.0, 0.0));
            let screen = CGPoint::new(cx, cy);
            let local = CGPoint::new(cx - wx, cy - wy);
            eprintln!(
                "cg-pid click pid={} wid={} screen=({cx:.0},{cy:.0}) local=({:.0},{:.0})",
                ctx.pid, wid, local.x, local.y,
            );
            input::mouse_click_bg(ctx.pid, wid, screen, local);
            Ok(())
        }
        ClickStrategy::Cg => {
            let should_activate = activate;

            // Menu extras (status menu icons) must not be activated — doing so
            // steals focus and dismisses the menu.  Always coord-click.
            if node.subrole().as_deref() == Some("AXMenuExtra") {
                let (cx, cy) = ctx.element_center(&node, false)?;
                eprintln!("Clicking at ({cx:.0}, {cy:.0}) [AXMenuExtra]");
                input::mouse_click(cx, cy);
                return Ok(());
            }

            if should_activate {
                ctx.activate();
            }

            if is_menu_role(&role) {
                eprintln!("Performing AXPress on {role}");
                if !accessibility::perform_action(&node.0, "AXPress") {
                    return Err(AxError::ActionFailed("AXPress".to_string()));
                }
            } else {
                let (cx, cy) = ctx.element_center(&node, false)?;
                eprintln!(
                    "Clicking at ({cx:.0}, {cy:.0}){}{}",
                    if !should_activate { " [no-activate]" } else { "" },
                    if hover { " [hover]" } else { "" },
                );
                input::mouse_click(cx, cy);
            }
            Ok(())
        }
    }
}

/// Decide which click path to use when --strategy auto.
///
/// Always cg-pid — background-safe, no focus steal, works on both
/// native AppKit and Chromium/Electron apps.
fn choose_auto_strategy(_ctx: &ExecutionContext, _node: &AXNode) -> ClickStrategy {
    eprintln!("auto → cg-pid (background-safe)");
    ClickStrategy::CgPid
}

fn cmd_dblclick(ctx: &ExecutionContext, locator: &str) -> Result<(), AxError> {
    let node = resolve_one(ctx, locator)?;

    if overlay::is_enabled() {
        if let Ok((cx, cy)) = ctx.element_center(&node, false) {
            overlay::animate_to_and_click(cx, cy);
        }
    }

    let wid = match node.window_id() {
        Some(w) => w,
        None => {
            return Err(AxError::ActionFailed(
                "could not find the window that owns this element".to_string(),
            ));
        }
    };
    let (cx, cy) = ctx.element_center(&node, false)?;
    let (wx, wy) = find_owning_window(&node)
        .and_then(|w| w.position())
        .unwrap_or((0.0, 0.0));
    let screen = CGPoint::new(cx, cy);
    let local = CGPoint::new(cx - wx, cy - wy);
    eprintln!(
        "cg-pid dblclick pid={} wid={} screen=({cx:.0},{cy:.0}) local=({:.0},{:.0})",
        ctx.pid, wid, local.x, local.y,
    );
    input::mouse_dblclick_bg(ctx.pid, wid, screen, local);
    Ok(())
}

fn cmd_input(ctx: &ExecutionContext, locator: &str, text: &str) -> Result<(), AxError> {
    let node = resolve_one(ctx, locator)?;

    ctx.activate();
    ensure_focused(ctx, &node)?;

    eprintln!("Typing: {text:?}");
    input::type_text(text);
    Ok(())
}

fn cmd_fill(ctx: &ExecutionContext, locator: &str, text: &str) -> Result<(), AxError> {
    let node = resolve_one(ctx, locator)?;

    ctx.activate();
    ensure_focused(ctx, &node)?;

    // Select all + delete
    let (kc_a, fl_a) = input::parse_key_combo("Command+a");
    input::press_key_combo(kc_a, fl_a);
    std::thread::sleep(std::time::Duration::from_millis(50));
    let (kc_del, fl_del) = input::parse_key_combo("Delete");
    input::press_key_combo(kc_del, fl_del);
    std::thread::sleep(std::time::Duration::from_millis(100));

    eprintln!("Filling: {text:?}");
    input::type_text(text);
    Ok(())
}

fn cmd_press(ctx: &ExecutionContext, key: &str, strategy: &PressStrategy) -> Result<(), AxError> {
    let (keycode, flags) = input::parse_key_combo(key);
    match strategy {
        PressStrategy::Hid => {
            ctx.activate();
            eprintln!("Pressing: {key} (keycode={keycode}, flags=0x{flags:x}) via HID");
            input::press_key_combo(keycode, flags);
        }
        PressStrategy::Pid => {
            eprintln!("Pressing: {key} (keycode={keycode}, flags=0x{flags:x}) via post_to_pid pid={}", ctx.pid);
            input::press_key_combo_bg(ctx.pid, keycode, flags);
        }
    }
    Ok(())
}

fn cmd_hover(ctx: &ExecutionContext, locator: &str) -> Result<(), AxError> {
    let node = resolve_one(ctx, locator)?;

    let (cx, cy) = ctx.element_center(&node, false)?;

    if overlay::is_enabled() {
        overlay::animate_to(cx, cy);
    }

    eprintln!("Moving mouse to ({cx:.0}, {cy:.0})");
    input::mouse_move(cx, cy);
    Ok(())
}

fn cmd_focus(ctx: &ExecutionContext, locator: &str) -> Result<(), AxError> {
    let node = resolve_one(ctx, locator)?;

    ctx.activate();
    ensure_focused(ctx, &node)?;
    eprintln!("Focused");
    Ok(())
}

fn cmd_scroll_to(ctx: &ExecutionContext, locator: &str) -> Result<(), AxError> {
    let node = resolve_one(ctx, locator)?;

    ctx.activate();

    eprintln!("Scrolling element into view...");
    if !accessibility::perform_action(&node.0, "AXScrollToVisible") {
        eprintln!("warning: AXScrollToVisible failed (element may not be in a scroll area)");
    }
    Ok(())
}

fn cmd_scroll(ctx: &ExecutionContext, locator: &str, direction: &str, pixels: i32, strategy: &ScrollStrategy) -> Result<(), AxError> {
    let node = resolve_one(ctx, locator)?;

    let (cx, cy) = ctx.element_center(&node, false)?;

    let (dx, dy) = match direction {
        "up" => (0, pixels),
        "down" => (0, -pixels),
        "left" => (pixels, 0),
        "right" => (-pixels, 0),
        _ => {
            return Err(AxError::InvalidArgument(
                format!("invalid direction '{direction}', use up/down/left/right"),
            ));
        }
    };

    let resolved = match strategy {
        ScrollStrategy::Auto => {
            eprintln!("auto → cg-pid (background-safe)");
            ScrollStrategy::CgPid
        }
        s => s.clone(),
    };

    match resolved {
        ScrollStrategy::Auto => unreachable!(),
        ScrollStrategy::CgPid => {
            let wid = match node.window_id() {
                Some(w) => w,
                None => {
                    eprintln!("warning: no window ID, falling back to global scroll");
                    return scroll_global(ctx, cx, cy, dx, dy, direction, pixels);
                }
            };
            let (wx, wy) = find_owning_window(&node)
                .and_then(|w| w.position())
                .unwrap_or((0.0, 0.0));
            let screen = CGPoint::new(cx, cy);
            let local = CGPoint::new(cx - wx, cy - wy);
            eprintln!(
                "cg-pid scroll {direction} {pixels}px pid={} wid={wid} screen=({cx:.0},{cy:.0})",
                ctx.pid,
            );
            input::scroll_wheel_bg(ctx.pid, wid, screen, local, dx, dy);
            Ok(())
        }
        ScrollStrategy::Cg => {
            if let Some(wid) = node.window_id() {
                match accessibility::is_window_visible_at(wid, cx, cy) {
                    Ok(true) => {}
                    Ok(false) => {
                        eprintln!("warning: target occluded, activating app to bring window to front");
                        ctx.activate();
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                    Err(_) => {}
                }
            }
            scroll_global(ctx, cx, cy, dx, dy, direction, pixels)
        }
    }
}

fn scroll_global(_ctx: &ExecutionContext, cx: f64, cy: f64, dx: i32, dy: i32, direction: &str, pixels: i32) -> Result<(), AxError> {
    eprintln!("Scrolling {direction} {pixels}px at ({cx:.0}, {cy:.0}) [global]");
    input::mouse_move(cx, cy);
    std::thread::sleep(std::time::Duration::from_millis(50));
    input::scroll_wheel(cx, cy, dx, dy);
    Ok(())
}

/// Walk up the AX tree from a node to find its owning AXWindow.
fn find_owning_window(node: &AXNode) -> Option<AXNode> {
    let mut current = AXNode::new(node.0.clone());
    for _ in 0..50 {
        if current.role().as_deref() == Some("AXWindow") {
            return Some(current);
        }
        current = current.parent()?;
    }
    None
}

/// Find the first visible AXWindow of the app.
fn find_first_ax_window(ctx: &ExecutionContext) -> Option<AXNode> {
    ctx.app.children().into_iter().find(|w| {
        w.role().as_deref() == Some("AXWindow")
            && w.size().map_or(false, |(w, h)| w > 0.0 && h > 0.0)
    })
}

fn run_ocr(image: &CGImage) -> Result<(), AxError> {
    use objc2_foundation::{NSArray, NSString};
    use axcli::vision;

    let text_req = vision::VNRecognizeTextRequest::new();
    let zh = NSString::from_str("zh-Hans");
    let en = NSString::from_str("en-US");
    let lang = NSArray::from_slice(&[&*zh, &*en]);
    text_req.setRecognitionLanguages(&lang);

    let text_req_ref: &vision::VNRequest =
        unsafe { &*((&*text_req) as *const _ as *const vision::VNRequest) };
    let reqs = NSArray::from_slice(&[text_req_ref]);
    let handler = vision::new_handler_with_cgimage(image);
    vision::perform_requests(&handler, &reqs)
        .map_err(|e| AxError::ScreenshotFailed(format!("OCR failed: {e}")))?;

    if let Some(results) = text_req.results() {
        for item in results.iter() {
            let candidates = item.topCandidates(1);
            for candidate in candidates.iter() {
                println!("{}", candidate.string());
            }
        }
    }
    Ok(())
}

fn cmd_screenshot(ctx: &ExecutionContext, locator: Option<&str>, output: Option<&str>, ocr: bool, legacy: bool) -> Result<(), AxError> {
    screenshot::ensure_cg_init();

    let path = output
        .map(String::from)
        .unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("/tmp/ax_screenshot_{ts}.png")
        });

    // Legacy path: activate app to foreground + CGWindowListCreateImage
    if legacy {
        ctx.activate();
        std::thread::sleep(std::time::Duration::from_millis(100));

        let rect = if let Some(loc) = locator {
            validate_locator(loc)?;
            let node = ctx.resolve_one(loc)?;
            let (x, y) = node.position().unwrap_or((0.0, 0.0));
            let (w, h) = node.size().unwrap_or((0.0, 0.0));
            eprintln!("Capturing {w:.0}x{h:.0} at ({x:.0},{y:.0}) → {path}");
            CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
        } else {
            let windows = ctx.app.children();
            let win = windows.iter().find(|w| {
                w.role().as_deref() == Some("AXWindow")
                    && w.size().map_or(false, |(w, h)| w > 0.0 && h > 0.0)
            });
            if let Some(win) = win {
                let (x, y) = win.position().unwrap_or((0.0, 0.0));
                let (w, h) = win.size().unwrap_or((0.0, 0.0));
                eprintln!("Capturing window {w:.0}x{h:.0} at ({x:.0},{y:.0}) → {path}");
                CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
            } else {
                eprintln!("warning: no visible window found, capturing full screen → {path}");
                CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0))
            }
        };
        let image = screenshot::capture(rect)
            .ok_or_else(|| AxError::ScreenshotFailed("capture failed".to_string()))?;
        if !screenshot::save_png(&image, &path) {
            return Err(AxError::ScreenshotFailed(format!("failed to save {path}")));
        }
        eprintln!("Saved: {path}");
        if ocr {
            run_ocr(&image)?;
        }
        return Ok(());
    }

    // Default: ScreenCaptureKit (no need to activate/foreground the app)
    let image = if let Some(loc) = locator {
        validate_locator(loc)?;
        let node = ctx.resolve_one(loc)?;
        let (el_x, el_y) = node.position().unwrap_or((0.0, 0.0));
        let (el_w, el_h) = node.size().unwrap_or((0.0, 0.0));

        // Walk up the AX tree to find the owning AXWindow
        let ax_win = find_owning_window(&node);
        let (win_x, win_y, win_w, win_h) = ax_win
            .as_ref()
            .map(|aw| {
                let (x, y) = aw.position().unwrap_or((0.0, 0.0));
                let (w, h) = aw.size().unwrap_or((0.0, 0.0));
                (x, y, w, h)
            })
            .unwrap_or((0.0, 0.0, 0.0, 0.0));

        // Try SCK: capture the specific window that contains this element
        let win_image = if win_w > 0.0 && win_h > 0.0 {
            axcli::screen_capture::capture_window_by_frame(ctx.pid, win_x, win_y, win_w, win_h)
        } else {
            axcli::screen_capture::capture_window_by_pid(ctx.pid)
        };

        if let Some(win_image) = win_image {
            // Compute element position relative to window, in pixels
            let img_w = CGImage::width(Some(&win_image));
            let scale = if win_w > 0.0 { img_w as f64 / win_w } else { 1.0 };
            let crop_rect = CGRect::new(
                CGPoint::new((el_x - win_x) * scale, (el_y - win_y) * scale),
                CGSize::new(el_w * scale, el_h * scale),
            );
            eprintln!("Capturing element {el_w:.0}x{el_h:.0} at ({el_x:.0},{el_y:.0}) → {path}");
            CGImage::with_image_in_rect(Some(&win_image), crop_rect)
                .ok_or_else(|| AxError::ScreenshotFailed("crop failed".to_string()))?
        } else {
            // Fallback: activate + CGWindowListCreateImage
            eprintln!("ScreenCaptureKit unavailable, falling back to legacy capture");
            ctx.activate();
            std::thread::sleep(std::time::Duration::from_millis(100));
            let rect = CGRect::new(CGPoint::new(el_x, el_y), CGSize::new(el_w, el_h));
            eprintln!("Capturing {el_w:.0}x{el_h:.0} at ({el_x:.0},{el_y:.0}) → {path}");
            screenshot::capture(rect)
                .ok_or_else(|| AxError::ScreenshotFailed("capture failed".to_string()))?
        }
    } else {
        // Whole window capture: use first AX window's frame to pick the right SCK window
        let ax_win = find_first_ax_window(ctx);
        let win_image = if let Some(ref w) = ax_win {
            let (x, y) = w.position().unwrap_or((0.0, 0.0));
            let (ww, wh) = w.size().unwrap_or((0.0, 0.0));
            axcli::screen_capture::capture_window_by_frame(ctx.pid, x, y, ww, wh)
        } else {
            None
        };
        let win_image = win_image.or_else(|| axcli::screen_capture::capture_window_by_pid(ctx.pid));

        if let Some(win_image) = win_image {
            eprintln!("Capturing window → {path}");
            win_image
        } else {
            // Fallback: activate + CGWindowListCreateImage
            eprintln!("ScreenCaptureKit unavailable, falling back to legacy capture");
            ctx.activate();
            std::thread::sleep(std::time::Duration::from_millis(100));
            let rect = if let Some(ref w) = ax_win {
                let (x, y) = w.position().unwrap_or((0.0, 0.0));
                let (ww, wh) = w.size().unwrap_or((0.0, 0.0));
                eprintln!("Capturing window {ww:.0}x{wh:.0} at ({x:.0},{y:.0}) → {path}");
                CGRect::new(CGPoint::new(x, y), CGSize::new(ww, wh))
            } else {
                eprintln!("warning: no visible window found, capturing full screen → {path}");
                CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(0.0, 0.0))
            };
            screenshot::capture(rect)
                .ok_or_else(|| AxError::ScreenshotFailed("capture failed".to_string()))?
        }
    };

    if !screenshot::save_png(&image, &path) {
        return Err(AxError::ScreenshotFailed(format!("failed to save {path}")));
    }
    eprintln!("Saved: {path}");

    if ocr {
        run_ocr(&image)?;
    }

    Ok(())
}

fn cmd_wait(ctx: &ExecutionContext, target: &str) -> Result<(), AxError> {
    // Pure number = sleep ms
    if let Ok(ms) = target.parse::<u64>() {
        eprintln!("Waiting {ms}ms...");
        std::thread::sleep(std::time::Duration::from_millis(ms));
        return Ok(());
    }

    // Otherwise: poll for locator (timeout 10s)
    validate_locator(target)?;
    eprintln!("Waiting for '{target}'...");
    let timeout = std::time::Duration::from_secs(10);
    let node = ctx.wait_for(target, timeout)?;
    let _ = node;
    eprintln!("Found!");
    Ok(())
}

/// Collect text from a subtree, inserting newlines at group boundaries.
fn collect_text(node: &AXNode, max_depth: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    collect_text_inner(node, max_depth, &mut parts);
    parts.join("")
}

fn collect_text_inner(node: &AXNode, max_depth: usize, parts: &mut Vec<String>) {
    if max_depth == 0 {
        return;
    }
    let role = node.role().unwrap_or_default();

    // Text leaf: emit value
    if role == "AXStaticText" || role == "AXTextArea" || role == "AXTextField" {
        if let Some(val) = node.value() {
            if !val.is_empty() {
                parts.push(val);
            }
        }
        return;
    }

    // Block-level elements: insert newline before if we already have content
    let is_block = role == "AXGroup" || role == "AXList" || role == "AXTable"
        || role == "AXRow" || role == "AXHeading" || role == "AXParagraph"
        || role == "AXBlockquote" || role == "AXArticle";

    if is_block && !parts.is_empty() {
        if !parts.last().map_or(true, |s| s.ends_with('\n')) {
            parts.push("\n".to_string());
        }
    }

    for child in node.children() {
        collect_text_inner(&child, max_depth - 1, parts);
    }

    if is_block && !parts.is_empty() {
        if !parts.last().map_or(true, |s| s.ends_with('\n')) {
            parts.push("\n".to_string());
        }
    }
}

fn get_attr_value(node: &AXNode, attr: &GetAttr) -> Result<String, AxError> {
    match attr {
        GetAttr::Text => Ok(collect_text(node, 50)),
        GetAttr::Role => Ok(node.role().unwrap_or_default()),
        GetAttr::Title => Ok(node.title().unwrap_or_default()),
        GetAttr::Description => Ok(node.description().unwrap_or_default()),
        GetAttr::Value => Ok(node.value().unwrap_or_default()),
        GetAttr::DomId => {
            Ok(accessibility::attr_string(&node.0, "AXDOMIdentifier").unwrap_or_default())
        }
        GetAttr::Classes => Ok(node.dom_classes().join(" ")),
        GetAttr::Actions => Ok(node.actions().join(" ")),
        GetAttr::Position => match node.position() {
            Some((x, y)) => Ok(format!("{x:.0},{y:.0}")),
            None => Err(AxError::AttributeNotFound("AXPosition".to_string())),
        },
        GetAttr::Size => match node.size() {
            Some((w, h)) => Ok(format!("{w:.0},{h:.0}")),
            None => Err(AxError::AttributeNotFound("AXSize".to_string())),
        },
        GetAttr::ChildCount => Ok(node.child_count().to_string()),
        GetAttr::Raw(name) => {
            let ax_name = if name.starts_with("AX") {
                name.clone()
            } else {
                let mut c = name.chars();
                let cap = match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().to_string() + c.as_str(),
                };
                format!("AX{cap}")
            };
            match accessibility::attr_string(&node.0, &ax_name) {
                Some(val) => Ok(val),
                None => Err(AxError::AttributeNotFound(ax_name)),
            }
        }
    }
}

fn cmd_get(ctx: &ExecutionContext, attr: &GetAttr, locator: &str, all: bool) -> Result<(), AxError> {
    validate_locator(locator)?;

    if all {
        let nodes = ctx.app.locate_all(locator);
        if nodes.is_empty() {
            return Err(AxError::LocatorNotFound(locator.to_string()));
        }
        eprintln!("Found {} matches for {locator}", nodes.len());
        for (i, node) in nodes.iter().enumerate() {
            let val = get_attr_value(node, attr)?;
            if nodes.len() > 1 {
                eprintln!("--- match {}/{} ---", i + 1, nodes.len());
            }
            print!("{val}");
            if !val.ends_with('\n') {
                println!();
            }
        }
    } else {
        let node = resolve_one(ctx, locator)?;
        let val = get_attr_value(&node, attr)?;
        print!("{val}");
        if !val.ends_with('\n') {
            println!();
        }
    }
    Ok(())
}

// --- Watch (daemon mode) ---

/// AXObserver callback — called on every notification.
unsafe extern "C-unwind" fn watch_callback(
    _observer: NonNull<AXObserver>,
    element: NonNull<AXUIElement>,
    notification: NonNull<CFString>,
    refcon: *mut c_void,
) {
    let json_mode = !refcon.is_null();
    let notif = unsafe { notification.as_ref() }.to_string();
    let el = unsafe { element.as_ref() };
    let role = attr_string(el, "AXRole").unwrap_or_default();
    let title = attr_string(el, "AXTitle").unwrap_or_default();
    let desc = attr_string(el, "AXDescription").unwrap_or_default();

    // Classify: does this notification invalidate refs?
    let stale = matches!(
        notif.as_str(),
        "AXUIElementDestroyed" | "AXCreated" | "AXLayoutChanged" | "AXRowCountChanged"
    );

    if json_mode {
        let label = if !title.is_empty() { &title } else { &desc };
        let escaped_label = label.replace('\\', "\\\\").replace('"', "\\\"");
        let escaped_notif = notif.replace('\\', "\\\\").replace('"', "\\\"");
        let escaped_role = role.replace('\\', "\\\\").replace('"', "\\\"");
        println!(
            r#"{{"notification":"{}","role":"{}","label":"{}","refs_stale":{}}}"#,
            escaped_notif, escaped_role, escaped_label, stale
        );
    } else {
        let label = if !title.is_empty() {
            &title
        } else if !desc.is_empty() {
            &desc
        } else {
            ""
        };
        let stale_marker = if stale { " ⚠️  refs stale" } else { "" };
        let short = if label.len() > 60 {
            format!("{}...", &label[..57])
        } else {
            label.to_string()
        };
        eprintln!("{notif} → {role} \"{short}\"{stale_marker}");
    }
}

fn cmd_watch(pid: i32, _app: &AXNode, format: &str) -> Result<(), AxError> {
    let json_mode = format == "json";

    let mut observer_ptr: *mut AXObserver = std::ptr::null_mut();
    let cb: AXObserverCallback = Some(watch_callback);
    #[allow(deprecated)]
    let err = unsafe {
        objc2_application_services::AXObserverCreate(
            pid,
            cb,
            NonNull::new(&mut observer_ptr as *mut *mut AXObserver).unwrap(),
        )
    };
    if err != AXErr(0) || observer_ptr.is_null() {
        return Err(AxError::ActionFailed(format!("AXObserverCreate failed: {err:?}")));
    }
    let observer = unsafe { &*observer_ptr };

    // The app-level element to observe
    let ax_app = unsafe { AXUIElement::new_application(pid) };

    // Notifications to monitor
    let notifications = [
        "AXUIElementDestroyed",
        "AXCreated",
        "AXLayoutChanged",
        "AXValueChanged",
        "AXTitleChanged",
        "AXFocusedUIElementChanged",
        "AXSelectedChildrenChanged",
        "AXRowCountChanged",
        "AXWindowCreated",
        "AXWindowMoved",
        "AXWindowResized",
        "AXMenuOpened",
        "AXMenuClosed",
    ];

    let refcon = if json_mode {
        1usize as *mut c_void
    } else {
        std::ptr::null_mut()
    };

    let mut registered = 0;
    for notif in &notifications {
        let cf_notif = CFString::from_str(notif);
        #[allow(deprecated)]
        let result = unsafe {
            objc2_application_services::AXObserverAddNotification(
                observer,
                &ax_app,
                &cf_notif,
                refcon,
            )
        };
        if result == AXErr(0) {
            registered += 1;
        }
    }

    if registered == 0 {
        return Err(AxError::ActionFailed("failed to register any notifications".into()));
    }

    // Add observer to run loop
    let source = unsafe { observer.run_loop_source() };
    let run_loop = CFRunLoop::current().expect("no current run loop");
    unsafe {
        run_loop.add_source(Some(&source), kCFRunLoopDefaultMode.as_deref());
    }

    if !json_mode {
        eprintln!("Watching {registered}/{} notifications (Ctrl+C to stop)...", notifications.len());
    }

    // Run forever (until Ctrl+C)
    CFRunLoop::run();

    Ok(())
}
