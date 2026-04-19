# macOS 后台点击（Background Click）深度调研

> 目标：在**不激活、不抢焦点**的前提下，向指定 App 的某个 UI 元素投递点击事件。即"目标应用 `active=false`、窗口可能被遮挡、用户鼠标不动"的场景下仍能生效。
>
> 本文档汇总并标注 2025–2026 年社区已知方案，作为 `axcli` 新增 `strategy=cg-pid` / `strategy=sls` 等路径的设计依据。

---

## 0. 信度标注说明

下文所有结论按三级标注：

- ✅ **已验证**：有公开文档、Apple header、或开源项目的实际生产代码支撑。
- ⚠️ **社区反向工程**：来自 CGSInternal 类 header、逆向博客、论坛讨论；多年稳定但不在 Apple 官方 API 范围。
- ❓ **未证实**：仅在传闻、旧 gist、闭源工具 UI 里出现，未能在主流开源项目中找到落地代码。

---

## 1. macOS 输入事件架构速览

```
 ┌─────────────────┐
 │  IOHIDSystem    │  ← 真实硬件（内核）
 └────────┬────────┘
          │ HID events
 ┌────────▼────────┐
 │   WindowServer  │  ← SkyLight.framework 守护进程
 │  (hit-testing)  │    负责命中测试、坐标转换、焦点路由
 └────────┬────────┘
          │ Mach IPC
 ┌────────▼────────────────────────────┐
 │  每个 GUI 进程的事件队列            │
 │  → NSApplication.sendEvent:         │
 │  → NSWindow.sendEvent: → hit view   │
 └─────────────────────────────────────┘

 旁路：
  - accessibilityd ──AX IPC──→ 目标 App 的 AX server
    （AXUIElementPerformAction 走这条路，不经 WindowServer）
```

几个关键抽象：

- `CGEventRef`（公开） → 底层是 `CGSEventRecord`（私有）。`CGEventGetEventRecord(...)` 是两者之间的桥。✅
- `CGWindowID`：进程视角下的窗口 ID，来自 `CGWindowListCopyWindowInfo`。✅
- `PSN (ProcessSerialNumber)`：老 Carbon 时代的进程句柄，SkyLight/SLPS 系列 API 仍在用。✅
- `CGSConnectionID`：每个进程对 WindowServer 的连接 ID，由 `CGSMainConnectionID()` / `SLSMainConnectionID()` 返回。⚠️

---

## 2. 方案 A：CoreGraphics 公开 + 半公开字段

**核心思路**：构造 `CGEventRef`，往它内部塞 `CGWindowID` + 窗口内局部坐标，调用 `CGEventPostToPid(pid, event)` 绕过 WindowServer 的命中测试，直接把事件送到目标进程的事件队列。

### 2.1 关键 API

| API | 可见性 | 作用 |
|---|---|---|
| `CGEventCreateMouseEvent` | ✅ 公开 | 构造鼠标事件。现有代码已在用。 |
| `CGEventSetIntegerValueField` | ✅ 公开 | 以整数索引写 CGEvent 内部字段。 |
| `CGEventPostToPid(pid, event)` | ✅ 公开 | 直接投递给特定进程，不走 HID tap。 |
| `CGEventGetEventRecord(evt, &rec, size)` | ⚠️ 私有但 NUIKit/CGSInternal 里声明稳定 | 取底层记录。 |
| `CGSSetEventRecordWindow(evt, wid)` | ❓ 旧 header 里有，但等价于 SetIntegerValueField(55, wid)，实际项目都走后者 | — |

### 2.2 `CGEventField` 索引表（公开 + 社区）

