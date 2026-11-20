# RarkZip：自绘标题栏 + 窗口阴影可行性分析

> 技术栈：Rust 2021 / `gpui-kit 0.9.0`（封装 `gpui-pre 0.3.11`，即 Zed `gpui` 的再发布版）/ `gpui-component 0.9.0`
> 分析方式：逐行核对 `~/.cargo` 中 `gpui-pre-linux / gpui-pre-windows / gpui-component` 源码 + 项目现有实现
> 说明：本机 X11 会话缺少可用 GPU adapter（`No GPU adapter found that can configure the display surface`），未能截图实跑，结论均来自源码与调用链核对。

## 1. 结论速览

| 结论项 | 判定 |
| --- | --- |
| 用 CSD（客户端装饰）做自绘标题栏 | **可行，且项目已完成约 80%**，不需要引入 GTK / libdecor |
| 窗口阴影 | **Linux 必须自绘**（WM 不会给 CSD 窗口画阴影）；Windows 必须交给 DWM，自绘反而更差 |
| 最大风险 | X11 无合成器 / WM 不支持 `_GTK_FRAME_EXTENTS` 时回退服务端装饰，出现**双标题栏** |
| 次要风险 | Wayland 阴影区吞点击、窗口尺寸语义含 40px 阴影、圆角裁剪仍是矩形 mask |
| 是否需要换方案 | 不需要。混合策略（Windows=DWM / Linux=CSD 自绘）就是当前最优解 |

一句话：**"CSD header bar" 这条路在 GPUI 上不是"能不能做"，而是"剩下的边角要不要补"**。GPUI 平台层已经提供了做这件事的全部原语：`WindowDecorations::Client`、透明 surface、`set_client_inset`、`set_client_corner_radii`、`start_window_move`、`show_window_menu`、`set_input_region`（仅 Wayland）。

## 2. 现状盘点：代码里已经有什么

| 位置 | 作用 |
| --- | --- |
| `crates/rarkzip-gui/src/main.rs:222-282` | 主窗口/快速窗口创建：`client_decorations = custom_titlebar && linux`，据此选 `Transparent` + `WindowDecorations::Client` |
| `crates/rarkzip-gui/src/app.rs:1868-1935` | 独立功能窗口重复同一套逻辑 |
| `crates/rarkzip-gui/src/app/chrome.rs:180-267` | 自绘标题栏：左侧工具、中间拖拽区、右侧最小化/最大化/关闭 |
| `crates/rarkzip-gui/src/app/chrome.rs:250-258` | 非 Windows 直接 `start_window_move()` + `on_double_click → zoom_window()`（`WindowControlArea` 在 Linux 不参与命中测试） |
| `crates/rarkzip-gui/src/design.rs:1161-1193` | `window_frame()`：给内容裁 12px 圆角，服务端装饰时原样返回 |
| `gpui-component/src/root.rs:735` | `WindowState::decorate` → `window_border().child(surface)`，所有窗口统一加阴影外壳 |
| `gpui-component/src/window_border.rs` | 真正的阴影实现：20px 边距 + BoxShadow + 1px 边框 + resize 热区 + `set_client_inset` |
| `crates/rarkzip-engine/src/settings.rs:124,153` | `use_custom_titlebar` 默认 `true` |

也就是说：**标题栏自绘 + 拖拽 + 双击最大化 + 阴影 + 缩放热区，这套东西已经在跑了**。下面是可行性边界与还差什么。

## 3. 平台通路拆解（调用链级）

### 3.1 Linux / Wayland —— 标准 xdg-decoration CSD

- 窗口创建时 surface 强制透明（`gpui-pre-linux/src/linux/wayland/window.rs:581-589`，`transparent: true`），wgpu 选 `PreMultiplied` 合成（`gpui-pre-wgpu/src/wgpu_renderer.rs:433-446`）。
- 请求 `WindowDecorations::Client` → `zxdg_toplevel_decoration_v1.set_mode(ClientSide)`（`wayland/window.rs:2189-2208`）。
- **合成器有权拒绝**：收到 `Configure(ServerSide)` 会把 `state.decorations` 改回 Server，并触发 `appearance_changed`（`wayland/window.rs:1184-1214`）。这是双标题栏的第二个入口。
- 阴影画在 surface 内部，靠 `xdg_surface.set_window_geometry` 把 20px 边距排除在"窗口几何"之外（`wayland/window.rs:1150-1170`、`1770-1800`），合成器据此做贴边吸附与平铺。
- 不承诺 opaque region（`wayland/window.rs:2461-2477`）——注释写得很直白："客户端装饰包含透明阴影和抗锯齿圆角"。代价是每帧全窗混合。
- 输入区域默认是整块 surface；`set_input_region` 是 **Wayland 专属** API（`gpui-pre/src/window.rs:2726-2738`，X11 未实现，trait 默认空实现见 `platform.rs:1389`）。当前没有任何代码调用它 → 20px 阴影带会吃掉点击。

