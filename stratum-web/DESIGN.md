# Design System: 运筹 Stratum

## 1. Visual Theme & Atmosphere

运筹是一套克制、可信、偏工程工作台气质的 Agent 产品界面。整体密度为 Daily App Balanced（6/10），布局变化度为 Offset Asymmetric（6/10），动效强度为 Fluid CSS + Focused GSAP（5/10）。页面保留空气感，但工作区的信息层级必须明确；视觉重心来自留白、细分隔线和单一低饱和蓝色，而不是大圆角、霓虹光或大量悬浮卡片。

当前首页 Hero 是未来 Dashboard 的临时占位，保持现有结构，不在本阶段重做。Chat 工作区是独立的 `Longzhong` 路由；固定导航通过左右过渡在两个页面间切换，不在 Chat 区重复显示“工作台”标题。

## 2. Color Palette & Roles

- **Cloud Canvas** (`#F3F2EE`) — 浅色主背景，承载 Hero 与 Chat 工作区。
- **Paper Surface** (`#FAF9F5`) — 历史会话卡片与输入框表面。
- **Charcoal Ink** (`#2B3033`) — 主文本；禁止使用纯黑 `#000000`。
- **Muted Steel** (`#70767A`) — 辅助文案、时间和未选中导航。
- **Whisper Border** (`rgba(75, 113, 139, 0.16)`) — 1px 结构线、卡片边框和分隔线。
- **Baltic Blue** (`#4B718B`) — 唯一功能强调色，用于主按钮、焦点环和导航选中标识；不使用外发光。
- **Night Canvas** (`#20272B`) — 深色主背景。
- **Night Surface** (`#30373A`) — 深色卡片与输入区域。
- **Warm White Ink** (`#EFEDE6`) — 深色主文本。

品牌图形可以保留既有多色细线，但这些颜色不得扩散为界面语义色。界面中只有 Baltic Blue 是可交互强调色。

## 3. Typography Rules

- **Display / 中文标题：** `Noto Sans Variable`，紧凑字距，使用字重和留白建立层级，不使用夸张超大字号。
- **Body / UI：** `Nunito Sans Variable`，正文最大宽度 `65ch`，行高保持宽松。
- **Metadata：** 继续使用无衬线字体；只有未来出现高密度运行编号或时间序列时才引入 `Geist Mono`。
- **Dashboard 约束：** 全部使用无衬线字体。禁止 Inter 和通用衬线字体。
- **最小正文：** `14px`；主要对话正文在桌面与移动端均不低于 `16px`。

## 4. Component Stylings

- **Navigation：** 固定定位、无玻璃拟态。桌面导航项为“概览 / 隆中对”。当前项使用文字加深与底部 `2px` Baltic Blue 短线，不增加胶囊底色。
- **Active Indicator：** 一个共享元素在导航项之间移动。GSAP 只动画 `transform` 与 `scaleX`，时长 `0.28s`，`power2.out`；减少动态效果时立即定位。点击导航和滚动进入区段都必须同步选中态。
- **Cards：** 仅历史会话 drawer/overlay 和对话输入区使用卡片。圆角 `12px`，1px Whisper Border，阴影极轻。中央对话流没有外层卡片。
- **Buttons：** 平面填充或描边，无外发光；按下时仅 `translateY(1px)`。触控目标至少 `44px`。
- **Composer：** 标签或提示文案位于输入区上方/内部，错误信息位于下方。输入区固定在中央列底部，圆角 `12px`，不得使用巨型胶囊。Composer 只使用一层表面，不得在 `Card` 内再次嵌套输入容器；桌面端与导航共享响应式内容宽度，最大宽度约 `896px`，常态高度约 `116px`。正文输入占据上层主空间，Agent、Model 与状态操作收敛到约 `44px` 的底栏，发送按钮固定为 `44px` 圆形主操作。新对话与已有对话保持相同尺寸，只改变垂直位置；新对话必须按组件自身中心精确垂直居中。移动端使用视口减 `24px` 的可用宽度，Agent 与 Model 触发器允许收缩和省略，发送按钮不得被挤压或遮挡。
- **Conversation Rows：** 助手消息直接落在页面背景上；用户消息只允许使用轻微 Surface 色块区分，不再嵌套卡片。
- **Empty State：** 使用真实结构展示如何开始一轮对话，不显示孤立的“No data”。

## 5. Layout Principles

### Routes

1. **Overview (`/`)：** 当前 Hero / 未来 Dashboard 占位，`min-height: 100dvh`。
2. **Longzhong Chat (`/longzhong`)：** 独立的真实 Agent 对话工作区，顶部和底部为固定导航与 Composer 留出安全空间。