公开 header 到大约 46 就截止了。高序号字段源自 [`NUIKit/CGSInternal/CGSEvent.h`](https://github.com/NUIKit/CGSInternal/blob/master/CGSEvent.h) 和 Hammerspoon [`libeventtap_event.m`](https://github.com/Hammerspoon/hammerspoon/blob/master/extensions/eventtap/libeventtap_event.m)。

| Index | 名称（社区） | 含义 | 信度 |
|---|---|---|---|
| 7 | `kCGMouseEventSubtype` | 鼠标事件子类型（0/1/2/3） | ✅ 公开 |
| 42 | `kCGEventSourceUserData` | 用户自定义 | ✅ 公开 |
| 51 (0x33) | `kCGEventSourceUserData` subfield / source-related | 透传 | ⚠️ |
| 55 (0x37) | `kCGMouseEventWindowUnderMousePointer` | **鼠标下的 CGWindowID** | ✅ 公开（10.15+） |
| 56 (0x38) | `kCGMouseEventWindowUnderMousePointerThatCanHandleThisEvent` | **实际接收事件的 CGWindowID** | ✅ 公开（10.15+） |
| 89 (0x59) | `kCGEventUnacceleratedPointerMovementX` | 原始 dx（float） | ⚠️ |
| 90 (0x5A) | `kCGEventUnacceleratedPointerMovementY` | 原始 dy | ⚠️ |
| 91 (0x5B) | **窗口内局部 X 坐标**（bit pattern of CGFloat） | 截图里的 `field91` | ⚠️ 强推测 |
| 92 (0x5C) | **窗口内局部 Y 坐标** | 与 91 配对 | ⚠️ 强推测 |
| 93 (0x5D) | 有观测是目标 window ID 的另一字段 | 备用 | ❓ |

> **关于截图中的 `field91=295362512`**：这个 "大整数" 是 `CGFloat` 的 bit pattern 被 `CGEventSetIntegerValueField` 当 int64 存进去的结果。值本身不直接可读，需要 `f64::from_bits` 还原。社区脚本大多采用成对写 91/92。

### 2.3 鼠标子类型（`subtype`）

| 值 | 名称 | 备注 |
|---|---|---|
| 0 | `kCGEventMouseSubtypeDefault` | 普通鼠标 ✅ |
| 1 | `kCGEventMouseSubtypeTabletPoint` | 数位板 ✅ |
| 2 | `kCGEventMouseSubtypeTabletProximity` | 数位板接近 ✅ |
| **3** | 推测 = `kCGEventMouseSubtypeWindowServer`（社区命名） | **"事件已携带解析后的目标窗口 ID，不要再做命中测试"** ⚠️ |

截图里 `subtype=3` 与字段 55/56/91/92 同时出现，语义自洽：告诉接收方 "我已经锁定窗口和局部坐标，不要回退到全局 hit-test"。

### 2.4 实现骨架（Rust 伪代码）

```rust
// 已知：pid, window_id (CGWindowID), window_frame (AX 拿到),
//       screen_point = window_frame.origin + local_point

let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)?;
let down = CGEvent::new_mouse_event(
    source.clone(), CGEventType::LeftMouseDown,
    screen_point, CGMouseButton::Left)?;

// 塞目标窗口
unsafe {
    CGEventSetIntegerValueField(down, 55 /* UnderMousePointer */, wid as i64);
    CGEventSetIntegerValueField(down, 56 /* ...CanHandle */,       wid as i64);
    // 窗口内局部坐标（注意 f64 bit pattern）
    CGEventSetIntegerValueField(down, 91, local_x.to_bits() as i64);
    CGEventSetIntegerValueField(down, 92, local_y.to_bits() as i64);
    CGEventSetIntegerValueField(down, 7  /* Subtype */, 3);
}
CGEventPostToPid(pid, down);

// 同样构造 up 并 post
```

### 2.5 典型参考实现

- **Hammerspoon** `hs.eventtap` — 最成熟、被验证过的开源实现。暴露 `mouseEventWindowUnderMousePointer` 属性。[`libeventtap_event.m`](https://github.com/Hammerspoon/hammerspoon/blob/master/extensions/eventtap/libeventtap_event.m)
- **WebKit** `WebKitTestRunner/mac/EventSenderProxy.mm` — Apple 自家 UI 测试的合成事件路径。[`EventSenderProxy.mm`](https://github.com/WebKit/WebKit/blob/main/Tools/WebKitTestRunner/mac/EventSenderProxy.mm)
- **Chromium** `ui/base/test/ui_controls_mac.mm`
- **Rust `core-graphics`** crate — 已绑定 `CGEvent::post_to_pid`，但 `CGEventField` 枚举只覆盖公开值；高序号索引需自行 `unsafe` 调用。

### 2.6 TCC / 权限

| 项 | 是否必需 |
|---|---|
| Accessibility | ✅ 必需（与 `CGEventPost` 相同） |
| Screen Recording | 仅当你还要 `CGWindowListCreateImage` 验证 |
| Input Monitoring | **不需要**（只投递、不监听） |
| 禁 SIP | 不需要 |
| 特殊 entitlement | 不需要，但**不能 sandbox** |
| Hardened runtime | 兼容 |

---

## 3. 方案 B：SkyLight / WindowServer 私有路径

**核心思路**：直接使用 `SLS` / `SLPS` 前缀的私有函数，绕过 CGEvent 封装，把原始事件字节喂给 WindowServer。

### 3.1 已验证存在的符号

来自 [`NUIKit/CGSInternal`](https://github.com/NUIKit/CGSInternal) 和 [yabai/src/window_manager.c](https://github.com/koekeishiya/yabai/blob/master/src/window_manager.c)：

| 符号 | 原型（大致） | 验证来源 |
|---|---|---|
| `CGSMainConnectionID()` / `SLSMainConnectionID()` | `() -> CGSConnectionID` | NUIKit、yabai、Hammerspoon ✅ |
| `_SLPSGetFrontProcess(&psn)` | 获取前台进程 PSN | yabai ✅ |
| `_SLPSSetFrontProcessWithOptions(psn, wid, mode)` | 设置前台进程及关联窗口，带 `kCPSUserGenerated` / `kCPSNoWindows` 选项 | yabai ✅ |
| `SLPSPostEventRecordTo(psn, bytes)` | 向 PSN 投递一段事件字节 | yabai ✅（用于激活信号，不是任意点击） |
| `CGEventGetEventRecord` | 由 CGEventRef 得到 CGSEventRecord | NUIKit ✅ |
| `CGSGetWindowBounds` / `CGSGetWindowList` | 窗口信息 | 稳定，多项目在用 ✅ |

### 3.2 **存疑**符号（不要轻信）

| 符号 | 说明 |
|---|---|
| `SLPSPostEventRecordToWindow(cid, wid, &rec)` | ❓ 老 gist/博客里被提到，但在当前 yabai master 和 NUIKit 里**没有找到声明或调用点**。yabai 实际用的是 `SLPSPostEventRecordTo(psn, bytes)`，后者按 PSN 而非 windowID 投递。 |
| `SLSPostEventRecordToWindowWithOptions` | ❓ 同上。命名符合 SLS 规则，但无公开源可验证。 |
| SkyLight 内 "EventTap" 前缀符号 | ❓ 大多数 EventTap 语义都走公开的 `CGEventTap*`。 |

**一个重要结论**：公开开源社区里没有一个工具稳定演示了"通过 `SLPS*` 把任意鼠标事件投递到指定 `CGWindowID` 且保持窗口遮挡状态"。yabai 只用 `SLPS*` 完成**激活/抬起焦点的三步走**（`_SLPSSetFrontProcessWithOptions` → 两次 `SLPSPostEventRecordTo`（activation bytes 0x01/0x02）→ `AXUIElementPerformAction(kAXRaise)`），**点击的命中逻辑不在 SLPS 这条路上**。

### 3.3 `CGSEventRecord` 结构（NUIKit）

```c
typedef struct _CGSEventRecord {
    CGSEventRecordVersion major, minor;
    CGSByteCount length;
    CGSEventType type;
    CGPoint location, windowLocation;   // 全局坐标 + 窗口内坐标
    CGSEventRecordTime time;
    CGSEventFlag flags;
    CGWindowID window;                  // ← 关键
    CGSConnectionID connection;
    struct __CGEventSourceData { ... } eventSource;
    struct _CGEventProcess  { ... } eventProcess;
    NXEventData eventData;
    void *ioEventData;
    struct _CGSEventAppendix { ... } *appendix;
    CFDataRef data;
} CGSEventRecord;
```

> **警告**：这个结构体布局**每个 macOS 主版本都可能变**。`CGEventGetEventRecord` 的第三个参数是显式的 `recSize`，就是为此而设。任何依赖固定 `sizeof` 的代码都会在下一个系统升级时炸掉。

### 3.4 权限特别注意

- SLS/SLPS 大部分符号**不要求关 SIP**（它们在 dyld shared cache 里，可 `dlsym` 到）。
- yabai 的 **scripting addition**（注入 Dock.app）才需要关 SIP —— 我们不需要那一层。
- 仍需 Accessibility 权限。
- 私有 API 的通告（notarization）**是可以通过**的（yabai、BetterTouchTool 都已通过），但这不代表 Apple 批准使用。每次系统大版本升级都要重验。

### 3.5 小结

SkyLight 路径在"**激活特定进程+特定窗口但不抢整个 App 焦点**"这个子问题上有用（yabai 模式）。对于"**彻底后台、不抬焦点**地点某个按钮"，**目前没有公开可复现的 SLS 路径**。优先级低于方案 A。

---

## 4. 方案 C：纯 AX（AXUIElementPerformAction）

**核心思路**：不发任何合成事件，直接通过 AX IPC 让目标 App 自己去触发 action。这条路走 `accessibilityd`，不经 WindowServer，天然支持后台。

### 4.1 关键 action 常量

来自 `HIServices/AXActionConstants.h`（公开）：

- `kAXPressAction` — 默认点击
- `kAXIncrementAction` / `kAXDecrementAction` — 滑块、stepper
- `kAXConfirmAction` / `kAXCancelAction` — 对话框按钮
- `kAXShowMenuAction` — 右键菜单
- `kAXRaiseAction` — 把窗口抬到同 App 的其他窗口之前（不抢整个 App 焦点）
- `kAXPickAction` — 拾取（如颜色吸管）
- `kAXShowAlternateUIAction` / `kAXShowDefaultUIAction`

### 4.2 属性替代（Attribute Set）

对许多控件，"设置属性"比"点一下"更可靠：

- `kAXFocusedAttribute = true` — 让控件聚焦
- `kAXValueAttribute` — 设文本框 / 滑块值
- `kAXSelectedAttribute`、`kAXSelectedChildrenAttribute` — 选中列表项
- `kAXSelectedTextAttribute`、`kAXSelectedTextRangeAttribute` — 文本编辑

对 Cocoa 原生控件几乎总有效；对 Electron/CEF 渲染内容脆弱。

### 4.3 验证副作用

`AXPress` 会返回 `kAXErrorSuccess` **即便什么也没发生**（Electron 的典型反应）。务必二次验证：

- 用 `AXObserverAddNotification` 监 `kAXValueChangedNotification` / `kAXSelectedChildrenChangedNotification`
- 或直接在点击后短延迟 re-read 目标属性

### 4.4 防挂起

`AXUIElementSetMessagingTimeout(elem, 1.5)` —— 目标 App 被 App Nap 挂起时，AX IPC 默认会久等。必设超时（1–2 秒）。

---

## 5. 常见失效场景（按方案）

| 场景 | CGEventPostToPid | SLS/SLPS | AXPress |
|---|---|---|---|
| 游戏（Unity/Unreal/SDL） | ❌ 走 IOHIDManager，合成事件到不了 | ❌ | ❌（无 AX 树） |
| Electron 渲染内容 | ⚠️ 部分生效，焦点敏感交互失败 | ⚠️ | ⚠️ 需 App 显式打开 `AXManualAccessibility` |
| 浏览器内页面 | ❌ 渲染进程不接收 NSEvent | ❌ | 通过浏览器 AX 树可行 |
| Secure Input（密码框、sudo、1Password） | 鼠标可过，键盘被系统层拒 | 同左 | AXPress 仍可，但 TCC 弹窗类需 `hid-control` entitlement |
| 检查 `[NSApp isActive]` 的 AppKit 应用 | ⚠️ 事件到了但被 no-op | ⚠️ | ✅（走 AX） |
| 反作弊（Roblox 类） | ❌ | ❌ | ❌ |
| Mac App Store sandbox 目标 | ✅（调用者不 sandbox 即可） | ✅ | ✅ |
| 调用方 sandbox | ❌ 全部禁止 | ❌ | ❌ |

---

## 6. 工具横向对比

| 工具 | 点击机制 | 支持后台窗口 | 源码 |
|---|---|---|---|
| **cliclick** | `CGEventPost(HIDEventTap)` | ❌ | [BlueM/cliclick](https://github.com/BlueM/cliclick) |
| **pyautogui (macOS)** | `CGEventPost(HIDEventTap)` | ❌ | [asweigart/pyautogui](https://github.com/asweigart/pyautogui) |
| **enigo** (Rust) | `CGEventPost(HIDEventTap)` | ❌ | [enigo-rs/enigo](https://github.com/enigo-rs/enigo) |
| **Hammerspoon** `hs.eventtap` | CGEventPost 或 `CGEventPostToPid`（带 app 参数时） | ⚠️ 部分，与 axcli 要做的相同路径 | [libeventtap_event.m](https://github.com/Hammerspoon/hammerspoon/blob/master/extensions/eventtap/libeventtap_event.m) |
| **Hammerspoon** `hs.axuielement` | `AXUIElementPerformAction` | ✅ | 同上 |
| **atomac / atomacos** | AXPress 为主 | ✅ | [pyatom](https://github.com/pyatom/pyatom)、[atomacos](https://github.com/daveenguyen/atomacos) |
| **yabai** | SLPS 激活 + AXRaise，不做任意点击 | 专注窗口管理 | [yabai](https://github.com/koekeishiya/yabai) |
| **Appium Mac2** | XCTest + AX | 会激活目标 | [appium-mac2-driver](https://github.com/appium/appium-mac2-driver) |
| **WebKit 测试器** | `CGEventPostToPid(self, evt)` | 本质是"给自己发" | [EventSenderProxy.mm](https://github.com/WebKit/WebKit/blob/main/Tools/WebKitTestRunner/mac/EventSenderProxy.mm) |
| **System Events (AppleScript)** | AX wrapper | ⚠️ 多数场景需激活 | 闭源 |

---

## 7. 社区讨论与经典文献

- Apple DevForums: [CGEventPostToPid not posting to background app's open dialog](https://developer.apple.com/forums/thread/724835)
- Apple DevForums: [Emulate mouse click (sandbox limits)](https://developer.apple.com/forums/thread/685618)
- MacroMates (2005): [Controlling inactive windows](https://macromates.com/blog/2005/controlling-inactive-windows/) — 二十年前就点出了核心限制
- MacScripter: [Simulate click in unscriptable background app window](https://www.macscripter.net/t/simulate-click-or-keystroke-in-unscriptable-background-app-window/72541)
- Eclectic Light: [WindowServer as display compositor and input event router](https://eclecticlight.co/2020/06/08/windowserver-display-compositor-and-input-event-router/)
- Tencent Keen Lab: [WindowServer: The privilege chameleon (Part 1)](https://keenlab.tencent.com/en/2016/07/22/WindowServer-The-privilege-chameleon-on-macOS-Part-1/) / [Part 2](https://keenlab.tencent.com/en/2016/07/28/WindowServer-The-privilege-chameleon-on-macOS-Part-2/) — MIG IPC 层
- RET2 Pwn2Own 2018: [Exploiting the macOS WindowServer](https://blog.ret2.io/2018/08/28/pwn2own-2018-sandbox-escape/) — SLPS/HotKey 机制逆向
- Siguza: [IOHIDeous](https://github.com/Siguza/IOHIDeous/blob/master/docs/index.md) — IOHIDSystem 内核内幕
- Jamf: [Synthetic Reality](https://www.jamf.com/blog/synthetic-reality/) — 为何现代 macOS 在受保护 UI 过滤合成事件
- Hammerspoon issues [#1282](https://github.com/Hammerspoon/hammerspoon/issues/1282), [#1334](https://github.com/Hammerspoon/hammerspoon/issues/1334), [#3769](https://github.com/Hammerspoon/hammerspoon/issues/3769) — `leftClick` 在非前台 / 焦点敏感 App 的失败记录

---

## 8. 对 `axcli` 的推荐路线

按优先级：

### 8.1 P0 — 新增 `strategy=cg-pid`（本分支目标）

参照 Hammerspoon。在现有 `strategy=cg`（全局 HID tap）旁再加一条：

1. 从 AX 拿 `pid` 和 window frame（已有）
2. 从 `CGWindowListCopyWindowInfo` 拿 `CGWindowID`（应该已有；若无则加）
3. 构造 down/up event，写入字段 55/56（windowID）、91/92（局部坐标 bits）、7（subtype=3）
4. `CGEventPostToPid(pid, ...)`

**落点建议**：
- `src/input.rs`:
  - 新增 `click_at_window(pid: i32, window_id: u32, window_local: CGPoint, screen: CGPoint)`
  - 抽一个 `set_integer_field(event, idx, value)` unsafe 小帮手，便于写非公开 index
- `src/actions.rs`:
  - click 路径按 strategy 分派
- CLI: `--strategy cg-pid`

### 8.2 P1 — AX 优先 + CG 回退的组合策略

- 先尝试 `AXPress`
- 带 `AXUIElementSetMessagingTimeout(~1.5s)`
- 调用后短等（150–300ms）读一个可观测属性（例如 focused/selected/value）
- 若无变化，回退到 `cg-pid` 路径
- 再无则回退到 `cg`（全局）并可选择先 activate app

这是 atomacos / Keyboard Maestro 类工具的实证最稳定组合。

### 8.3 P2 — Demo 验证工具

写一个小的 PoC 程序，针对自写的 `EchoApp` 验证以下矩阵：

| field55 | field56 | field91/92 | subtype | PostToPid | 结果 |
|---|---|---|---|---|---|
| — | — | — | 0 | ✓ |（对照组）|
| wid | — | — | 0 | ✓ | |
| wid | wid | — | 0 | ✓ | |
| wid | wid | local | 0 | ✓ | |
| wid | wid | local | 3 | ✓ | ← 截图的组合 |

目的：隔离出每一位的实际必要性。社区文档都是"拼凑出来的"，需要我们自己交叉验证当前 macOS 版本。

### 8.4 P3 — SkyLight 路径（探索性，低优先级）

仅当 P0 + P1 都不够用（例如某些检查 `NSApp.isActive` 的 App），再考虑：

- `_SLPSSetFrontProcessWithOptions(psn, wid, kCPSNoWindows)` — 让目标 App 误以为自己是前台（不真正改 z-order）
- 之后走 P0 的 CGEventPostToPid

这会牺牲"完全无感"属性，但对顽固目标可能有效。yabai 的焦点路径是参考。

---

## 9. 风险与未决问题

1. **字段索引跨版本漂移**：89–93 在 10.11 → 10.12 之间有过一次挪动（社区信息）。发布前在目标 macOS 版本全跑一遍验证矩阵。
2. **Sequoia 收紧**（传闻，未实测）：不属于目标 pid 的 windowID 会被静默丢弃。我们的流程自带了 pid、window_id 的匹配，影响小，但要打 log 监测。
3. **App 用 `isActive` / `isKeyWindow` 闸门**：这类 App 即使收到事件也不执行逻辑。没有通用解法，只能 P3 方案 SLPS 欺骗或真实激活。
4. **Electron AX 空树**：`AXManualAccessibility` 需 App 自己打开。CG 方向比 AX 方向更有可能生效，但内部控件的命中测试也可能失败。
5. **`subtype=3` 的合法性**：未经 Apple 官方命名。最坏情况某次系统更新把 subtype=3 改成校验失败即静默丢弃。保持 subtype=0 为回退。

---

## 10. 执行情况

调研阶段（§1-§9）列出的方案在 `research/background-click` 分支做了 PoC。**实装完成 + 实测结论见附录 A**，该节替代了 §8.1–§8.4 的初步计划，是当前事实记录。

----

## 附录 A：最终实证结果（2026-04-19，macOS 14.x）

> 本节是 PoC 收尾后的**确定结论**。上文的假设（特别是 §5、§9 里对 `isKeyWindow` 门控的推测）部分被实验推翻，请以本节为准。

### A.1 测试方法

三个独立诊断手段叠加，以隔离不同失败可能：

1. **Post 路径对照**：同一 `CGEventRef` 分别走 `CGEventPostToPid(pid, ev)` 和 `CGEventPost(HIDEventTap, ev)`，观察目标响应差异 —— 这分离"事件内容"和"投递路径"。
2. **字段 dump**：post 之前用 `CGEventGetIntegerValueField` 把 type / location / flags / f7(Subtype) / f91/92(WindowUnderMousePointer/CanHandle) / f42(SourceUserData) / f46(TargetUnixProcessID) 全读回来，确认写入生效。
3. **前台状态与目标状态对照**：`osascript` 实时读前台应用，`axcli get` 读目标控件属性，双向确认"点击是否真的落地 + 焦点是否真的没动"。

对照组：`strategy=ax`（AXUIElementPerformAction）与 `strategy=cg`（全局 HID tap + 激活）。

### A.2 鼠标 `cg-pid`：对 AppKit **无效**

**Calculator 前台 + key window**下的字段组合矩阵（`press 7 等价`的鼠标点击"7"按钮）：

| 字段组合 | post 路径 | 显示变化 | 结论 |
|---|---|---|---|
| f7=3 + f91/92=wid | `CGEventPostToPid` | 77 → 77 | FAIL |
| f7=3 + f91/92=wid | `CGEventPost(HIDEventTap)` | 77 → **777** | **PASS** |
| f7=0 + 无 winfields（裸 post_to_pid）| `CGEventPostToPid` | 777 → 777 | FAIL |
| — | `AXPress` | 0 → 7 | PASS（对照）|

dump 确认所有字段写入成功：`f7=3, f91=28668, f92=28668`。**同一事件、同一 PID**，换成 HID tap 投递就生效 —— 说明事件内容完全合法，**是 `CGEventPostToPid` 本身没把鼠标事件送到 Calculator**。

TextEdit 用 align-center radio（有干净的 0/1 AXValue）复验，结论相同：

| 策略 | center 值 | 前台 | 判定 |
|---|---|---|---|
| `ax` | 0→1 ✓ | Finder（不抢）| 后台生效 |
| `cg-pid` | 0→0 | Finder | **事件未送达** |
| `cg` | 0→1 ✓ | TextEdit（抢了）| 生效但抢焦点 |

### A.3 键盘 `--strategy pid`：对 AppKit **有效**

`press_key_combo_bg(pid, keycode, flags)` = `CGEventCreateKeyboardEvent` + `CGEventPostToPid`：

| 目标 | 前台 | 操作 | 结果 |
|---|---|---|---|
| Calculator | Finder | `press 7 7 8 9 --strategy pid` | 777 → **7777789** ✓ |
| TextEdit | Finder | 逐字符 `press h e l l o --strategy pid` | '' → **'hello'** ✓ |

两次测试 Finder 全程前台未动，键盘事件都落到目标 App 的 first responder。

### A.4 架构解释：为什么鼠标不行、键盘行

两种事件的 AppKit 分发机制不同：

- **键盘事件**：走 first-responder chain。`[NSApplication sendEvent:]` 直接把 key event 交给 `[window firstResponder]`，**不需要 WindowServer 先做窗口命中测试**。`CGEventPostToPid` 把事件塞进目标进程的事件队列，first responder 一样处理。
- **鼠标事件**：需要位置到窗口、窗口到视图的 hit-test。这层 hit-test 由 **WindowServer** 完成，结果以 windowID 形式附在事件上交给 App。`CGEventPostToPid` **绕开了 WindowServer**，App 收到事件时没有合法的窗口路由信息 —— 即使我们手动写了 f91/f92，AppKit 新版本也不消费这些字段。`-[NSWindow sendEvent:]` 没 view 可 route，就 drop 了。

这也解释了为什么 GitHub 上成功案例几乎全是键盘、被明确标记失败的几乎全是鼠标（见 A.5）。

### A.5 GitHub 交叉验证

鼠标 `CGEventPostToPid` 失败是社区共识，不是我们代码 bug：

| 项目 | 用途 | 与我们配方一致？ | 结果 |
|---|---|---|---|
| [barry-ran/learn-macos](https://github.com/barry-ran/learn-macos/blob/master/src/Cocoa/EventPostToPSN/EventPostToPSN/WindowDelegate.m) | 鼠标 | **完全同款**（f91/f92 + PostToPid）| 源码注释："鼠标事件转发到其他窗口无效" |
| [tiagosiebler/UniveralPokerBot](https://github.com/tiagosiebler/UniveralPokerBot/blob/master/Universal%20Poker%20Bot/External%20Window/ExternalWindow.m) | 鼠标 | 仅 ClickState | 目标是 Flash 非 Cocoa；"NSEvent won't work since flash isn't cocoa" |
| [Apple DevForum 724835](https://developer.apple.com/forums/thread/724835) | 鼠标到 Logic Pro 后台 dialog | — | 报告 PostToPid 不送达，**无 Apple 回复** |
| [Apple DevForum 730441](https://developer.apple.com/forums/thread/730441) | 鼠标到后台窗口 | — | "Unable to post synthetic mouse events outside" |
| [Quicksilver](https://github.com/quicksilver/Quicksilver/blob/master/Quicksilver/Code-QuickStepCore/QSGlobalSelectionProvider.m) | **键盘** | — | 生产使用多年，工作 |
| [GNOME/dasher](https://github.com/GNOME/dasher/blob/master/Src/MacOSX/KeyboardEvent.m) | **键盘** | — | 工作 |
| [keymanapp/keyman](https://github.com/keymanapp/keyman/blob/master/mac/Keyman4MacIM/Keyman4MacIM/KeySender.m) | **键盘** | — | 输入法场景大量使用 |

### A.6 对 axcli 的最终决策矩阵

| 任务 | 推荐策略 | 命令 | 后台 | 抢焦点 |
|---|---|---|---|---|
| 点击有 AXPress 的控件 | AX | `click --strategy ax` | ✅ | ✗ |
| 点击无 AXPress（自定义控件、坐标点）| 激活+HID | `click --strategy cg` | ✗ | ✓ |
| 点击非 Cocoa 自绘目标（游戏、Flash、某些 SwiftUI demo）| 尝试 pid | `click --strategy cg-pid` | 视情况 | ✗ |
| 键盘后台输入 | **pid** | `press --strategy pid` | ✅ | ✗ |
| 键盘需要全局广播 / first responder 在别处 | HID | `press --strategy hid` | ✗ | ✓ |
| 读树 / 读属性 | AX | `snapshot` / `get` | ✅（天然）| ✗ |
| 读像素 | SCK | `screenshot` | ✅（天然）| ✗ |

### A.7 未做的探索

两个方向保留给后续：

1. **P3 SLPS 焦点欺骗**（`_SLPSSetFrontProcessWithOptions` + 激活字节）：让目标 App 认为自己前台而不真正切 z-order。能不能让 AppKit 接 PostToPid 的鼠标事件，未测。风险：SIP 关系、跨版本稳定性。
2. **`click --strategy auto`** hybrid：AXPress 尝试失败回退到 cg + 激活。当前 click 没做，留作后续需求出现时再加。

---

## 附录 B：关键参考链接

**Headers**
- [NUIKit/CGSInternal](https://github.com/NUIKit/CGSInternal)（最活跃维护）
- [calftrail/Touch CGSPrivate.h](https://github.com/calftrail/Touch/blob/master/CGSPrivate.h)
- [rjw57 gist CGSPrivate.h](https://gist.github.com/rjw57/5495406)

**参考实现**
- [Hammerspoon libeventtap_event.m](https://github.com/Hammerspoon/hammerspoon/blob/master/extensions/eventtap/libeventtap_event.m)
- [WebKit EventSenderProxy.mm](https://github.com/WebKit/WebKit/blob/main/Tools/WebKitTestRunner/mac/EventSenderProxy.mm)
- [yabai window_manager.c](https://github.com/koekeishiya/yabai/blob/master/src/window_manager.c)
- [Chromium ui_controls_mac.mm](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/ui/base/test/ui_controls_mac.mm)

**Rust 生态**
- [core-foundation-rs/core-graphics](https://github.com/servo/core-foundation-rs/tree/main/core-graphics)
- [enigo-rs/enigo](https://github.com/enigo-rs/enigo)