### 3.2 Linux / X11 —— `_MOTIF_WM_HINTS` + `_GTK_FRAME_EXTENTS`

- 是否支持客户端装饰的判据非常严格：
  `client_side_decorations_supported = compositor_present && gtk_frame_extents_supported`（`x11/client.rs:420-422`）。
- 去边框用 `_MOTIF_WM_HINTS`（`x11/window.rs:1917-1965`）；阴影边距用 `_GTK_FRAME_EXTENTS` 告知 WM（`x11/window.rs:1868-1898`）。
- 透明依赖 ARGB visual + 合成器（`x11/window.rs:487-500`）；渲染器初始 `transparent: false`，随后按 `background_appearance` 更新（`x11/window.rs:296-310`）。
- **回退路径是坑**：没有合成器时 `request_decorations` 直接把 Client 改回 Server（`x11/window.rs:1921-1928`），但应用层仍然画标题栏 → 系统标题栏 + 自绘标题栏叠一起。
- X11 没有 input shape 支持，阴影区吞点击无法用 GPUI 修复（只能靠 WM 的 frame extents 语义，而 X11 的输入形状不在 GPUI 手里）。

### 3.3 Windows —— 保留 DWM，自绘标题栏但不自绘阴影

- `TitlebarOptions { appears_transparent: true }` → `hide_title_bar`（`gpui-pre-windows/src/window.rs:480-484`）。
- `WM_NCCALCSIZE` 里把客户区顶边拉回 `rgrc[0].top`，等于把整个标题栏并入客户区，但**样式位不动**（`events.rs:774-797`）；样式仍保留 `WS_THICKFRAME | WS_MAXIMIZEBOX | WS_MINIMIZEBOX`（`window.rs:496-506`）。
- 因此 DWM 的投影、Win11 圆角、Aero Snap、动画、`HTMAXBUTTON`（Snap Layouts）全部保留；`WM_NCHITTEST` 自己补了边框命中（`events.rs:993-1040`）。
- 结论：**Windows 上不要自绘阴影**。`Decorations::Server` 让 `window_border` 直接透传（`window_border.rs:148-153`），这是对的。

### 3.4 macOS

`traffic_light_position` 已支持，走原生红绿灯，不在本次范围（CI 也只构建 Windows）。

## 4. 方案族对比

| 方案 | 可行性 | 代价 / 结论 |
| --- | --- | --- |
| **A. 纯自绘 CSD（现状）** | 高 | 阴影、圆角、缩放、吸附、窗口菜单、无障碍都得自己兜；GPUI 已提供 80% 原语 |
| B. 服务端装饰 + 顶部工具栏 | 最高（零风险） | 标题栏不可定制，视觉与产品设计割裂；只适合"能用就行" |
| C. **混合：Windows 用 DWM / Linux 用 CSD** | 高（推荐） | 当前架构即是；只需补平台回退判断 |
| D. libdecor / GTK HeaderBar | **不可行** | libdecor 通过 Cairo buffer 合成装饰，需要独占 surface；GPUI 是 wgpu 交换链直写，二者无法共存（除非整窗让给 GTK） |
| E. 让 WM 画 CSD 阴影（`_KDE_NET_WM_SHADOW` 等） | 低 | 仅 X11、仅部分 WM、协议早已废弃；`_GTK_FRAME_EXTENTS` 只是"告知边距"，不是"请求阴影" |
| F. layer-shell | 不适用 | 面板/悬浮层专用，不是普通窗口 |

> 结论：**CSD 自绘是唯一跨平台可控路线**；"header bar" 只是标题栏内部的排版形态，不影响可行性判断。