两个页面是独立 route，不放进共享纵向滚动轨道，也不使用 `#overview` / `#longzhong`
anchor 互相跳转。

### Navigation

- 左侧品牌保持“运筹”。
- 右侧顺序为“概览、隆中对、分隔线、语言、主题、注册”。
- “概览”对应 `/`，“隆中对”对应 `/longzhong`。
- 导航选中态已经表达页面位置，因此 Chat 区不再渲染“隆中对 Chat 工作台”标题。
- 两个 route 通过有方向的 view transition 切换；导航 tab 不触发页内滚动。

### Chat workspace

- `/longzhong` 始终是单个居中的主列。桌面端只调整主列左右 gutter，不增加永久侧栏。
- 历史会话通过可切换 overlay / drawer 打开，不进入主布局流。宽屏 history trigger 可作为导航左侧的 detached pill，drawer 向左下展开并保持视口安全边距。
- Composer 与导航共享最大约 `896px` 的响应式外宽；对话正文在其中保持最大约 `800px` 的阅读宽度并居中。对话流直接渲染在背景上。
- 消息使用 document scroll，不创建内部消息 scroller。固定 Composer 不得遮挡最后一条消息。
- Navbar 和 Composer 的 viewport top/bottom offset 由各自最外层 fixed container 的 margin 表达，不把外部间距藏在内部 padding 或定位偏移中。
- Composer 左侧相邻显示 Agent 和 Model 选择。新会话默认第一个模板；切换 Agent 会进入新的未创建会话并恢复该模板默认模型。创建前的模型配置随创建请求提交，已有会话的模型配置用于下一条消息。
- 工作区连接真实 Agent API：投影流式文本、reasoning、tool trace、durable messages、终态、恢复和审批。思考与工具细节默认折叠，按需展开。
- 审批 UI 只能描述 event 明确携带的 tool name、arguments、kind、danger level 等事实；后端未提供时不得编造原因、影响、风险结论或可逆性建议。
- 不渲染第三列，也不为未来事件栏留空白占位。

### Responsive behavior

- `< 768px` 时切换为单列，无横向滚动。
- 历史会话仍是 overlay / drawer，不移动到对话流上方成为永久内容。
- 输入区保持可见，但不得遮挡最后一条消息。
- Agent / Model trigger 允许收缩和省略；发送按钮、历史入口、语言、主题和注册控件仍保持可用且满足最小触控目标。

## 6. Motion & Interaction

- 只在状态反馈确实需要时使用现有 GSAP，不增加新的动画依赖；当前独立 route 导航不使用 `ScrollTrigger`。
- route 切换使用有方向的 view transition；导航指示器只反映当前 pathname，不使用 ScrollTrigger 监听另一个 route 的区段。
- 若导航指示器使用 GSAP，只动画 `transform` 与 `scaleX`；禁止动画 `left`、`width`、`top` 或 `height`。
- 历史 drawer 和列表首次出现可使用 `opacity + translateY` 的轻量级过渡，单项间隔不超过 `40ms`；关闭时必须恢复触发器焦点。
- 对话仅在用户位于真实底部时自动跟随新增内容。用户向上滚动后必须保持当前阅读位置，推理完成、推理内容展开或消息高度变化不得重新锁定到底部；仅当用户主动滚回底部或点击“滚动到底部”时恢复跟随。“滚动到底部”按钮与固定 Composer 顶部保持约 12px 间距，不得遮挡输入表面。
- `prefers-reduced-motion: reduce` 下取消 route、drawer、列表和指示器过渡，直接切换最终状态。

## 7. Anti-Patterns (Banned)

- 不使用 emoji、Inter、通用衬线字体或纯黑。
- 不使用紫色/蓝色霓虹、外发光或高饱和渐变。
- 不把中央对话流包进大卡片，也不做卡片套卡片。
- 不把 history 变成永久左/右 rail，也不让它占据主列布局空间。
- 不创建空的右侧事件栏占位。
- 不使用三等分卡片布局。
- 不重做当前 Hero；它是未来 Dashboard 的占位。
- 不引入毛玻璃或 backdrop-filter 效果。
- 不伪造静态历史、工具状态、审批解释或后端成功状态；所有运行数据来自 Agent API/事件投影。
- 不显示“滚动探索”、箭头提示或其他填充型导航文案。
- 不使用绝对定位堆叠正文内容；每个区域都占据清晰的 Grid 空间。
