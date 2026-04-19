# 后台点击尝试记录（journey）

> 短文档：按时间记录我们对 **macOS 后台鼠标点击** 的每次尝试、为什么失败、以及最终跑通的配方。深度调研见 [background-click.md](background-click.md)。

## TL;DR — 可达路径

三要素必须**同时满足**。少一件就失败。

1. **用 NSEvent 工厂构造事件**（不要用 `CGEventCreateMouseEvent`）
   ```swift
   +[NSEvent mouseEventWithType:location:modifierFlags:timestamp:
                   windowNumber:context:eventNumber:clickCount:pressure:]
   → -[NSEvent CGEvent]
   ```
   工厂会自动填 12 个内部字段：`0, 1, 2, 41, 43, 44, 50, 51, 55, 59, 102, 108`。其中 **field 55 = windowNumber**，AppKit 的 `-[NSWindow sendEvent:]` 靠它做 view 路由。`CGEventCreateMouseEvent` 不填这些，所以事件到了 AppKit 也不知 route 去哪。

2. **写窗口内局部坐标**（私有 `CGEventSetWindowLocation`）
   ```c
   void (*CGEventSetWindowLocation)(CGEventRef, CGPoint);
   // via: dlsym(RTLD_DEFAULT, "CGEventSetWindowLocation")
   ```
   事件携带**两套坐标**：屏幕坐标 + 窗口内坐标。hit-test 靠的是后者。

3. **Target 非 active 时设 Command flag**
   ```
   modifierFlags = kCGEventFlagMaskCommand (0x00100000)
   条件：[NSRunningApplication runningApplicationWithProcessIdentifier:pid].isActive == false
   ```
   这是**逆向出来的不成文约定**，没有文档，作用类似 "WindowServer 绕过位" 的信号。Cmd 本身的语义在这儿不重要。

配套字段写入（即使 NSEvent 路径部分是冗余也写，按原作者配方）：
- `field 3 = kCGMouseEventButtonNumber` = 0（左键）
- `field 7 = kCGMouseEventSubtype` = 3
- `field 91 = kCGMouseEventWindowUnderMousePointer` = wid
- `field 92 = ...ThatCanHandleThisEvent` = wid

投递：`CGEventPostToPid(pid, event)`。不激活，不移动光标。

**代码位置**：`src/input.rs::mouse_click_bg`。

**实证**（macOS 14.x，Finder 前台）：
- Calculator 后台，连续点击 `1 2 3 + 4 5` 全部生效，display 0 → 45，Finder 全程前台未动
- TextEdit 后台，`align center` radio 切换 0→1（同时 `align left` 1→0），Finder 未动

## 尝试时间线

每一步是"相对上一步的增量"。

### 1. 最小配方：`CGEventPostToPid` + field 91/92 + subtype=3 → FAIL

Hammerspoon `hs.eventtap` 风格。`CGEventCreateMouseEvent(screen)` 后写 f91/f92 = wid，f7=3，post_to_pid。Calculator、TextEdit 都毫无反应，焦点也没动。

第一反应：`[NSWindow isKeyWindow]` 门控。

### 2. Calculator 前台时重试 → 仍 FAIL

**推翻 isKeyWindow 假设**。Calculator 就是前台 + key window，display=7 时 `cg-pid` 点 "7"，display 仍是 7，没变成 77。`ax` 对照组同一时刻能切到 77。

结论：不是 key-window 问题，是**事件根本没送达 Calculator**。

### 3. 字段 dump + HID-path 切换诊断 → 定位到投递路径

加了 `AXCLI_CGPID_DUMP=1` 和 `AXCLI_CGPID_POST=hid`：
- dump：确认 f7=3、f91/92=28668 全部写入成功
- 同一个 CGEvent 换 `CGEventPost(HIDEventTap)` 投递 → **生效** (77 → 777)
- 换回 `CGEventPostToPid` → 不生效

硬结论：**事件内容合法，`CGEventPostToPid` 本身没把鼠标事件送到 Calculator**。