## 5. 阴影可行性细节（这是最容易翻车的部分）

`gpui-component/src/window_border.rs` 的实现要点与硬约束：

| 项 | 值 / 约束 |
| --- | --- |
| 阴影预算 | 四边各 20px（`SHADOW_SIZE`，`window_border.rs:13-14`），非 Linux 为 0 |
| BoxShadow | `blur 10 + spread -1 + offset y2` 与 `blur 3 + spread 0 + offset y1` 两层（`:244-277`） |
| 为什么不能加大 | GPUI **不会为 blur 扩展 paint bounds**，超出窗口 surface 的部分会被裁掉（源码注释明确说明） |
| 边框 | 1px，非激活窗口整体透明度 0.7（`:242-243`、`:172-186`，颜色硬编码 0.2/0.8 亮度） |
| 圆角 | `BORDER_RADIUS = 0` —— GPUI 裁剪 mask 是矩形，圆角会让子元素背景在四角露出（`:19-22`） |
| 平铺/最大化 | 四边都 tiling 时可视阴影归零，但 `set_client_inset` 仍传 20px，避免还原时尺寸跳变（`:158-169`） |
| 缩放热区 | 只有内框 ±4px 的窄带（`RESIZE_HIT_SIZE`，`:17`、`:322-422`），阴影带不响应 |

可选的观感增强：Wayland 下可把 `window_background` 设为 `Blurred`，GPUI 会走 `ext-background-effect-v1`（KDE / 较新合成器）或 `kde-blur`（`wayland/window.rs:2478-2510`），得到类似 Win11 Mica 的毛玻璃，但**不能被当作阴影的替代品**。

## 6. 风险清单（按严重度）

### P0 双标题栏（X11 无合成器 / WM 不认 `_GTK_FRAME_EXTENTS`；Wayland 合成器强制 ServerSide）

现象：系统标题栏 + 应用标题栏同时存在，且 20px 阴影边距变成实心边。

修复：把"是否渲染应用标题栏"与真实装饰状态绑定，而不是只看设置：

```rust
// 在 chrome.rs / app.rs 渲染入口处
let client_frame = matches!(window.window_decorations(), Decorations::Client { .. });
let show_app_titlebar = self.custom_titlebar && (cfg!(windows) || client_frame);
```

顺带在启动时打一条 `log::info!` 记录 `window_decorations()`，便于用户反馈时定位。

### P1 阴影区吞点击（Wayland）

20px 透明边距仍然接收输入，用户点阴影等于点窗口。GPUI 有 `set_input_region`：

```rust
if matches!(window.window_decorations(), Decorations::Client { .. }) {
    let inset = window.client_inset().unwrap_or(px(0.0));
    let full = window.window_bounds().get_bounds().size;
    let hit = Bounds {
        origin: point(inset, inset),
        size: size(full.width - inset * 2.0, full.height - inset * 2.0),
    };
    window.set_input_region(Some(&[hit]));
}
```

注意：坐标是 surface 坐标（含阴影），且需在 resize 后重设；X11 上该调用是空操作，不要指望它解决 X11。

### P1 窗口尺寸语义偏移

`window_bounds()` 返回**含 40px 阴影的外框**（X11 `x11/window.rs:1399-1406`；Wayland `wayland/window.rs:1730-1740`），而 `inner_window_bounds()` 已扣除 inset（X11 `:1408-1434`、Wayland `:1742-1754`）。当前：

- `window_min_size: 520x420` 实际内容只有 `480x380`；
- `save_window_state()` 存的是外框尺寸，若用户之后关掉自绘标题栏，窗口视觉上会"变大" 40px。

建议：创建时按 `+2 * SHADOW_SIZE` 补偿，持久化改用 `inner_window_bounds()`。

### P2 缩放体验

阴影带 16px 不响应 resize，只有内框 4px 窄带响应，边角偏难点中。建议把 `resize_hit_size` 调到 6-8px，或在阴影带上也做 edge 判定（`window_border.rs::resize_edge` 已具备该逻辑，只是入参是内框）。

### P2 标题栏右键语义

当前右键弹应用自己的菜单（`chrome.rs:237-249`）。Linux 桌面惯例是系统窗口菜单（移动/置顶/关闭/工作区）。GPUI 已实现：

```rust
window.show_window_menu(event.position); // Wayland: toplevel.show_window_menu(serial)
```

（`gpui-pre/src/window.rs:3219`；Wayland `:2085-2096`；X11 `:1796`）。建议 Linux 上用系统菜单，Windows 保留应用菜单。

### P3 其他

- **圆角裁切**：12px 圆角只作用于内容容器，子元素背景仍会在四角露出小三角；要么等 GPUI 支持圆角 mask，要么在四角叠一层同底色圆角遮罩。
- **无障碍**：自绘的最小化/最大化/关闭需要 `aria-label` 与 Tab 焦点；Windows 的 Snap Layouts 靠 `WindowControlArea::Max`，Linux 无等价物。
- **主题耦合**：边框色写死在 `window_border.rs:172-186`，深浅主题切换时可能偏重；建议改为跟随 `palette.line`。
- **HiDPI**：inset 以逻辑像素下发、X11 内部再乘 `scale_factor`（`x11/window.rs:1868-1878`），1.25/1.5 缩放下 1px 边框可能出现半像素抖动。
- **性能**：CSD 下没有 opaque region，合成器整窗混合；4K 全屏 + 软合成环境要注意。

## 7. 建议路线

| 阶段 | 内容 | 说明 |
| --- | --- | --- |
| 1. 止血 | P0 装饰状态绑定、P1 input region、P1 `inner_window_bounds` | 改动集中在 `chrome.rs` / `window_frame` 调用点与 `save_window_state`，风险低 |
| 2. 打磨 | 窗口菜单、resize 热区、边框/阴影 token 化、四角遮罩 | 纯 UI 层，可增量验证 |
| 3. 增强（可选） | `Blurred` 背景、非激活窗口阴影弱化策略、平铺时切回 Opaque | 依赖合成器能力，需特性探测 |
| 4. 测试矩阵 | GNOME Wayland、KDE Wayland、Sway（无 decoration 协议）、X11+picom、X11 无合成器、Win10/11 100%/150%/200%、最大化/平铺/全屏/多显示器 | X11 无合成器与 Sway 是最容易暴露问题的两个场景 |

## 8. 直接回答"要不要换方案"

1. **不要** 引入 libdecor / GTK HeaderBar：与 wgpu surface 模型冲突，收益只有"省掉自己画阴影"。
2. **不要** 试图让窗口管理器替 CSD 画阴影：协议层没有这个能力，X11 的 `_GTK_FRAME_EXTENTS` 只是几何提示。
3. Windows 侧保持"隐藏 caption + 保留 WS_THICKFRAME"，让 DWM 提供阴影/圆角/Snap；自绘阴影在 Windows 上只会更差（失焦、DPI 迁移、圆角不同步）。
4. Linux 侧 CSD 自绘是正确且唯一可控的路线；当前实现已经覆盖 80%，剩下的是**回退判定 + 输入区域 + 尺寸语义**这三件实事。
5. 如果团队不想承担 CSD 的长期维护成本（每个 DE 的观感差异、无障碍、缩放细节），唯一"零风险"退路是方案 B：服务端装饰 + 顶部工具栏，放弃自绘标题栏——但那就不是"自定义标题栏"了。

## 附：关键 API 速查

| API | 平台 | 用途 |
| --- | --- | --- |
| `Window::window_decorations()` | 全平台 | 判断当前是 Client 还是 Server，渲染分支的唯一可信来源 |
| `Window::set_client_inset(px)` | X11/Wayland | 告知平台阴影边距（X11 写 `_GTK_FRAME_EXTENTS`，Wayland 影响 `set_window_geometry`） |
| `Window::client_inset()` | 全平台 | 读回边距，用于布局/输入区域 |
| `Window::inner_window_bounds()` | X11/Wayland | 去掉阴影的真实窗口区域，用于持久化 |
| `Window::set_input_region(&[Bounds])` | **仅 Wayland** | 让阴影边距点击穿透 |
| `Window::start_window_move()` | 全平台 | 标题栏拖拽 |
| `Window::zoom_window()` / `minimize_window()` / `remove_window()` | 全平台 | 窗口控制 |
| `Window::show_window_menu(pos)` | X11/Wayland | 系统窗口菜单 |
| `Window::set_client_corner_radii()` | Wayland | 仅供模糊区域使用，不做裁剪 |
| `WindowBackgroundAppearance::{Transparent, Blurred}` | 全平台 | CSD 透明 / 合成器模糊 |