### 4. GitHub 调研 → 发现键盘与鼠标的路由差异

查到：
- [barry-ran/learn-macos](https://github.com/barry-ran/learn-macos) 用一模一样的配方，作者明确写 "鼠标事件转发到其他窗口无效"
- [Apple DevForum 724835](https://developer.apple.com/forums/thread/724835) 相同现象，**无 Apple 回复**
- 键盘路径 Quicksilver / Keyman / Dasher 都在用 PostToPid，多年生产

架构差异：键盘走 first-responder chain 不需要 WindowServer 路由，鼠标需要 hit-test。PostToPid 绕过了 WindowServer。

**这一阶段我们以为鼠标 PostToPid 对 Cocoa 就是不可行的，下结论写进了 background-click.md 附录 A**。

### 5. 顺手验证键盘路径 → ✅ 成功

加了 `press --strategy pid`：
- Calculator 后台：`press 7 7 8 9 pid` → display `777` → `7777` → `77777789`，Finder 不动
- TextEdit 后台：逐字符 `press h e l l o pid` → text area 从 '' 变 `'hello'`，Finder 不动

键盘通道确认可用。

### 6. 用户找到原作者 skill 文档 → 回头重试鼠标

[Lakr233/bgclick-rev-skill](https://github.com/Lakr233/bgclick-rev-skill) 文档揭示我们**缺了两件**：

- **`CGEventSetWindowLocation`** (私有、dlsym 解析)：事件要同时携带窗口内局部坐标
- **`kCGEventFlagMaskCommand`** flag：target 非 active 时设，作为 WindowServer 绕过位

加上这两件后重试 Calculator → 仍 FAIL。

### 7. 再读一遍 skill，发现关键遗漏：NSEvent 工厂

skill 文档的 invariant #2 明确：

> Event synthesis is `+[NSEvent mouseEventWithType:...]` then `-[NSEvent CGEvent]` to extract the `CGEventRef`, then explicit field writes.

**不是** `CGEventCreateMouseEvent`。NSEvent 工厂填 12 个字段，其中 field 55 = windowNumber 是 AppKit 路由的关键。`CGEventCreateMouseEvent` 不填这些 → AppKit 收到事件但没 windowNumber 能 route 的控件。

切到 NSEvent 路径 → **Calculator 立刻响应，TextEdit 也响应**，焦点都没动。✅

## 失败路径备忘录（避免重走）

| 尝试 | 为什么失败 |
|---|---|
| 裸 `CGEventPostToPid` 鼠标事件 | AppKit 不知 route 去哪个窗口 |
| 只加 f91/f92 + subtype=3 | 同上。windowID 字段 AppKit 新版不消费 |
| 加 `CGEventSetWindowLocation` 但用 `CGEventCreateMouseEvent` 构造 | 12 个 auto-fill 字段缺失，尤其 field 55 windowNumber |
| 加 Command flag 但用 `CGEventCreateMouseEvent` 构造 | 同上 |

## 关键文件索引

- `src/input.rs`
  - `mouse_click_bg(pid, wid, screen, local)` — 最终配方
  - `make_mouse_event_via_nsevent(...)` — NSEvent 工厂封装
  - `cg_event_set_window_location()` — dlsym 懒加载
  - `app_is_active(pid)` — Cmd-flag 条件判断
  - `press_key_combo_bg(pid, keycode, flags)` — 键盘路径（独立配方）
- `src/accessibility.rs`
  - `window_id_for_element(elem)` — 私有 `_AXUIElementGetWindow` wrapper
- `src/main.rs`
  - `ClickStrategy::CgPid` — CLI 分派（click --strategy cg-pid）
  - `PressStrategy::Pid` — CLI 分派（press --strategy pid）

## 未做 / 低优先级

- 把同样的 NSEvent 配方铺到 dblclick、type_text（多字符后台输入）
- 在 SWaveAXRaceDemoApp 本尊上回归验证（如果能拿到源码）
- 跨版本兼容：`CGEventSetWindowLocation` 这个私有符号在更老/更新 macOS 的存在性