## 9. 已落地改动（本轮实现）

针对第 6 节里判定为"必须做"的三项，代码已改完并通过 `cargo clippy -p rarkzip-gui`：

| 项 | 改动位置 | 做法 |
| --- | --- | --- |
| P0 双标题栏 | `design::uses_own_titlebar()`；`app.rs::frame_window`、`app.rs` 功能窗口渲染、`app/chrome.rs` 工具栏、`quick.rs::render` | 渲染分支改看 `window.window_decorations()` 的实际结果：Linux 上只有真拿到 `Decorations::Client` 才画自绘标题栏与窗口按钮，回退 `Server` 时整套控件（含右上角三键、拖拽区）交还系统 |
| P1 阴影区吞点击 | `design::InputRegionSync`；`MainView`/`QuickView` 新增字段，渲染时 `sync()` | 以 `window_paddings()` 为内框、向外各扩 4px 缩放命中带，调 `set_input_region`；按 `(x,y,w,h)` 缓存，只在尺寸变化时提交，避免每帧一次 surface commit |
| P1 尺寸语义 | `design::outer_size` / `outer_size_for` / `is_client_decorated`；`main.rs` 与 `app.rs` 的两处 `open_window`、`app/impls.rs::toggle_new_advanced`、`quick.rs::resize_for_step`、三处持久化函数 | 统一内容尺寸与外框尺寸的换算：创建与 `Window::resize` 都补 `2×20px`；持久化改用 `inner_window_bounds()`。是否补边距看运行时 `window_decorations()`，合成器拒绝 CSD 时不补 |
| 平台判定收敛 | `design::wants_client_decorations()`；`main.rs:226` | 三处重复的 `cfg!(linux/freebsd)` 判断合并成一个函数，避免以后改漏 |

几点实现说明：

- **不要把 `set_input_region` 当 X11 的解法**。它只在 Wayland 有实现（`x11/window.rs` 里是空函数），X11 的阴影带点击仍然落在窗口上；要彻底解决只能在 X11 侧另做输入形状（`XShapeCombineRectangles`），代价大于收益，暂不做。
- **平铺/最大化**依赖 `window_paddings()` 自动归零（`gpui-component` 在 `Decorations::Client { tiling }` 里处理），所以输入区域在最大化时会自然退化为整窗，不需要额外分支。
- **`Window::resize` 的语义容易被忽略**：它写的是 `state.bounds.size`，即**外框**（X11 `x11/window.rs:1439-1458`、Wayland `set_size_and_scale`），不是文档所说的 content size。快速窗口的步骤切换与「高级选项」展开都走这条路径，所以也一并换算，否则每切一步窗口就会缩 40px。
- **顺序有讲究**：`sync()` 必须在 `window_decorations()` 之后、绘制之前调用，且只在 Client 时生效，否则会把系统装饰窗口裁成一条窄区。
- **标题栏右键暂不改系统菜单**。现有的应用菜单（打开 / 新建 / 设置 / 最大化 / 关闭）信息密度高于系统窗口菜单，换成 `show_window_menu` 是净损失；`Window::show_window_menu` 已确认在 X11 与 Wayland 都有实现，若以后要做"设置里可选系统菜单"可以直接接上。
- **`resize_hit_size` 仍是 4px**。该常量在 `gpui-component` 内部（`window_border.rs:17`），改它得 fork 依赖；折中办法是让输入区域外扩 4px（已做），可视命中带仍是内框 ±4px，触控板用户可能觉得紧，等上游放宽参数。
- **圆角问题已在上游解决**（`gpui-component 0.9.1`），细节见第 10 节；本节的 `design::window_frame` 已随之删除。

### 验证方式

```bash
cargo clippy -p rarkzip-gui     # 静态检查（已通过）
cargo run -p rarkzip-gui        # 真机观感验证
```

需要人工确认的场景（本机 GPU adapter 不可用，未做截图验证）：

1. **GNOME Wayland**：标题栏自绘 + 阴影可见 + 阴影带点击穿透；
2. **Sway（不实现 xdg-decoration）**：应自动回退为系统标题栏，且只剩一条标题栏；
3. **X11 + picom**：阴影可见，但阴影带点击仍会命中窗口（已知限制）；
4. **X11 无合成器**：回退 `Server`，不得出现双标题栏；
5. **最大化 / 平铺 / 全屏**：圆角与阴影边距归零，输入区域等于整窗；
6. **关闭再启动**：窗口尺寸与关闭前内容尺寸一致，不出现每启动一次"长大 40px"。

## 10. 窗口圆角（本轮实现）

### 结论

`gpui-component 0.9.0` 里 `BORDER_RADIUS` 被写死为 0（注释说明 GPUI 只有矩形裁剪），所以圆角只能靠应用自己在内容层叠遮罩——而遮罩需要"擦除"，GPUI 没有 `destination-out`，做不出来。**0.9.1 把这个能力补上了**，且实现方式与第 9 节的三处修复完全对齐。

### 上游改了什么（`gpui-component 0.9.1`）

| 变化 | 作用 |
| --- | --- |
| `Theme::window_radius`（默认 `px(0)`） | 窗口圆角成为主题 token；`px(0)` 时行为与旧版完全一致 |
| `window_corner_radii(window, cx)` | 按 `window_radius` + `window_decorations()` 的实际 `tiling` 计算四角，平铺边自动归 0，Server 装饰全 0 |
| `WindowBorder` 内部 | 边框、透明背板、阴影孔洞全部改用该半径，并调用 `set_client_corner_radii()` 告知合成器 |
| 边框色 | 从写死的 `l = 0.8 / 0.2` 改为主题 token `colors.window_border`（回退到 `colors.border`），跟随 `palette.line` |

### 应用侧改了什么

1. **依赖升级**：`gpui-kit 0.9.0 → 0.9.1`、`gpui-component 0.9.0 → 0.9.1`、`gpui-pre 0.3.11 → 0.3.12`。都落在原有 caret 约束内，业务代码零改动即编译通过。
2. **主题**：`theme::apply` 里 `theme.window_radius = px(design::WINDOW_CORNER_RADIUS)`（12px）。
3. **`design::WindowCorners` trait**：三个方法 `round_window_corners` / `round_window_top` / `round_window_bottom`，内部统一取 `window_corner_radii`，服务端装饰时半径全 0，因此可以无条件调用。
4. **贴边元素全部自绘圆角**（这是上游文档明确要求的）：窗口根节点、顶部工具栏/标题栏、底部状态栏、模态遮罩、页面外壳（`palette.page` 整幅背景）、页面底部操作栏、快速窗口的两个页脚。任何新增的整幅背景元素只要贴到窗口边缘，都必须挂上对应方法。
5. **删除 `design::window_frame`**：旧实现给内容套一层 12px 圆角的 `overflow_hidden`，但矩形裁剪并不能裁子元素，属于"看起来做了、实际没做"，现在由上游统一处理。

### 关键坑（写下来给以后的人）

- **只给根节点圆角不够**：GPUI 的裁剪只有矩形，任何自带背景又贴边的东西都会把四角填回直角。判断标准是"这个元素的背景是否覆盖了窗口角"，而不是"它是不是根节点"。
- **`page_shell` 也必须参与**：它的 `palette.page` 是整幅背景，在独立窗口里直接贴住下沿；主窗口里它夹在工具栏与状态栏之间，圆角处背后同色，不会出现缺口。为此把 `window`/`cx` 透传进了 `page_shell` / `page_shell_bare`。
- **`Theme::update` 要在 `Theme::change` 之后**：`change` 会把主题重置为预设，`window_radius` 必须写在 `update` 里，否则会被打回 0。
- **平铺/最大化**：`window_corner_radii` 已按 `tiling` 把对应角归零，应用侧不用再判断。

### 验证

- `cargo clippy -p rarkzip-gui`：无警告。
- `cargo build -p rarkzip-gui`：通过（首次全量重编 gpui 约 7 分钟）。
- **仍无截图证据**：本机 `DISPLAY=:0` 下启动仍是 `No GPU adapter found that can configure the display surface`（mesa 无可用 adapter，软渲染同样起不来），圆角的实际观感需要在有 GPU 的机器上确认。

需要人工确认：四角是否干净（无 1px 直角残影）、最大化后圆角消失、模态打开时遮罩四角是否跟着圆、独立窗口底部操作栏两角是否正确。
