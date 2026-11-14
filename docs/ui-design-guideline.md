# QookiX Launcher UI 统一设计方案

> 适用范围：`src/views`、`src/components`、`src/styles.css`、`src/theme.ts` 全部前端界面。
> 目标：让任何协作者在不读别人代码的情况下，也能写出视觉一致的页面。
> 本文只是规范，不包含代码改动。

> **落地进度（2026-11）**
> §13.1 的令牌地基与 §13.2 的共享原子类已经落地，并按职责拆成三份文件：
> `src/styles/tokens.css`（令牌）、`src/styles/base.css`（重置 / 外壳 / 转场）、
> `src/styles/components.css`（`.qx-*` 组件类）。
> `src/styles.css` 现在只是入口 + 第三方组件覆写。
> 新页面请直接引用 `.qx-*`，不要再复制一份 `.btn` / `.card` / `.mini-btn`。
> 自定义主页（`home.json`）见 [custom-home.md](./custom-home.md)。
>
> 已统一到 `.qx-*`：首页 / 工具箱 / 实例页骨架；联机、实例详情、内容详情、创建实例、
> 文件管理器、实例设置、实例页的按钮；实例详情 / NBT / 下载抽屉 / 实例内容页签的空状态。
> 已知仍待收口：`mini-btn` 在 17 个文件里有 12 份略不相同的定义（约 219 处引用，
> 且「primary」在设置页是描边而非填充，需要先定语义再改名）；`BrowseView` 的吸附头
> 与页面标题存在布局耦合，需开窗口实测。

---

> **阅读顺序提示（2026-11-14）**：§3 令牌、§5 组件、§6 网格是**现行**约束；
> §4 已按实际判断重写（**不设宽度上限、不强制页面标题**）；
> §10 / §13 / §14 是早期施工计划，其中「统一 1120 宽度」「每页补标题」的条款**已作废**，
> 仅作为改造过程记录保留，不要照着做。

## 0. 一句话结论

项目已经有一套很好的令牌底座（`src/styles.css` + `src/theme.ts`），问题不在"没有设计系统"，而在**令牌没有被强制使用**：每个页面都在令牌之上自行发明了尺寸、圆角和结构。本规范的核心任务不是重新设计外观，而是**把自由度收敛到 6 个档位以内**。

---

## 1. 现状诊断：割裂的 15 个根因

| # | 问题 | 取证 |
|---|---|---|
| 1 | ~~**页面骨架不统一**~~ | 当时的判断是「有的页有上限、有的没有」。**后来推翻**：上限本身就是错的解法，全站已统一为「都不设上限」（见 §4） |
| 2 | **页面标题层级混乱** | InstanceDetail `h1 21px`、ServerDetail `h2 20px`、ProjectDetail ≈20px；而 Browse / Instances / Toolbox / Multiplayer 干脆**没有标题** |
| 3 | **按钮没有尺寸分级** | 主按钮 `9px 18px / 10px / 13px`、`mini-btn 7px 13px / 8px / 12px`、`.btn.sm 6px 12px`、`.btn.big 13px 26px`、标题栏 `28px 高 / 8px / 12px`、`icon-btn 32×32`、`pin-unpin 28×28`、`tb-btn 30×30` 圆形、`ctrl-btn 32×32` —— 至少 8 套 |
| 4 | **圆角失控** | 实际出现 3/4/5/6/7/8/9/10/12/14/16/18/999px 共 13 种；同一页面里 `.glass 14` 与 `.map-info 10`、`.id-ver-list 10` 并存 |
| 5 | **卡片网格列宽随意** | `minmax` 用了 88/104/200/210/240/280/300/330/340 九种值，同一视觉层级的卡片在不同页面大小不一 |
| 6 | **弹窗宽度无规范** | 380/420/440/460/480/520/560/580/600/620/640/680/700/720/780px，`max-width` 混用 92vw/94vw/95vw |
| 7 | **指示器各写一遍** | `useSlidingIndicator.ts` 已封装，但 Browse / Multiplayer / ServerDetail / InstanceDetail / DownloadsDrawer 仍手写 indicator，padding 2/4/5/6px、圆角 6/7/9px 各不相同 |
| 8 | **硬编码颜色与全局样式泄漏** | `#e5534b`、`rgba(232,163,61,.35)`、`#e8a04b/#7aa2f7/#b48ead/#9ece6a`、`#6aa9ff`、`#7f77dd`、`#9fe1cb`、`rgba(255,255,255,.16~.18)` 等散落各处；**6 个**组件使用非 scoped 全局 `<style>`：`InstallDialog`、`AddAccountDialog`、`IconPickerDialog`、`InstallChecklist`、`VersionPicker`、`AccountChip`（第二个 `<style>` 块） |
| 9 | **出现小数字号** | `10.5px`、`11.5px`、`12.5px`、`13.5px` 共 16 处，横跨 TimeMachine、Settings、ServerDetail、ContextMenu、FileManager、ServerFileManager、CrashDialog |
| 10 | **z-index 无层级体系** | 实测值：`0/1/2/5/30/40/44/45/50/60/90/95/100/900/1200/1400/2000/2500/3000/9999` 共 20 种，其中 `2000` 被 ColorPicker / SkinView / NbtTreeEditor 三处独立占用，`3000` 被 ImageViewer / TimeMachineLightbox 占用，`9999` 被 SplashScreen / LaunchProgress 占用 |
| 11 | **同类控件的变体数量失控** | `.mini-btn` 有 **7 套**：`6px 12px/8px`、`7px 13px/8px`、`8px 12px/9px`、`8px 14px`、`4px 10px`、`5px 9px`（BackupPanel）、`4px 8px/radius 7`（ChunkMap）；`.seg` 有 **6 套**（TimeMachine `radius 9/indicator 7`、Skin `radius 8/indicator 6`、ServerDetail `padding 5/indicator 9`、ChunkEditor `容器 9/项 7`、RoomDirectory `.rd-subtabs 10/8`、SavesTab 名为 `.seg` 实为 pill 按钮 `radius 999`），高度 30–38px 不等 |
| 12 | **幽灵令牌（引用了不存在的变量）** | `--bg-3`（OnboardingBar 3 处、**无 fallback** → 背景失效）、`--surface-2`（ItemEditor 2 处、NbtTreeEditor、PlayerEditor、EnchantEditor，**无 fallback** → 输入框/槽位无底色）、`--surface-1`、`--panel-2`、`--panel-sub`、`--text-main`、`--card-bg`、`--radius-lg`、`--radius-md`、`--border-strong`、`--glass-blur`（共 11 个在 `styles.css` 中**均无定义**）。这些组件在作者的本机"看起来正常"，换主题/换人维护立刻露馅 |
| 13 | **同一行为的两个"双胞胎"文件已分叉** | `FileManager.vue` 与 `ServerFileManager.vue` 的样式几乎逐行复制，但容器高度一个 `calc(100vh - 320px)`、一个固定 `480px`；侧栏导航圆角 7、行圆角 9 也只存在于这两处 |
| 14 | **抽屉规格不一致** | `AccountDrawer` 宽 **380px**、`DownloadsDrawer` 宽 **420px**；两者遮罩均为 `rgba(0,0,0,.18)` 且无模糊，与弹窗遮罩 `rgba(0,0,0,.45)+blur(6px)` 完全不同 |
| 15 | **hover / 按下行为与"克制"原则相悖** | 卡片 hover 位移：`RoomDirectoryPanel .rd-room:hover { translateY(-2px) }`；色板 hover `scale(1.15)`；主按钮 17 处用 `filter: brightness()` 提亮；`FriendsPanel / QookixLoginDialog` 主按钮按下 `scale(.97)`。同一应用内出现"位移 / 缩放 / 提亮"三种反馈语义 |

---

## 2. 设计原则

1. **令牌优先（Token First）**：任何颜色 / 圆角 / 间距 / 字号，先问"有没有令牌"，没有就提 PR 加令牌，**不允许就地写死数值**。
2. **档位制（Steps, not values）**：尺寸只允许存在于预定义档位中（4 档按钮、6 档圆角、7 档间距）。不做"13px 和 14px 都行"的暧昧选择。
3. **容器先行（Skeleton First）**：新页面先套用统一骨架（标题区 + 工具条 + 内容区），再写业务，禁止直接铺 grid。
4. **静默克制（Quiet Motion）**：动效只用于状态变化和空间连续性，禁止装饰性动画；卡片 hover 只加深阴影，不位移、不缩放。
5. **单一实现（One Way）**：同类交互只有一种实现路径（例如所有切换器必须用 `useSlidingIndicator`），不允许"我这里手写一个也行"。

---

## 3. 设计令牌（Design Tokens）

### 3.1 颜色

保留现有 `--accent` 体系（`#e89a4b` 系列），补齐缺失语义。**所有页面只允许引用下列变量。**

| 类别 | 令牌 | 用途 |
|---|---|---|
| 强调 | `--accent` / `--accent-deep` / `--accent-hover` | 主按钮渐变、激活态 |
| 强调透明 | `--accent-06 … --accent-60` | 所有强调底色 / 描边，**禁止 `rgba()` 手写橙色** |
| 背景 | `--bg-0` / `--bg-1` / `--bg-2` | 窗口底 / 面板底 / 浮层底 |
| 面板 | `--panel` / `--panel-hover` / `--border` | 玻璃卡片 |
| 文本 | `--text-1` / `--text-2` / `--text-3` | 标题 / 正文 / 次要 |
| 叠加 | `--w-02 … --w-20` | 深浅主题自适应的浅色叠加（唯一合法的"半透明白"） |
| 语义 | `--danger` `--success` `--info` `--warn` + `--danger-text` 等 | 状态 |
| 语义底 | **新增** `--danger-bg` `--success-bg` `--info-bg` `--warn-bg`（对应色 12%~16% 叠加） | 统一徽章 / 警告条底色 |

**硬编码黑名单（必须替换）**

| 现状 | 替换为 |
|---|---|
| `#e5534b`（TitleBar 关闭、CloudSync） | `var(--danger)` / `var(--danger-bg)` |
| `rgba(232,163,61,.14/.35)`、`rgba(224,168,90,.18/.4)` | `var(--warn-bg)` / `var(--warn)` |
| `rgba(90,176,255,.15)`、`rgba(122,208,138,.15)` | `var(--info-bg)` / `var(--success-bg)` |
| `#e8a04b/#7aa2f7/#b48ead/#9ece6a`（loader 色） | 语义色变量 |
| `rgba(22,25,34,.68)`（Browse 吸附底衬） | **新增** `--head-glass`，深浅主题各一份 |
| `#6aa9ff`（TimeMachine 云端快照） | `var(--info)` |
| `#7f77dd` + `rgba(127,119,221,.16/.45)`（TimeMachine purple chip） | 新增 `--purple` / `--purple-bg` |
| `#9fe1cb`、`rgba(60,180,60,.2)` | `var(--success)` / `--success-bg` |
| `rgba(255,255,255,.16/.18/.85)`（封面标签、漫游提示） | 封面蒙层场景允许，但必须集中为 `--on-media-*` 令牌，禁止散落 |
| `rgba(229,83,75,.88)`、`rgba(232,154,75,.92)`（hero chip） | `var(--danger)` / `var(--accent)` + 统一 `--on-media` 透明度 |

> 唯一允许保留的例外：猫主题配色方案（皮肤层），必须通过覆盖 `--accent` 系列实现，不得新增独立色值。

### 3.2 字体

字体族沿用现有 `"Segoe UI", "Microsoft YaHei", "PingFang SC", system-ui`，不修改。字号收敛为 **8 档**：

| 令牌 | 值 | 用途 |
|---|---|---|
| `--fs-2xs` | 11px | 角标、分组小标题、badge |
| `--fs-xs` | 12px | meta 信息、说明文字、小按钮 |
| `--fs-sm` | 13px | **默认正文 / 按钮 / 表单 / 标签页** |
| `--fs-base` | 14px | 面板标题、弹窗正文、卡片名（2026-11 补：全仓 65 处 14px 的无名档） |
| `--fs-md` | 15px | 卡片标题、区块标题 |
| `--fs-lg` | 20px | 页面主标题（全局唯一） |
| `--fs-xl` | 26px | 大数字展示（验证码、统计值） |
| `--mono` | Cascadia Code | 路径、版本号、代码块 |

**删除** 16 / 17 / 21px 三处游离字号，以及全部小数字号 `10.5 / 11.5 / 12.5 / 13.5px`（实测 16 处）。字重只保留 **400 / 500 / 600 / 700**，禁止 800。

> 14px 原来是"删除项"，但它是全仓出现最多的字号（65 处，面板标题 / 弹窗正文 / 卡片名都在用），删不掉。与其让它散落在各页，不如正式收编为 `--fs-base`，字号档位变 8 档 —— 这是唯一一处对早期规范的推翻，理由是「规范要跟着真实密度走」。

> `font-size` 也常被用来给 svg 图标定尺寸（`font-size: 21px` 的图标容器、`font-size: 14px` 的 `.mini-btn svg`）。这类值不要当字号处理，收编时一律跳过。

> 2026-11 已完成一轮机械收编：`<style>` 块里的字号字面量从 1001 处降到 59 处（94% 走令牌），圆角从 482 处降到 1 处。剩下的 18 / 16 / 22 / 34 / 36 / 46px 多为图标尺寸与装饰性大数字，**不要再强行并入 8 档**。

行高：标题 `1.3`，正文 `1.55`，等宽 `1.5`。

### 3.3 间距

只允许 **4 / 8 / 12 / 16 / 20 / 24 / 32** 七档。删除 2 / 6 / 7 / 9 / 10 / 14 / 18 / 22 / 26。

| 场景 | 值 |
|---|---|
| 页面左右内边距 | 28px |
| 页面顶部内边距 | 24px |
| 页面底部留白（避让 Dock） | 80px |
| 页面 section 间距 | 16px |
| 卡片内边距 | 18px |
| 面板内边距 | `20px 24px` |
| 卡片内部元素间距 | 14px |
| 卡片网格 gap | 16px |
| 工具条内边距 | `12px 14px` |
| 表单行间距 | 14px |

### 3.4 圆角

收敛为 **6 档**：

| 令牌 | 值 | 适用 |
|---|---|---|
| `--r-xs` | 6px | badge、tag、小角标 |
| `--r-sm` | 8px | 小按钮、图标按钮、输入框内小控件 |
| `--r-md` | 10px | 按钮、输入框、下拉、选项卡容器 |
| `--r-lg` | 14px | **卡片 / 玻璃面板（主用）** |
| `--r-xl` | 16px | Dock、抽屉、浮动大面板 |
| `--r-pill` | 999px | 仅状态 pill（如 `core-badge`） |

删除 3 / 4 / 5 / 7 / 9 / 11 / 16px。**规则：嵌套元素圆角 = 父圆角 − 4px**（14 → 10 → 8）。

### 3.5 阴影与描边

| 令牌 | 值 | 用途 |
|---|---|---|
| `--card-shadow` | 深色 `none` / 浅色 `0 2px 8px rgba(0,0,0,.08)` | 静置卡片 |
| `--card-shadow-hover` | 内外各 1px `--accent-50` 细环（浅色额外叠 `0 10px 28px`） | hover |
| `--float-shadow` | 深色 `none` / 浅色 `0 8px 24px rgba(0,0,0,.1)`（**任何主题都不画 1px 细环**） | 底部悬浮岛：Dock、启动条 |
| `--layer-shadow` | **新增** `0 8px 28px rgba(0,0,0,.18)` | 抽屉 / 浮层 |
| `--pop-shadow` | **新增** `0 12px 36px rgba(0,0,0,.32)` | 弹出菜单 / Popover |
| `--border` | 现有 | 唯一描边色；禁止 `1px solid rgba(...)` 手写 |

**卡片不要黑色投影**：深色主题底色本身就是 `#10131a`，纯黑阴影压上去只是把卡片四周压得更黑 —— 静止态 28% 几乎看不出，一旦悬停到 42% 就会沿四边糊出一圈黑边，看起来像描边而不是浮起；有背景图 / 动态场景时（`:root.has-bg` 把 `--panel` 变成深色半透）黑圈会更明显。深色下的层次靠 `--panel` 明度差 + 悬停时的 accent 细环表达；只有浅色主题的白卡需要投影来拉开与浅底的层次。**悬浮岛同理**（`--float-shadow`）：Dock 这类半透明玻璃条在深色底上投黑影子只会糊出一圈脏边；但**用 1px 细环勾边界同样不行**（圆角处会显出一道亮线，像贴了边框贴纸）。正解是「不描边、不投影」，靠 `--panel` 玻璃底 + 磨砂与卡片保持同一套视觉语言。

**hover 统一规则**：卡片与列表行只变阴影或背景（`--panel` → `--panel-hover`），**不 translate / scale**；按钮 hover 只变背景色档位，禁止 `filter: brightness()`。

### 3.6 动效

| 场景 | 时长 / 曲线 |
|---|---|
| 按钮、图标、hover | `--dur-sm .18s` `--ease-out` |
| 指示器滑动 | `0.26s` `--ease-out`（强制走 composable） |
| 页面转场 | `--dur-md .24s` |
| 主题切换 | 320ms，仅颜色过渡，禁止过渡 `backdrop-filter` |
| 弹窗出现 | `--dur-md`，位移 ≤ 8px |

**禁止**：装饰性 bounce、超过 400ms 的入场、线性动画、未处理 `prefers-reduced-motion` 的动效。

### 3.7 层级（z-index，当前完全缺失）

| z-index | 层 |
|---|---|
| 10 | Dock / 启动区 |
| 20 | 吸附页头（sticky） |
| 30 | 下拉、Popover、右键菜单 |
| 40 | 抽屉 |
| 50 | 弹窗（n-modal） |
| 60 | Message / Notification |
| 70 | Tooltip |

**现状实测冲突**（必须按上表归位）：`SideBar 45/44`、`InstanceCard 50/60`、`ServerDetail 100`、`AccountDrawer / DownloadsDrawer 90/95`、`FileManager / ServerFileManager 900`、`ContextMenu 1200`、`ModpackDropOverlay 1400`、`ColorPicker / SkinView / NbtTreeEditor 2000`、`InstanceDetail 2500`、`ImageViewer / TimeMachineLightbox 3000`、`SplashScreen / LaunchProgress 9999`。

---

## 4. 页面骨架规范

所有主视图（`src/views` 下的一级页面）使用同一骨架：

```text
.qx-page                  width: 100%;（不设宽度上限，不居中收窄）
                          display: flex; flex-direction: column; gap: 16px;
  .qx-toolbar             玻璃面板：padding 10px 12px / radius 14 / gap 10px
  .qx-page-body           flex: 1; gap: 16px;
```

> `.qx-page-head / .qx-page-title / .qx-page-sub` 已从 `components.css` 删除（零引用）。
> 页面标题属于页面自己的事，要在某页出现就在那一页写，不要复活共享类。

规则：

- **不设内容宽度上限**。内容区宽度直接跟随窗口、左对齐。原先的 `max-width: 1120px; margin: 0 auto`
  已移除：宽屏下右侧大片留白，而且各页加不加全靠自觉，反而制造了更多不一致。要收窄是页面自己的事，
  不要写回 `.qx-page`。
- **不强制页面标题**。实例页原本补过「标题 + 实例总数 + 导入整合包按钮」的页头，已按实际观感撤掉，
  导入按钮回到分组筛选行里。页标题属于页面自己的选择，
  不要再按「每页必须有且只有一个 20px 主标题」去补。发现流、工具箱这类以内容为主的页面直接铺内容即可。
- 详情页（Instance / Project / Server）头部沿用 `.glass` 大卡（`padding: 20px 24px`）。
- 如果某页确实需要标题，字号用 `--fs-lg 20px`、字重 600，不要再出现 21 / 22px 的「差不多大」。
- `.qx-page` 只管纵向排布与间距；根元素已有自定义 flex 布局的页面不必强行套用它。

---

## 5. 组件规范

### 5.1 按钮（4 档，仅此 4 档）

| 档位 | 高 | padding | 字号 | 圆角 | 用途 |
|---|---|---|---|---|---|
| `lg` | 36px | `0 18px` | 13 / 600 | 10 | 页面主操作、空状态 CTA、创建流程 |
| `md` | 32px | `0 14px` | 13 / 600 | 10 | **默认**：卡片操作、工具条、表单提交 |
| `sm` | 28px | `0 12px` | 12 / 600 | 8 | 次级操作、卡片脚部 |
| `xs` | 24px | `0 10px` | 12 / 500 | 6 | 极密列表内操作（少用） |

变体（颜色规则，非尺寸）：

- **primary**：`linear-gradient(135deg, var(--accent), var(--accent-deep))`，文字 `--accent-contrast`
- **ghost**：`background: var(--w-06)` + `1px solid var(--border)`
- **text**：无底无边，hover `--w-06`
- **danger**：`var(--danger-bg)` 底 + `1px solid var(--danger)` 40% 边 + `var(--danger-text)` 字

**废除**：`.btn.big`（用 `lg`）、`.btn.sm` 的 `6px 12px`，以及 `9px 18px`、`8px 18px`、`13px 26px`、`7px 13px`、`8px 14px`、`8px 12px`、`6px 12px` 等所有 padding 字面量。主按钮 hover 一律用 `--accent-hover`，废除全项目 `filter: brightness(1.05~1.12)`。

#### `.mini-btn`（兼容层，已收口到 `components.css`）

历史上 17 个文件各写一份 `.mini-btn`：padding 六种（`4×8 / 5×9 / 5×10 / 6×10 / 7×12 / 7×13`）、圆角三种（7/8/9）、高度 24–34 不等，并排时按钮高度对不齐。现在**只在 `components.css` 定义一次**，规格 = 上面的 `sm` 档（28px 高 / `0 12px` / 12px / 600 / radius 8）。

| 变体 | 外观 | 用途 |
|---|---|---|
| `.mini-btn` | `--w-06` 底 + `--border` 描边 | 默认次级操作 |
| `.mini-btn.primary` | `--accent-soft` 底 + `--accent-45` 描边 + `--accent` 字 | 行内「这个可以点」 |
| `.mini-btn.cta` | accent 渐变 + `--accent-contrast` 字 | 区块主操作（一屏最多一个） |
| `.mini-btn.danger` | `--danger-14` 底 + `--danger-text` 字 | 删除类 |
| `.mini-btn.active` | 同 primary | 可切换态 |
| `.mini-btn.icon-only` | 28×28 方形，无左右内边距 | 纯图标 |
| `.mini-btn.solid` | `--bg-2` 实底 | 压在图片 / 地图 / 画布上 |

**规矩**：页面 `<style scoped>` 里**禁止**再出现 `.mini-btn { ... }` 本体或变体定义 —— scoped 会编译成 `.mini-btn[data-v-x]`，优先级高于全局，留着就永远盖不住。页面里只允许留 `margin-left: auto` / `flex-shrink` 这类纯布局覆写。

### 5.2 图标按钮

只有两档：**32×32 / radius 8**（内容区默认）、**28×28 / radius 8**（紧凑列表）。
圆形按钮（`border-radius: 50%`）**仅限**标题栏窗口按钮与抽屉头部关闭键。
统一 hover：`background: var(--w-08)`。

### 5.3 输入框 / 下拉

统一规格：高 32px、`padding: 0 12px`、`radius: 10px`、`background: var(--w-06)`、`border: 1px solid transparent`，focus 时 `border-color: var(--accent-50)` + `box-shadow: 0 0 0 2px var(--accent-16)`。

naive-ui 的 `NSelect / NInput` 统一使用 `size="medium"`，**禁止** `size="small"` 与自定义高度混用（当前 Browse 使用 small、其他页面使用 medium）。搜索框统一最小宽 220px，可弹性伸缩。

### 5.4 分段控件 / 标签页（强制统一）

只有两档，全部由 `src/styles/components.css` 的 `.qx-seg` 提供，页面里不再写容器 / 按钮 / 滑块的尺寸与配色。

| 属性 | 默认档（页面级标签栏） | 紧凑档 `.qx-seg--sm`（面板 / 弹窗 / 表单行内的切换） |
|---|---|---|
| 容器 | `padding: 4px` / `radius: 14px` / `--panel` + backdrop 8px / `gap: 4px` | `padding: 4px` / `radius: 10px` / 其余同左 |
| 项 | `height: 36px`（`--ctl-lg`） / `padding: 0 12px` / 13px / 600 / `radius: 8px` | `height: 28px`（`--ctl-sm`） / `padding: 0 10px` / 12px / 600 / `radius: 6px` |
| 整条高度 | **44px** | **36px** |
| 指示器 | `.qx-seg__indicator`：`--accent-soft` / 上下各 4px 贴合 / 圆角随档位 | 同左 |
| 实现 | **必须** `useSlidingIndicator`，禁止手写 indicator；指示器元素固定用 `.qx-seg__indicator` | 同左 |
| 状态色 | 默认 `--text-2` → 悬停只铺 `--w-06` 淡底、文字色不动 → 选中 `--accent`（全走令牌，禁写死颜色）。**禁止**把悬停文字提到 `--text-1`：深色下那是 `#f2f3f7`，几乎纯白，会盖过选中态 | 同左 |

> 早期版本这里写的是「项 28px / `padding: 0 16px`」。实际落地时项内是 13px 文字 + 14px 图标，28px 会被行高撑破、比页面里其它控件矮一档；发现页分类切换实测 35px 才是视觉基准，故默认档改为 **36px**（`--ctl-lg`）。28px 保留给紧凑档，用于弹窗与设置页表单行，不用于页面级标签栏。

用法（页面里只留布局差异）：

```html
<div ref="segRef" class="my-tabs qx-seg">           <!-- 或 qx-seg qx-seg--sm -->
  <div class="qx-seg__indicator" :style="segStyle"></div>
  <button :class="{ active: tab === 'a' }">A</button>
  <button :class="{ active: tab === 'b' }">B</button>
</div>
```

```css
/* 页面样式里最多只保留这类布局覆盖，尺寸/颜色一律不写 */
.my-tabs { align-self: flex-start; }
```

> 指示器挂在**页面组件**上（`useSlidingIndicator` 在页面 setup 里调用），`onMounted` 只跑一次。控件那一刻若正被收起的页签（`v-show`）藏着、或在 `v-if` 分支里还没渲染，量到的矩形是 `0×0`。composable 自带兜底：量不到尺寸就不写样式，并在 `ResizeObserver` / 父组件 `onUpdated` 后补测，所以一般不用手动 `refresh()`。但「切页会整体重建面板」的页面（设置页 `.settings-body` 带 `:key="tab"`）仍建议把新增的 seg 加进它的统一 refresh 列表，双保险。
>
> 按钮内容始终水平居中（`.qx-seg > button` 带 `justify-content: center`）。**默认不要给按钮加 `flex: 1` 或给容器定宽**——项比文字宽出来，右侧就会空一块；需要等宽时让内容居中，或者干脆按内容宽度排。

已迁移（页内只留 `align-self` / `flex: none` / `display: flex` 这类布局差异）：

| 默认档（36px） | 紧凑档（28px） |
|---|---|
| MultiplayerView `.mode-tabs` / `.mp-subtabs` | SettingsView `.seg` ×6（关闭行为 / 主题 / 语言 / 内存模式 / 代理模式 / 更新源） |
| BrowseView `.type-card`（吸顶过渡保留在页内） | SkinView 动作 / 模型切换 ×2 |
| ProjectDetailView `.type-card` | DownloadsDrawer `.tabs` |
| CreateInstanceView `.mode-tabs` | InstallDialog `.id-type-tabs` |
| ServerDetailView `.tabs` | ServerDetailView `.props-mode-tabs`（表单 / 源码） |
| InstanceDetailView `.tabs`（保留整行 + 右侧操作区靠右） | RoomDirectoryPanel `.rd-subtabs` |
| NBTEditorView 三大分区 `.seg` | NBTEditorView 表单 / 树形切换 |
| TimeMachineView `.seg` | NBT ChunkEditor `.seg-group`（地图 / 列表） |
| SkinView `.skin-tab-seg`（已保存 / 官方） | BrowseView `.view-switch`（网格 / 列表 / 紧凑，按钮定宽 30px） |

仍是独立按钮组、**不属于**本节范围（不是"一条容器 + 滑块"的标签栏）：`CreateInstanceView` / `MultiplayerView` 的 `.ver-cats`（版本分类筛选 chips）、`DiagnosticsDialog` 的 `.dg-modes`（预览模式）、`SavesTab` 的 `.seg`（实为 `radius 999` 胶囊，应按 §5.4 之前的约定更名 `.pill-group`）。

### 5.5 卡片

| 属性 | 值 |
|---|---|
| 容器 | `.glass`（`--panel` + `radius 14px` + backdrop 8px） |
| 内边距 | 18px（内容密集可用 `20px 24px`） |
| 内部 gap | 14px |
| 图标 | 46×46 / `radius 12px`（详情页头 56×56 / 14px） |
| 标题 | `15px / 600 / --text-1`，单行省略 |
| 副文本 | `12px / --text-3` |
| 脚部 | `border-top: 1px solid var(--border); padding-top: 12px` |
| hover | 仅 `box-shadow: var(--card-shadow-hover)` |

**统一卡片背景写法**：内容区卡片只用 `--panel`；卡片内次级块用 `--w-03`；浮层用 `--bg-2`。禁止 `color-mix()` 自造底色（SeedMap 的 `--feat-solid` 除外，因为它必须保证地图上可读）。

> **`.qx-card` 已与 `.glass` 对齐**：不再自带 `1px solid var(--border)` 与 `--card-shadow`。此前它俩不一致，导致「按规范用 `.qx-card` 反而和周围的卡长得不一样」——全站 6 个真实卡片类（`.inst-card` / `.pin-card` / `.server-card` / `.tool-card` / `.p-card` / 设置页 `.card`）都是无描边 + 无静置投影，只有悬停给 `--card-shadow-hover` 细环。要描边的场景请显式加 `.qx-card--outline`，不要改基础款。

### 5.6 列表行（菜单 / 下拉 / 实例列表）

行高 40px（紧凑 36px）、`padding: 0 10px`、`radius: 8px`、hover `--w-06`、选中 `var(--accent-14)` + 文字 `--accent`。

弹出面板：`--bg-2` 实色（不用玻璃）、`radius 12px`、`--pop-shadow`、`padding 6px`。

### 5.7 徽章 / 标签

高 20px、`padding: 0 6px`、`radius 6px`、`11px / 600`。语义色只用 `--{sem}-bg` + `--{sem}-text`。版本号类使用 `--mono 11px`。

### 5.8 空状态（统一为一种）

```text
padding: 48px 24px; text-align: center; gap: 10px;
图标 36×36 / --text-3 / opacity .5
标题 15px / 600 / --text-2
说明 13px / --text-3
可选 一个 md 尺寸按钮
```

CloudSync 的 `padding 26px 16px` + dashed 边框属于特例：dashed 边框可保留，但 padding 与图标尺寸必须符合上式。

现状另有 3 种空态：Schematic / ResourcePack 的 `.empty-state`（`radius 12 / padding 24 / gap 16`，无图标标题结构）、Shimu 的 `.empty`（`min-height 260` 纯文字居中）、TimeMachine 的 `.gate`（图标 52×52 / `radius 16`）。全部归并为上式的单一结构。

### 5.9 加载骨架

沿用现有 `.skel-card / .skel-icon / .skel-line / .skel-tag / .skel-btn`。要求：**任何 async 列表首次加载都必须用骨架**，禁止只用文字"加载中"。骨架块圆角同样遵守 §3.4 层级。

### 5.10 弹窗（4 档宽度 + 统一遮罩）

| 档 | 宽度 | 用途 |
|---|---|---|
| `sm` | 420px | 确认框、单字段表单 |
| `md` | 560px | 标准表单、列表选择 |
| `lg` | 680px | 复杂表单、安装流程 |
| `xl` | 780px | 文件浏览、诊断、宽内容 |

统一：`max-width: 92vw`、`radius: 16px`、`preset="card"`、header `16px / 600`、body padding `20px 24px`、footer 右对齐 + `gap 8px`、遮罩 `rgba(0,0,0,.45)` + `backdrop-filter: blur(6px)`。

改造清单：380 → 420；440/460/480 → 420 或 560；520/580 → 560；600/620/640 → 680；700/720 → 780；`max-width` 全部改为 92vw。

### 5.11 抽屉

统一：`right: 10px; top: 50px; bottom: 10px; width: 420px; radius: 18px; --layer-shadow`。头部高 44px、标题 `15px / 600`、右侧 28×28 关闭键。遮罩与弹窗一致。

现状：`DownloadsDrawer` 已是 420px，但 `AccountDrawer` 仍是 **380px**；两者遮罩都是 `rgba(0,0,0,.18)` 且无模糊，需统一到弹窗遮罩 `rgba(0,0,0,.45) + blur(6px)`。

---

## 6. 网格规范

按内容类型定四档，**不允许出现第 5 种值**：

| 令牌 | minmax | 用于 |
|---|---|---|
| `--grid-rich` | `minmax(320px, 1fr)` | 带图标 + 两行文本 + 脚部操作的卡片（实例、服务器、整合包、工具卡） |
| `--grid-card` | `minmax(280px, 1fr)` | 纯信息卡片、设置卡、置顶卡 |
| `--grid-tile` | `minmax(200px, 1fr)` | 版本条目、截图、存档、世界 |
| `--grid-thumb` | `minmax(96px, 1fr)` | 材质 / 皮肤图标墙 |

映射：Home / Instances / Multiplayer / ServerDetail → `rich`；Settings / Toolbox → `card`；ProjectDetail 版本、NBT 世界、InstanceDetail 截图 → `tile`；Skin / RP → `thumb`。当前 240 / 210 / 330 / 340 全部归位。

---

## 7. 交互与动效规范

- **加载态**：超过 300ms 的操作必须显示骨架或按钮内 loading；按钮 loading 时宽度不变。
- **禁用态**：`opacity: .45` + `cursor: not-allowed`，不使用 `grayscale`。
- **焦点**：键盘焦点统一 `outline: 2px solid var(--accent-50); outline-offset: 2px`，禁止直接 `outline: none`。
- **滚动**：内容区滚动条 8px，页面级不使用横向滚动。
- **吸附**：吸附页头的负偏移必须用 `calc(-1 * var(--content-pad-top))`，底衬用 `--head-glass` + `backdrop-filter`，并与 App 的 `--glass-blur` 联动。
- **减少动效**：所有新增动画必须放进 `@media (prefers-reduced-motion: reduce)` 的关闭列表。

---

## 8. 深浅主题与自定义背景图

- 所有页面元素**必须**在 `:root.light` 下仍可读。判定标准：不使用白色叠加时，浅色主题自动反转为黑色叠加（`--w-*` 已实现），因此**禁止 `rgba(255,255,255,x)` 与 `#fff` 字面量**。
- 自定义背景图（`has-bg`）模式下，卡片 `--panel` 的不透明度由 App 统一控制，页面**不得**再自行加 `background-color: var(--bg-1)` 遮住背景。
- 地图、预览器这类需要"绝对可读"的场景，允许使用 `--feat-solid`（color-mix 88%），这是唯一被批准的 `color-mix` 用法。

---

## 9. 代码落地约定（协作红线）

### 必须

1. 颜色 / 圆角 / 间距 / 字号 / 阴影 / 时长，**只能**引用 §3 的令牌；缺令牌时先加令牌。
2. 所有新页面套用 §4 骨架，根节点含 `.qx-page` 语义类名。
3. 所有切换器使用 `useSlidingIndicator`；所有异步列表使用 `.skel-*`。
4. 弹窗宽度只能取 420 / 560 / 680 / 780 四档，`max-width: 92vw`。
5. 组件 `<style>` **必须 `scoped`**；弹窗内容若被 teleport，使用 `:deep()` 或全局专用前缀 `.qx-*`，**不得**定义裸 `.btn` / `.card` / `.tabs` 全局类。
6. naive-ui 使用边界：表单控件（`NInput / NSelect / NSwitch / NProgress`）、`NModal preset="card"`、`NPopover`、`useMessage / useDialog` 可用；**页面主体卡片、按钮、标签页不得使用 naive-ui 组件**，统一用项目类，避免两套视觉并存。

### 禁止

- 手写 `rgba(...)` 表达主题色叠加（用 `--accent-XX` / `--w-XX`）。
- 手写 indicator、手写 `transition` 时长（用 `--dur-*`）。
- 卡片 hover 位移 / 缩放。
- 新增字号（14 / 16 / 17 / 21 / 26+）、圆角（3 / 4 / 5 / 7 / 9 / 16）、间距（2 / 6 / 7 / 9 / 10 / 14 / 18 / 22 / 26）。
- 在业务组件里重定义已被全局定义的类名。
- **引用未在 `styles.css` 中定义的变量**。`var()` 一律写 fallback 不够，必须先在 P0 步骤把 11 个幽灵令牌补齐或改用已存在令牌：`--bg-3`、`--surface-1/2`、`--panel-2`、`--panel-sub`、`--text-main`、`--card-bg`、`--radius-md/lg`、`--border-strong`、`--glass-blur`。

### 新增建议令牌（一次性加入 `styles.css`，后续全部受益）

`--fs-*`（7 个）、`--r-*`（6 个）、`--layer-shadow`、`--pop-shadow`、`--head-glass`、`--{danger|success|info|warn}-bg`、`--grid-*`（4 个）。

---

## 10. 落地优先级

### P0（不统一就永远割裂，影响所有页面）

1. 在 `styles.css` 补齐 §3 令牌（纯新增，零风险）。
2. 抽出共享类：`.qx-page / .qx-page-head / .qx-page-title / .qx-page-sub / .qx-toolbar / .qx-grid-* / .qx-btn(-lg/-md/-sm/-xs) / .qx-icon-btn / .qx-empty / .qx-seg`。
3. ~~给 5 个缺失标题的页面补统一页头~~（**已作废**：页面标题不是必须的，见 §4）。
4. ~~统一页面最大宽度 1120~~（**已作废**：全站取消宽度上限，见 §4）。左右 padding 仍按 `#app-content` 的 26px。

### P1（视觉差异最刺眼）

5. 按钮全量替换为 4 档（先改 `.btn` 定义，再逐页删本地 padding）。
6. 弹窗宽度归 4 档 + `max-width: 92vw`。
7. 切换器全量迁移到 `useSlidingIndicator`（6 处）。
8. 圆角 13 种 → 6 种（重点：9px 全部 → 8 或 10，16px → 14）。
9. 网格 `minmax` 9 种 → 4 种。

### P2（收尾）

10. 硬编码颜色清理（表格见 §3.1）。
11. 6 个非 scoped 全局样式改为 scoped / `:deep()`：`InstallDialog`、`AddAccountDialog`、`IconPickerDialog`、`InstallChecklist`、`VersionPicker`、`AccountChip`。
12. DownloadsDrawer 遮罩与弹窗对齐。
13. 补 z-index 令牌并替换 20 种散落数值（含 3 处冲突的 `2000`）。
14. 清除 16 处小数字号与 17 处 `filter: brightness()`。
15. Skin 的 `.cape-overlay`、InstanceDetail 的 `2500`、ImageViewer 的 `3000` 统一到层级表。
16. 幽灵令牌清零（11 个）：优先补 `--surface-2` / `--bg-3` / `--panel-sub`，其余改引用现有令牌；补完后跑一次全量 `var(--` 存在性校验。
17. 合并 `FileManager` 与 `ServerFileManager` 的重复样式为一个共享组件，消除 480px / `calc(100vh - 320px)` 两套高度。
18. `AccountDrawer` 380 → 420，抽屉遮罩对齐弹窗；`SavesTab` 的 `.seg` 更名 `.pill-group`。
19. 统一按压反馈：删除 `RoomDirectoryPanel` 的 `translateY(-2px)`、色板的 `scale(1.15)`、`FriendsPanel` / `QookixLoginDialog` 的 `scale(.97)`，hover 只允许改背景/阴影。
20. 三处"深色子系统"（`CodeEditor`、`ImageViewer`、`TimeMachineLightbox`）把硬编码深色底收敛为 `--k-*` 令牌，并禁止在其中重定义 `--accent` 等全局语义变量（`CodeEditor` 目前把 `--accent` 覆盖成 `#e89a4b`，会让用户自定义主题色失效）。

---

## 11. 验收清单（PR Review 用）

- [ ] 页面没有擅自加 `max-width`（全站不设内容宽度上限）；确有需要时是本页的局部决定，不是规范要求
- [ ] 页面内出现的数字，全部能在 §3 表格里查到
- [ ] `rgrep "rgba(" src/views` 无手写主题色（0 处）
- [ ] `rgrep "#[0-9a-fA-F]{3,6}" src/views` 结果为空
- [ ] 每个 `n-modal` 宽度 ∈ {420, 560, 680, 780}
- [ ] 每个切换器的 indicator 来自 `useSlidingIndicator`
- [ ] 每个组件 `<style>` 都是 scoped
- [ ] 新增/修改的 `var(--x)` 中，`x` 能在 `styles.css` 或 `theme.ts` 里找到定义（幽灵令牌 0 处）

---

## 12. 附录 A：全量取证范围（80/80）

统计口径：`src/**/*.vue` 共 **80** 个文件（`App.vue` 1 + `views/` 17 + `components/` 62）。以下文件均已逐个读取 `<template>` 与 `<style>` 块，不是关键词扫描。

### 12.1 文件清单

```text
App.vue

views/ (17)
BrowseView  CreateInstanceView  HomeView  InstanceDetailView  InstancesView
MultiplayerView  NBTEditorView  ProjectDetailView  ResourcePackPreviewView
SchematicPreviewView  SeedMapView  ServerDetailView  SettingsView
ShimuLibraryView  SkinView  TimeMachineView  ToolboxView

components/ 根级 (47)
AboutShowcase  AccountChip  AccountDrawer  AddAccountDialog  AppIcon
CloseConfirmDialog  CloudSyncDialog  CodeEditor  ColorPicker  ContextMenu
CrashAnalyzer  CrashDialog  DiagnosticsDialog  DownloadsDrawer  FileManager
ForceUpdateOverlay  FriendsPanel  FrpDialog  FrpPanel  IconPickerDialog
ImageViewer  InstallChecklist  InstallDialog  InstanceCard  LanSyncPanel
LaunchProgress  LoadingBarBridge  LogViewer  McMotd  ModpackDropOverlay
OnboardingBar  PlaytimeCard  ProjectCard  QookixLoginDialog  RoomDirectoryPanel
ServerFileManager  ShimuLoginDialog  ShimuPostCard  SideBar  SimplePagination
SkinThumb  SplashScreen  TimeMachineLightbox  TitleBar  UpdateNotes
UpdaterCheck  VersionPicker

components/instance/ (5)
ContentTab  ExportDialog  KeybindTab  SavesTab  SettingsTab

components/nbt/ (8)
BackupPanel  ChunkEditor  ChunkMap  EnchantEditor  ItemEditor
NbtTreeEditor  PlayerEditor  TreeNode
```

### 12.2 逐文件关键偏差（仅列出有偏离的文件）

| 文件 | 关键偏差 |
|---|---|
| `AccountChip` | 双 `<style>` 块（第二个全局）；chip 48/12 与 bar 30/999 两套；头像 34/radius 8；类型徽章 10px/6 |
| `AccountDrawer` | 宽 380（应 420）；行 `8px 9px`/10；头像 34/9；遮罩无模糊 |
| `AboutShowcase` | 仅 canvas，高 200 硬编码，无令牌问题 |
| `AppIcon` | 7 组渐变色硬编码在 TS 中，与 `IconPickerDialog` 的 `.ip-swatch` 同一色系但数值不同 |
| `CodeEditor` | 圆角 `0 0 13px 13px`（第 7 档）；强制深色并重定义 `--accent: #e89a4b`；选区 `rgba(232,154,75,.32)`；语法色硬编码 |
| `ColorPicker` | z 2000；面板 232/16/14；底色 `rgba(24,27,36,.92)`；hover `scale(1.15)`；输入 28/8 |
| `ContextMenu` | z 1200；radius 11；`12.5px`；项 `7px 9px`/8；padding 5 |
| `CrashAnalyzer` | 侧栏 280；卡片 10；计数徽章 radius 20 |
| `CrashDialog` | `.mod-chip` radius 999 + `rgba(150,181,225,.14/.35)`；`--panel-2`/`--text-main` 幽灵令牌；`13.5px` |
| `DiagnosticsDialog` | 模式按钮 11px/6/`3px 10px`；删除色 `#e06c6c`；`--panel-sub` 幽灵令牌 |
| `FileManager` / `ServerFileManager` | 样式近乎复制；高度 `calc(100vh-320px)` vs 480；nav 26/7；行 9；`12.5px`；硬编码 `#7cb8f5/#57c257/#f08a67` |
| `ForceUpdateOverlay` | 卡宽 440（不在 4 档）；标题 18px；`--card-bg` 幽灵令牌；`#f0a020`/`#e55` |
| `FrpDialog` | 宽 700（不在 4 档） |
| `FrpPanel` | 网格 240；tun 12/`12px 14px`；subtab 26/8/11；`--border-strong` 幽灵令牌 |
| `FriendsPanel` | 空态图标 40；`.fp-btn` 高 32/radius 9/12；主按钮色 `#1a1206`；active `scale(.97)` + brightness |
| `IconPickerDialog` | **非 scoped**；`#8b8e9c/#c6c8d2/#e89a4b/#f2f3f7`；预览 72/16；按钮 9；图标墙 48 |
| `ImageViewer` | z 3000；全部硬编码深色（`#e6e7ec/#d5d7df/#b6b8c2`）；icon 32/8；pct 28/7 |
| `InstallChecklist` | **非 scoped**；行 `10px 12px`/10；tag 10/radius 5；`#e8a04b/#4caf50/#2196f3` |
| `LanSyncPanel` | 按钮 `6px 16px`/8/12；badge radius 20；二维码白底 `#fff` |
| `LogViewer` | 面板高 `calc(100vh-320px)`；`.mini` 7/`4px 10px`；日志色 `#c9ccd6` |
| `ModpackDropOverlay` | z 1400；卡片 18/`40px 56px`；图标 64 圆 |
| `OnboardingBar` | `--radius-lg`、`--bg-3` **无 fallback 幽灵令牌**（3 处背景失效）；关闭键 26 |
| `PlaytimeCard` | 数值 22px（不在字号档）；label 11；range 10；柱 radius 2 |
| `ProjectCard` | 卡 padding 14；图标 52/radius 11；按钮 `6px 13px`/8/12 + brightness + scale |
| `QookixLoginDialog` | 提交 36/10 + `#1a1206` + brightness + `scale(.97)` |
| `RoomDirectoryPanel` | 网格 260；房间 12 且 hover `translateY(-2px)`；subtab 10/8 |
| `SavesTab` | `.seg` 实为 pill（999/`7px 14px`）；列表 18；图标 44/10 |
| `ShimuPostCard` | 与 ProjectCard 同族但图标 56/10；封面渐变 `#6b5b95/#352f44` |
| `SimplePagination` | 30/radius 7/13；active 用 `--accent-soft` |
| `SplashScreen` | z 9999；logo radius 22；硬编码 `#0b0d12/#0f1218/#f2f3f7` |
| `TimeMachineLightbox` | z 3000（与 ImageViewer 争层）；侧栏 300/14；nav 44 vs ImageViewer 40 |
| `UpdateNotes` | 卡 10/`12px 14px`；tag 11/5；`#3fb950/#4493f8/#a371f7`；`--bg-3` 幽灵令牌（有 fallback） |
| `VersionPicker` | **非 scoped**；触发器 30/8/12；tab 11；项 7/radius 7；徽章 9/4；`#4caf50/#e8a04b/#f44336` |
| `ExportDialog` | 自绘 checkbox 15/4（与 InstallChecklist 的勾选框两套）；折叠 0.28s |
| `KeybindTab` | `.mini-btn` `7px 13px`/8；空态 12；键帽 radius 6/9px；冲突底 `rgba(229,83,75,.28)` |
| `ContentTab` | `.btn` `9px 16px`/10；icon-btn 30/8；空态无图标；`rgba(240,101,67/150,181,225/78,201,160)` |
| `SettingsTab`(instance) | 网格 320/gap 14；卡 16；text-input radius 9；`#5a8ef0/#8ab4ff/#e89a4b/#f2c079/#8ec4ff` |
| `ChunkEditor` | `.seg-group` 9 / `.seg` 7（第 6 套 seg） |
| `ChunkMap` | mini-btn 26/7/`4px 8px`；底色 `#101319`；badge 7 |
| `EnchantEditor` | `--surface-2` 幽灵令牌；input 8/`6px 10px` |
| `ItemEditor` | 槽位网格 66；slot 42/8；字号 9/10；`--surface-2`；mini-btn `5px 10px` |
| `NbtTreeEditor` | z 2000；菜单 10；`--surface-1`→`--surface-2`（两级都未定义） |
| `PlayerEditor` | `--surface-2` 幽灵令牌；input radius 8 |
| `TreeNode` | 行 `2px 4px`/6；字号 10–12 |

### 12.3 无需改造（合规或纯逻辑）

`App.vue`、`SideBar`、`TitleBar`、`InstanceCard`、`CloudSyncDialog`、`InstallDialog`、`AddAccountDialog`、`CloseConfirmDialog`、`DownloadsDrawer`（宽度已合规，仅遮罩）、`McMotd`、`SkinThumb`、`LoadingBarBridge`、`UpdaterCheck`（无样式块）、`SeedMapView` / `SchematicPreviewView` / `ResourcePackPreviewView` / `NBTEditorView` 的图面容器（仅记录，属工具型全屏布局）。

---

## 13. 改造实施方案（历史记录 · 大部分已执行，§13.5 已作废）

### 13.0 施工顺序

7 个 PR，严格按序；每个 PR 自带回滚点，**不跨 PR 混改**。

| PR | 内容 | 视觉变化 | 依赖 |
|---|---|---|---|
| PR-1 | 令牌补齐 + 幽灵令牌清零 | 无（应为 0 像素变化） | 无 |
| PR-2 | 共享原子类（按钮 / 输入 / 徽章 / 行 / 空态）落地 | 无（先新增类，不替换） | PR-1 |
| PR-3 | 按钮族收敛（`.btn` / `.mini-btn` / `icon-btn` / 按压反馈） | 小 | PR-2 |
| PR-4 | 分段控件 6 → 1 | 中 | PR-2 |
| PR-5 | 页面骨架（8 页） | 大 | PR-1 |
| PR-6 | 弹窗 / 抽屉 / 浮层层级 | 中 | PR-1 |
| PR-7 | 网格、颜色、非 scoped、双胞胎组件 | 大 | PR-1~6 |

原则：**先加后删**。PR-2 只往 `styles.css` 里加类，不动任何组件；PR-3 起才开始删组件内的本地定义。这样任何一步出问题都能单独 revert，不会把令牌和样式改动缠在一起。

---

### 13.1 PR-1：令牌地基

#### 13.1.1 追加到 `:root`（`styles.css` L111 前）

```css
/* 字号：7 档。禁止 10.5 / 11.5 / 12.5 / 13.5 / 14 / 16 / 17 / 18 / 21 / 22 */
--fs-xs: 11px;
--fs-sm: 12px;
--fs-md: 13px;
--fs-lg: 15px;
--fs-xl: 20px;
--fs-2xl: 26px;
--mono: "Cascadia Code", Consolas, "Courier New", monospace;

/* 圆角：6 档 */
--r-xs: 6px;
--r-sm: 8px;
--r-md: 10px;
--r-lg: 14px;
--r-xl: 18px;
--r-pill: 999px;

/* 控件高度：与 §5.1 按钮表一致 */
--ctl-xs: 24px;
--ctl-sm: 28px;
--ctl-md: 32px;
--ctl-lg: 36px;

/* 层级：7 层，全应用只允许这 7 个值 */
--z-dock: 10;
--z-header: 20;
--z-popover: 30;
--z-drawer: 40;
--z-modal: 50;
--z-lightbox: 60;
--z-boot: 70;

/* 模糊：Splash / LaunchProgress / TitleBar 等 30+ 处已在引用但从未定义 */
--glass-blur: 8px;
```

#### 13.1.2 幽灵令牌处置表（11 个）

| 幽灵令牌 | 使用位置 | 处置 |
|---|---|---|
| `--surface-2` | `ItemEditor` ×2、`NbtTreeEditor`、`PlayerEditor`、`EnchantEditor` | **新增为正式令牌**（表单项/槽位底色）：`:root` 加 `--surface-2: var(--w-04)`，`:root.light` 加 `--surface-2: rgba(0, 0, 0, 0.035)`。这是当前最实在的 bug——这些输入框现在没有底色 |
| `--surface-1` | `NbtTreeEditor`（`var(--surface-1, var(--surface-2))`） | 改为 `var(--surface-2)`，不新增 |
| `--bg-3` | `OnboardingBar` ×3（无 fallback）、`UpdateNotes`、`SettingsView` ×2 | 全部替换为 `var(--w-03)`，不新增 |
| `--panel-2` | `CrashDialog` | 替换为 `var(--w-04)` |
| `--panel-sub` | `CrashDialog`、`ExportDialog`、`DiagnosticsDialog` | 替换为 `var(--w-02)` |
| `--text-main` | `CrashDialog` | 替换为 `var(--text-1)` |
| `--card-bg` | `ForceUpdateOverlay` | 替换为 `var(--bg-2)` |
| `--radius-md` / `--radius-lg` | `OnboardingBar` | 替换为 `var(--r-md)` / `var(--r-lg)` |
| `--border-strong` | `FrpPanel` | 替换为 `var(--w-14)` |
| `--glass-blur` | 30+ 处 | 按 13.1.1 正式定义 |

> 只在 `:root` 定义必要项（`--surface-2`、`--glass-blur`），其余**一律替换**而不是补定义——否则等于给错误命名发永久许可证，与 §9 红线冲突。

#### 13.1.3 PR-1 验收

```powershell
# 1. 幽灵令牌必须为 0（排除刚合法化的 --surface-2 / --glass-blur）
Select-String -Path src\**\*.vue -Pattern "var\(--(bg-3|surface-1|panel-2|panel-sub|text-main|card-bg|radius-md|radius-lg|border-strong)\b"
# 2. 构建 + 深浅两套主题各截 3 张图（Home / Settings / NBT），必须与改动前逐像素一致
npm run build
```

---

### 13.2 PR-2：共享原子类

新增到 `styles.css`，**只加不改**：

```css
/* ── 页面骨架 ───────────────────────────────────────── */
.qx-page { width: 100%; display: flex; flex-direction: column; gap: var(--gap-page); }  /* 不设宽度上限 */
.qx-page-head { display: flex; align-items: flex-end; justify-content: space-between; gap: 16px; }
.qx-page-title { display: flex; align-items: center; gap: 10px; font-size: var(--fs-xl); font-weight: 600; color: var(--text-1); margin: 0; }
.qx-page-sub { margin-top: 4px; font-size: var(--fs-sm); color: var(--text-3); }
.qx-toolbar { padding: 12px 14px; border-radius: var(--r-lg); background: var(--panel); border: 1px solid var(--border); display: flex; align-items: center; gap: 10px; }

/* ── 按钮：4 档 ─────────────────────────────────────── */
.qx-btn { display: inline-flex; align-items: center; justify-content: center; gap: 6px;
  border: 1px solid var(--border); border-radius: var(--r-md); background: var(--w-05);
  color: var(--text-1); font: 600 var(--fs-md)/1 inherit; cursor: pointer;
  transition: background var(--dur-sm) var(--ease-out), border-color var(--dur-sm) var(--ease-out); }
.qx-btn:hover:not(:disabled) { background: var(--panel-hover); }
.qx-btn:disabled { opacity: .45; cursor: not-allowed; }
.qx-btn--lg { height: var(--ctl-lg); padding: 0 18px; }
.qx-btn--md { height: var(--ctl-md); padding: 0 14px; }
.qx-btn--sm { height: var(--ctl-sm); padding: 0 12px; font-size: var(--fs-sm); border-radius: var(--r-sm); }
.qx-btn--xs { height: var(--ctl-xs); padding: 0 10px; font-size: var(--fs-sm); font-weight: 500; border-radius: var(--r-xs); }
.qx-btn--primary { border-color: transparent; background: var(--accent); color: var(--accent-contrast); }
.qx-btn--primary:hover:not(:disabled) { background: var(--accent-hover); }
.qx-btn--ghost { border-color: transparent; background: transparent; color: var(--text-2); }
.qx-btn--ghost:hover:not(:disabled) { background: var(--w-06); color: var(--text-1); }
.qx-btn--danger { border-color: transparent; background: var(--danger-14); color: var(--danger); }

/* ── 图标按钮 ───────────────────────────────────────── */
.qx-icon-btn { display: inline-grid; place-items: center; width: var(--ctl-md); height: var(--ctl-md);
  border: none; border-radius: var(--r-sm); background: transparent; color: var(--text-2); cursor: pointer;
  transition: background var(--dur-xs) var(--ease-out), color var(--dur-xs) var(--ease-out); }
.qx-icon-btn:hover:not(:disabled) { background: var(--w-08); color: var(--text-1); }
.qx-icon-btn--sm { width: var(--ctl-sm); height: var(--ctl-sm); }

/* ── 表单 ───────────────────────────────────────────── */
.qx-input { height: var(--ctl-md); padding: 0 10px; border-radius: var(--r-sm);
  border: 1px solid var(--border); background: var(--surface-2); color: var(--text-1);
  font: 400 var(--fs-md)/1 inherit; outline: none; }
.qx-input:focus { border-color: var(--accent); box-shadow: 0 0 0 2px var(--accent-16); }

/* ── 行 / 徽章 / 空态 ───────────────────────────────── */
.qx-row { display: flex; align-items: center; gap: 10px; height: 40px; padding: 0 10px;
  border-radius: var(--r-sm); cursor: pointer; transition: background var(--dur-xs); }
.qx-row:hover { background: var(--w-06); }
.qx-row.is-active { background: var(--accent-14); color: var(--accent); }
.qx-badge { display: inline-flex; align-items: center; height: 20px; padding: 0 6px;
  border-radius: var(--r-xs); font-size: var(--fs-xs); font-weight: 600; }
.qx-empty { display: flex; flex-direction: column; align-items: center; justify-content: center;
  gap: 10px; padding: 48px 24px; text-align: center; color: var(--text-3); font-size: var(--fs-md); }
.qx-empty-icon { opacity: .5; }
.qx-empty-title { font-size: var(--fs-lg); font-weight: 600; color: var(--text-2); }
```

命名说明：用 `.qx-` 前缀避免与 6 个全局样式块里已有的 `.btn` / `.card` / `.seg` 撞名——这也是 §9 红线要求的。

**PR-2 验收**：只检查 `styles.css` 能正常构建，组件视觉不变。

---

### 13.3 PR-3：按钮族收敛

#### 13.3.1 25 处 `.mini-btn` 定义 → `.qx-btn--sm` / `--xs`

逐个删除本地定义并替换类名（行号为当前定义位置）：

| 文件 | 行 | 现状 | 归入 |
|---|---|---|---|
| `InstanceDetailView` | 981 | `6px 12px / 8 / 12` | `--sm` |
| `NBTEditorView` | 1044 | `4px 10px / 7` | `--xs` |
| `ResourcePackPreviewView` | 649 | `6px 12px / 8` | `--sm` |
| `SchematicPreviewView` | 621 | `6px 12px / 8` | `--sm` |
| `SeedMapView` | 1834 | `6px 12px / 8` | `--sm` |
| `ShimuLibraryView` | 1046 | `6px 12px / 8` | `--sm` |
| `SettingsView` | 3083 | `6px 12px / 8` | `--sm` |
| `SkinView` | 1302 | `6px 12px / 8` | `--sm` |
| `instance/SettingsTab` | 654 | `5px 10px / 8` | `--sm` |
| `instance/SavesTab` | 705 | `6px 12px / 8` | `--sm` |
| `instance/KeybindTab` | 1018 | `7px 13px / 8` | `--sm` |
| `nbt/ChunkEditor` | 182 | `5px 10px / 8` | `--xs` |
| `nbt/ChunkMap` | 471 | `4px 8px / 7` | `--xs` |
| `nbt/BackupPanel` | 226 | `5px 9px` | `--xs` |
| `nbt/EnchantEditor` | 87 | `5px 10px / 8` | `--xs` |
| `nbt/ItemEditor` | 479 | `5px 10px / 8` | `--xs` |
| `nbt/NbtTreeEditor` | 399 | `5px 10px / 8` | `--xs` |
| `LogViewer` | 141 | `.mini` `4px 10px / 7` | `--xs` |

> 注意：`.mini-btn` 在 NBT 系列里是**表单内嵌按钮**（同一行有输入框），归 `--xs` 保持行高；在视图页工具条里归 `--sm`。不要一刀切。

#### 13.3.2 主按钮的 hover / active

- 17 处 `filter: brightness(1.08)`：`CreateInstanceView:1050`、`HomeView:288`、`InstanceDetailView:812`、`InstancesView:534`、`MultiplayerView:1315`、`ProjectDetailView:608`、`ServerDetailView:1049`、`SettingsView:3033`、`SkinView:1332`、`FriendsPanel:769`、`ProjectCard:439`、`QookixLoginDialog:377`、`RoomDirectoryPanel:929`、`ShimuPostCard:453`、`SideBar:324/524`、`TitleBar:295`。
  → 全部删除，hover 改为 `background: var(--accent-hover)`（渐变按钮改为 `--accent-deep` 或保持渐变但换色标）。
- `FriendsPanel:772`、`QookixLoginDialog:380`、`ProjectCard:188`、`ShimuPostCard:175`、`RoomDirectoryPanel:932` 的 `transform: scale(.97)`：
  → 删除组件内定义。按压反馈统一由 [styles.css](file:///i:/program/vibe/mc/QookiX-Launcher/src/styles.css#L342-L351) 已有的 `.clickable:active` 提供，需要按压缩放的元素挂 `.clickable` 即可，**不再各写一份**。
- `InstanceCard:451` 的 `translateY(-4px) scale(.97)`：hover 位移违反 §7，改为 `box-shadow: var(--card-shadow-hover)`。
- `RoomDirectoryPanel` 的 `.rd-room:hover { translateY(-2px) }`：同上。
- `ColorPicker` 的 `.cp-preset:hover { scale(1.15) }`：改为 `box-shadow: 0 0 0 2px var(--accent)`。

**PR-3 验收**

```powershell
Select-String -Path src\**\*.vue -Pattern "filter:\s*brightness"      # 期望 0
Select-String -Path src\**\*.vue -Pattern "^\.mini-btn"               # 期望 0
```

---

### 13.4 PR-4：分段控件 6 → 1

目标规格见 §5.4（容器 `padding 4 / radius 10 / --w-05`，项 `28px / 0 16px / 13px`，指示器 `--r-sm / 0.26s`）。改造点：

| 文件 | 行 | 现状 | 动作 |
|---|---|---|---|
| `NBTEditorView` | 919 `.seg` | 第 3 套尺寸 | 换 `useSlidingIndicator` + `.qx-seg` |
| `SettingsView` | 3419 `.seg` | 第 4 套 | 同上 |
| `SkinView` | 1264 `.seg` | `radius 8 / indicator 6 / 11px` | 同上 |
| `TimeMachineView` | 1608 `.seg` | `radius 9 / indicator 7 / 高 38` | 同上 |
| `nbt/ChunkEditor` | 161 `.seg-group` + 168 `.seg` | `9 / 7 / 4px 12px` | 同上 |
| `RoomDirectoryPanel` | `.rd-subtabs` | 容器 10 / 项 8 | 同上 |
| `instance/SavesTab` | 561 `.seg` | **pill 按钮组**，非分段控件 | 更名 `.pill-group`，不进 composable |

同时把 §5.4 里的手写 indicator（ServerDetail `.tabs`、InstanceDetail `.tabs`、Multiplayer `.mode-tabs`、DownloadsDrawer `.tabs`、Browse 类型切换）一并迁到 composable——同一个 PR 内一次改完，否则会出现"新规格和旧规格并存"的中间态。

---

### 13.5 PR-5：页面骨架 ⚠️ 整节作废，按 §4 执行

| 文件 | 现状 | 动作 |
|---|---|---|
| `BrowseView` | 无标题、无宽度上限 | 加 `.qx-page` + `.qx-page-head`，标题 20px |
| `InstancesView` | 无标题 | 同上 |
| `ToolboxView` | 无标题，`grid 340` | 同上 |
| `MultiplayerView` | 无标题 | 同上 |
| `TimeMachineView` | 无标题 | 同上 |
| `HomeView` | 有标题，`max-width: 1080` | 改 1120 |
| `InstanceDetailView` | `h1 21px` | 改 20px + `.qx-page-head` |
| `ServerDetailView` | `h2 20px` | 改 `h1` + `.qx-page-title` |
| `ProjectDetailView` | 标题 ≈20px 但与工具条混排 | 拆 `.qx-page-head` + `.qx-toolbar` |

统一模板：

```vue
<div class="qx-page">
  <div class="qx-page-head">
    <div>
      <h1 class="qx-page-title"><IconXxx />{{ t("page.title") }}</h1>
      <div class="qx-page-sub">{{ t("page.subtitle") }}</div>
    </div>
    <div class="qx-page-actions"><button class="qx-btn qx-btn--md qx-btn--primary">…</button></div>
  </div>
  <div class="qx-toolbar">…</div>
  <div class="qx-page-body">…</div>
</div>
```

`InstanceDetail / ProjectDetail / ServerDetail` 的头部大卡保留 `.glass`，但标题必须用 `.qx-page-title`，不允许再出现 21px。

---

### 13.6 PR-6：弹窗 / 抽屉 / 层级

#### 13.6.1 弹窗宽度归 4 档

| 文件:行 | 现值 | 改为 |
|---|---|---|
| `CloudSyncDialog:475` | 600 | 680 |
| `CreateInstanceView:612` | `max-width 640` | 680 |
| `AddAccountDialog:271` | 480 | 560 |
| `NBTEditorView:795` | `max-width 460` | 420 |
| `ResourcePackPreviewView:601` | `max-width 460` | 420 |
| `MultiplayerView:1078 / 1231` | 460 | 420 |
| `MultiplayerView:1144` | 560 | ✓ |
| `InstancesView:401` | 380 | 420 |
| `DiagnosticsDialog:124` | 720 | 780 |
| `SeedMapView:1787` | `max-width 520` | 560 |
| `ServerDetailView:919` | 780 | ✓ |
| `FriendsPanel:428` | 380 | 420 |
| `FrpDialog:23` | 700 | 780 |
| `FrpPanel:490` | 720 | 780 |
| `IconPickerDialog:98` | 560 | ✓ |
| `TimeMachineView:1328 / 1511` | 620 / 520 | 680 / 560 |
| `InstallChecklist:201` | 580 | 560 |
| `SkinView:922` | `max-width 440` | 420 |
| `ExportDialog:220` | 640 | 680 |
| `instance/ContentTab:640` | 520 | 560 |
| `nbt/BackupPanel:117` | `max-width 620` | 680 |
| `QookixLoginDialog:178` / `ShimuLoginDialog:107` | 460 | 420 |
| `RoomDirectoryPanel:564` | 520 | 560 |
| `ForceUpdateOverlay` | 440（非 NModal） | 沿用 14px 圆角，宽度改 420 |

同时把所有 `max-width: 94vw / 95vw` 统一改为 `92vw`；只写 `max-width` 没写 `width` 的（CreateInstance、NBT、ResourcePack、SeedMap、SkinView、BackupPanel）补上对应档位的 `width`。

#### 13.6.2 z-index 归 7 层

只处理**全局浮层**；卡片内部为了盖住相邻卡片的 `z-index: 1/2` 属于局部堆叠，保留。

| 现值 | 位置 | 改为 |
|---|---|---|
| 45 / 44 | `SideBar` | `--z-header` |
| 30 | `BrowseView`、`TimeMachineView` 吸附头 | `--z-header` |
| 40 | `MultiplayerView` 浮层 | `--z-popover` |
| 50 / 60 | `InstanceCard` 悬停 | 局部 `1 / 2`（不参与全局层级） |
| 90 / 95 | `AccountDrawer`、`DownloadsDrawer` | `--z-drawer` |
| 100 | `ServerDetailView` | `--z-drawer` |
| 900 | `FileManager`、`ServerFileManager` | `--z-modal` |
| 1200 | `ContextMenu` | `--z-popover` |
| 1400 | `ModpackDropOverlay` | `--z-modal` |
| 2000 | `ColorPicker`、`SkinView`、`NbtTreeEditor` | `--z-popover` |
| 2500 | `InstanceDetailView` | `--z-modal` |
| 3000 | `ImageViewer`、`TimeMachineLightbox` | `--z-lightbox` |
| 9999 | `SplashScreen`、`LaunchProgress` | `--z-boot` |

#### 13.6.3 抽屉

`AccountDrawer` 380 → 420；两个抽屉的遮罩统一为 `rgba(0,0,0,.45)` + `backdrop-filter: blur(6px)`，与 NModal 一致。

---

### 13.7 PR-7：网格、颜色与收尾

#### 13.7.1 网格归 4 档

| 现值 | 文件 | 档位 |
|---|---|---|
| 340 | `ToolboxView:101`、`SettingsView:3043` | `rich 320` |
| 330 | `BrowseView:1463` | `rich 320` |
| 300 | `MultiplayerView:1339` | `rich 320` |
| 280 | `InstancesView:609`、`HomeView:363`、`TimeMachineView:1874/1986`、`ServerDetailView:1514` | `card 280` ✓ |
| 260 | `RoomDirectoryPanel:783`、`FrpPanel:618`、`ShimuLibraryView:1079` | `card 280` |
| 240 | 同上三处 | `card 280` |
| 210 | `NBTEditorView:1009` | `tile 200` |
| 200 | `ProjectDetailView:970`、`InstanceDetailView:1015`、`SchematicPreviewView:696/729` | `tile 200` ✓ |
| 180 | `ResourcePackPreviewView:913` | `tile 200` |
| 168 | `TimeMachineView:1990/1994` | `tile 200` |
| 140 | `MultiplayerView:1562` | `tile 200` |
| 104 | `SkinView:1380` | `thumb 96` |
| 88 | `ResourcePackPreviewView:783` | `thumb 96` |
| 320 | `instance/SettingsTab:428` | ✓ |

固定尺寸网格不算列宽档位，保持显式写法：`IconPickerDialog:252`（`48px` 图标墙）、`nbt/ItemEditor:354`（`66px` 物品槽）。

#### 13.7.2 颜色

按优先级处理：

1. **深色子系统**（`CodeEditor`、`ImageViewer`、`TimeMachineLightbox`、`SplashScreen`、`LaunchProgress`）：这些组件恒为深色是合理的，但必须把硬编码色收敛为 `--k-*` / `--bg-0` 令牌，并**删除 `CodeEditor` 里对 `--accent` 的覆盖**（它现在把用户自定义主题色锁死成 `#e89a4b`）。
2. **语义色**：`#4caf50` / `#f44336` / `#e8a04b` / `#2196f3` / `#57c257` / `#f08a67` / `#7cb8f5` / `#3fb950` / `#4493f8` / `#a371f7` → `--success` / `--danger` / `--accent` / `--info` 及对应 `-14` 底色。
3. **主按钮文字色 `#1a1206`**（`FriendsPanel`、`QookixLoginDialog`）→ `var(--accent-contrast)`。同类还有 `LanSyncPanel` 的 `.ls-btn.primary` 写了 `color: var(--bg-0)`（深色主题下 `--bg-0` 是近黑，等于强调底上压深色字）——那两个按钮已改挂 `.qx-btn--sm` + `--primary/--danger`，本地按钮外观整段删除。
4. **品牌渐变**（`AppIcon` 与 `IconPickerDialog` 两份同一色系、数值不同）→ 抽到 `src/instance-icons.ts` 一处导出，两处引用。

#### 13.7.3 收尾项

- 6 个非 scoped：`InstallDialog`、`AddAccountDialog`、`IconPickerDialog`、`InstallChecklist`、`VersionPicker`、`AccountChip`（第二块）改为 `scoped` / `:deep()`，类名统一加组件前缀。
- 16 处小数字号（`10.5 / 11.5 / 12.5 / 13.5`）换成 `--fs-*`。
- `FileManager` 与 `ServerFileManager` 合并为一个组件的两个 props（`mode: "instance" | "server"`），删除重复样式；高度统一 `calc(100vh - 320px)`。
- 复选框统一：`ExportDialog` 的 `.cb`（15/4）与 `InstallChecklist` 的 `.ic-check` 合并为 `.qx-check`。
- 空状态 4 种变体 → `.qx-empty`。

---

### 13.8 每个 PR 的通用验收

```powershell
# 视觉：深浅主题各过一遍 8 个主页面（Home / Instances / Browse / InstanceDetail / Settings / Multiplayer / Toolbox / TimeMachine）
npm run build

# 每阶段结束跑一次，数字必须单调下降
(Select-String -Path src\**\*.vue -Pattern "rgba\(").Count
(Select-String -Path src\**\*.vue -Pattern "#[0-9a-fA-F]{3,6}\b").Count
(Select-String -Path src\**\*.vue -Pattern "font-size:\s*\d+(\.\d+)?px").Count
(Select-String -Path src\**\*.vue -Pattern "border-radius:\s*\d+px").Count
(Select-String -Path src\**\*.vue -Pattern "z-index:\s*\d+").Count
```

PR 描述里必须附上以上 5 个数字的前后对比，否则不合并。

---

### 13.9 风险与回滚

| 风险 | 处理 |
|---|---|
| 大规模替换误伤（尤其 `--accent` 相关） | 一次 PR 只改一类属性；改完立即构建 + 截图对比 |
| naive-ui 内部样式被覆盖 | 禁止用全局 `.n-*` 选择器改样式，主题差异只允许在 `theme.ts` 里处理 |
| 深色子系统被浅色令牌污染 | `CodeEditor` 已有 `html.light .ed` 反向覆盖，改造时必须保留该隔离，只替换其中的字面量 |
| 用户自定义主题色失效 | 删除组件内对 `--accent` / `--text-*` 的重新赋值（当前 `CodeEditor` 一处） |
| 一次性删掉本地 `.btn` 导致漏改 | 先替换引用类名、构建通过后再删本地定义，两步分开提交 |

### 13.10 本方案不做什么

- 不改布局结构、不动交互流程、不加新功能。
- 不替换现有 naive-ui 表单控件（只统一它们外层的容器尺寸）。
- 不重新设计配色方案——`--accent` 橙色体系和已有的 `--w-*` / `--k-*` 叠加体系保留。
- 不追求一次性全量替换；按 §13.0 顺序推进，每个 PR 都可独立上线。

---

## 14. 以 SettingsView 为基准的分页改造步骤（历史记录，勿照做）

### 14.0 先明确：能复用的是 5 层，不是那张侧栏

把 [SettingsView.vue](file:///i:/program/vibe/mc/QookiX-Launcher/src/views/SettingsView.vue#L1332-L1401) 拆开看，它其实由 5 层组成，**只有第 5 层是设置页专属**。其他页面按需取前 4 层，不要机械照搬左侧导航。

| 层 | 组成 | 源码位置 | 适用页面 |
|---|---|---|---|
| L1 页壳 | 列布局 + 内容宽度上限 1120 + 页间距 18 | `.settings-view` L2753、`.settings-body` L2966 | 全部 9 页 |
| L2 页头 | 标题 20px + 副标题 + 右侧操作区 | 目前**缺失**，需新增 | 8 页（Home 用分组标题） |
| L3 工具条 | 一行控件容器（搜索/筛选/切换） | `.nav-search-card` L2768、`.toolbar` | Browse / Instances / Multiplayer / TimeMachine / 三个详情页 |
| L4 卡片网格 | `.grid` + `.card` + 卡片内 `h3` | L3041 / L3047 / L3051 | Instances / Toolbox / Home / Browse 结果 / Multiplayer |
| L5 侧栏 | 搜索 + 分组导航 + 品牌页脚 + 锚点卡片 | L1334-L1398 | 仅 SettingsView；`ShimuLibrary` 可作为后续候选 |

**关键前提**：`.grid`(L3041)、`.card`(L3047)、`.choice-row`(L3394) 都是 `scoped` 私有类，其他页面根本用不了。所以第一步不是改页面，而是**把骨架提升为全局公共层**。

---

### 14.1 Step 0：骨架提升（必须先做，否则每个页面都会复制一份）

#### ① `styles.css` 新增公共骨架

```css
/* L1 页壳：宽度上限 + 间距，替代各页自己的 max-width / gap */
.qx-page {
  max-width: 1120px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 18px;
  width: 100%;
}

/* L2 页头：三种变体共用 */
.qx-page-head { display: flex; align-items: flex-end; justify-content: space-between; gap: 16px; }
.qx-page-head-main { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
.qx-page-head-side { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.qx-page-title { display: flex; align-items: center; gap: 10px; margin: 0;
  font-size: var(--fs-xl); font-weight: 600; line-height: 1.2; color: var(--text-1); }
.qx-page-sub { margin: 0; font-size: var(--fs-sm); color: var(--text-3); }

/* L3 工具条 */
.qx-toolbar { display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
  padding: 10px 12px; border-radius: var(--r-lg);
  background: var(--panel); border: 1px solid var(--border); }
.qx-toolbar-spacer { flex: 1; }

/* L4 卡片网格：只保留 320 一档（其余档位见 §6） */
.qx-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 16px; }
.qx-card { padding: 18px; border-radius: var(--r-lg); }
.qx-card > h3 { display: flex; align-items: center; gap: 8px;
  margin: 0 0 14px; font-size: var(--fs-md); font-weight: 600; color: var(--text-1); }

/* 设定行：左侧文案 + 右侧控件 */
.qx-choice-row { display: flex; align-items: center; justify-content: space-between;
  gap: 16px; margin-bottom: 14px; font-size: var(--fs-md); color: var(--text-2); }
.qx-choice-info { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.qx-choice-label { color: var(--text-1); font-weight: 500; }
.qx-choice-hint { margin: 0; font-size: var(--fs-sm); line-height: 1.5; color: var(--text-3); }
```

#### ② SettingsView 回填

1. `.settings-view` 加 `max-width: 1120px; margin: 0 auto;`（它当前是全宽，作为基准页自己也不合规）。
2. `.settings-view` 的 `gap: 16px` → `18px`。
3. `.grid` / `.card` / `.choice-row` 三个本地定义改为引用 `.qx-grid` / `.qx-card` / `.qx-choice-row`，删除本地定义（先替换类名、构建通过后再删）。
4. `.grid` 的 `minmax(340px)` → `320px`。

#### ③ 页头三种变体

```vue
<!-- A 标准页头：有主操作 -->
<div class="qx-page-head">
  <div class="qx-page-head-main">
    <h1 class="qx-page-title"><IconGrid />{{ t("instances.title") }}</h1>
    <p class="qx-page-sub">{{ t("instances.subtitle") }}</p>
  </div>
  <div class="qx-page-head-side">
    <button class="qx-btn qx-btn--md qx-btn--primary">…</button>
  </div>
</div>

<!-- B 返回式页头：详情页 -->
<div class="qx-page-head">
  <div class="qx-page-head-main">
    <button class="qx-btn qx-btn--xs qx-btn--ghost" @click="router.back()">
      <IconChevronLeft /> {{ t("common.back") }}
    </button>
    <h1 class="qx-page-title">{{ name }}</h1>
    <p class="qx-page-sub">{{ meta }}</p>
  </div>
  <div class="qx-page-head-side">…</div>
</div>
```

> `--content-pad-top: 22px` / `--content-pad-x: 26px` 已由 [App.vue](file:///i:/program/vibe/mc/QookiX-Launcher/src/App.vue#L308-L320) 统一提供，**页面不要再写 `padding-top`**，否则吸顶偏移会算错。

---

### 14.2 逐页改造步骤

#### 1）ToolboxView（最简单，先做，用于验证骨架）

1. 根节点 `.toolbox-view` 加 `.qx-page` 语义：`max-width:1120; margin:0 auto;`。
2. 在 `.grid` 前插入 A 型页头，标题用 `t("nav.toolbox")` 或现有 i18n 键，副标题可省略。
3. `.grid` 改用 `.qx-grid`，`minmax(340px)` → `320px`。
4. `.tool-card` 保留 `glass`，补 `border-radius: var(--r-lg)`，删除本地 `gap` 重复声明（由 `.qx-page` 提供）。
5. 卡片内 `h3` 改用 `.qx-card > h3` 规格（13px/600），`BETA` 标签用 `.qx-badge`。

#### 2）InstancesView

1. 根 `.instances-view` 加 `max-width:1120; margin:0 auto;`。
2. 顶部插入 A 型页头，右侧放「导入整合包 / 新建实例」主按钮（现在这两个按钮散在 toolbar 里）。
3. `PlaytimeCard` 放在页头之后、工具条之前，作为独立信息卡。
4. `.toolbar` 改为 `.qx-toolbar`；筛选 `chip` 高度统一 28px（`--ctl-sm`）、圆角 `--r-pill`。
5. 实例网格改 `.qx-grid`（280 档用 `.qx-grid--card`）。
6. `.loading` 文本替换为骨架屏（§8 要求），不要再只显示一行文字。

#### 3）MultiplayerView

1. 根 `.mp-view` 加 `max-width:1120; margin:0 auto;`。
2. 插入 A 型页头，标题「联机」；右侧放「刷新/添加服务器」。
3. `.mp-head` 从「标签即页头」改为「页头下一行的工具条」：外层 `.qx-toolbar` 只包 `.mode-tabs`，去掉 `.mp-head` 的自定义间距。
4. `.mode-tabs` / `.mp-subtabs` 迁到 `useSlidingIndicator`（PR-4），容器规格用 §5.4。
5. 服务器卡网格改 `.qx-grid`；`server-card` 删除 hover `translateY`，只改阴影。

#### 4）TimeMachineView

1. 根 `.timemachine-view` 加 `max-width:1120; margin:0 auto;`。
2. 插入 A 型页头，标题「时间机器」，副标题写当前实例名/快照数（现在这些信息只能从内容里猜）。
3. `.toolbar` 改 `.qx-toolbar`：左侧 `.seg` 切视图、中间 spacer、右侧刷新/截图按钮组。
4. `.seg` 迁 composable（当前第 2 套规格 `radius 9 / indicator 7 / 高 38`）。
5. 快照网格 `minmax(330/168/140)` 统一到 `.qx-grid`（320）或 `.qx-grid--tile`（200）。

#### 5）BrowseView（最难，放到骨架和工具条都稳定之后）

1. 根 `.browse` 加 `max-width:1120; margin:0 auto;`。
2. 插入 A 型页头（标题「发现」/「资源」，右侧放搜索框或排序）。
3. **保持 `.browse-head` 的吸顶能力**：
   - `top` 继续用 `calc(-1 * var(--content-pad-top, 22px))`；
   - `.head-glass` 的横向 bleed 由 `calc(-1 * var(--content-pad-x))` **改为 `0`**——根节点已经有 1120 上限，再向外扩 26px 会盖到内容区留白上；
   - `.pinned` 状态下的左右内边距改为 0。
4. `.type-card` 的分段控件迁 composable（当前手写 `indicator`，L882-L915）。
5. 结果网格 `minmax(330px)` → `320px`；列表项 hover 只改背景。
6. 校验：滚动到吸顶、切换分类、窗口宽度 900/1200/1600 三档下都不出现横向滚动条。

#### 6）InstanceDetailView

1. 根 `.detail-page` / `.detail` 合并为 `.qx-page`（现在套了两层，`detail-page` 只承担 padding）。
2. `.d-head` 保留 `glass`，圆角 18；内部 `h1` 从 21px 改为 `.qx-page-title`（20px）。
3. `.d-actions` 的主按钮、置顶按钮改用 `.qx-btn--lg` / `.qx-btn--md`，删除本地 `.btn` 定义。
4. 手写 `.tabs` + `.indicator` 迁 `useSlidingIndicator`，规格 §5.4。
5. 各 Tab 面板统一 `.qx-card`（当前 `ContentTab` 的 `.btn` 是 `9px 16px`、`KeybindTab` 的 `.mini-btn` 是 `7px 13px`，都在 PR-3 里收敛）。
6. 页头下方加一行 `.qx-page-sub`：`MC 版本 · 加载器 · 最后游玩时间`，把现在散在 badge 里的信息归位。

#### 7）ServerDetailView

1. 根 `.server-detail-page` + `.server-detail` 合并 `.qx-page`。
2. 把独立的 `.back` 按钮移进 B 型页头的 `qx-page-head-main`，删除 `.back` 的本地样式。
3. `.head glass` 保留为信息卡，`h2` → `h1.qx-page-title`；`core-badge` 用 `.qx-badge`。
4. `.tabs` 迁 `useSlidingIndicator`；`.panel.glass` 用 `.qx-card`。
5. `.field` / `.field-row` 统一为 `.qx-choice-row`，控件高度用 `--ctl-md`。
6. `CORE_COLORS` 硬编码色映射到 `--success/--info/--accent/--danger`。

#### 8）ProjectDetailView

1. 根 `.pd` 加 `.qx-page`。
2. 顶部 `.toolbar glass` 拆成 B 型页头：返回按钮在左、`viewOnSite` 在右侧 `qx-page-head-side`。
3. `.pd-head glass` 保留为信息卡；`.pd-title` 改 `.qx-page-title`，`.pd-meta` 用 `--fs-sm` + `--text-3`。
4. `.pd-head-actions` 的两个按钮改 `.qx-btn--md`，删除 `style="margin-left: auto"` 这类内联布局，交给 `.qx-page-head-side` 的 `gap`。
5. 依赖前置、文件列表等分块统一 `.qx-card` + `h3`。

#### 9）HomeView（唯一不套「单标题页头」的页面）

1. `.home` 加 `.qx-page`（宽度上限 + 间距 18）。
2. 保留 `OnboardingBar` 在最上方（它是流程条，不是页头）。
3. pin 区块与实例区块各加一个分组标题（`.qx-page-title` 降到 15px 的 `h2.qx-section-title`），解决「首页没有任何文字层级」的问题。
4. `.pin-card` / `.empty` 复用 `.qx-card`；删除本地 `.btn` 重定义（L270 起整套复制了全局按钮）。
5. 卡片 hover 保留阴影变化，删除 `translateY` / `scale`。

---

### 14.3 落地顺序与依赖

| 顺序 | 页面 | 原因 |
|---|---|---|
| 0 | 骨架提升（§14.1） | 所有页面的前置 |
| 1 | ToolboxView | 结构最简单，验证 `.qx-grid` / 页头是否够用 |
| 2 | InstancesView | 验证网格 + 工具条 + 空状态 |
| 3 | MultiplayerView | 验证「页头 + 分段控件」组合 |
| 4 | TimeMachineView | 验证工具条右侧按钮组 |
| 5 | BrowseView | 唯一有吸顶逻辑，必须等前面稳定 |
| 6 | InstanceDetailView | 验证详情页 B 型页头 |
| 7 | ServerDetailView | 同上 + 表单行 |
| 8 | ProjectDetailView | 同上 |
| 9 | HomeView | 结构特殊，最后单独处理 |
| 10 | SettingsView 回填 | 基准页自身补齐宽度上限与公共类 |

每完成一页，把该页的「本地 `.btn` / `.grid` / `.card` / 手写 indicator」定义删除；否则公共类和本地类会同时生效，出现双重样式覆盖。

---

### 14.4 迁移时最容易踩的 4 个坑

1. **吸顶条偏移算错**：`.content` 的 `padding-top` 是 22px，`sticky` 必须写负偏移；页面自己再加 `padding-top` 会让吸附位置下移。
2. **加宽度上限后磨砂露边**：`.browse-head` 的玻璃底衬原本横向外扩 `--content-pad-x`，加 `max-width` 后必须改成 0，否则会在左右留白上盖出一条玻璃。
3. **`backdrop-filter` + 动画**：SettingsView 已验证入场动画不能用 `opacity` / `transform`，否则磨砂失效（L2973-L2979 的注释）。新页面照抄这套 `top` 位移方案，不要改回淡入。
4. **删除本地类顺序**：先替换模板里的类名 → 构建通过 → 再删 `<style>` 里的定义。直接删定义会先白屏再排查。

---

### 14.5 每页验收清单（截图对照）

- [ ] 根节点有 `max-width: 1120px; margin: 0 auto`
- [ ] 页头标题唯一且为 20px，副标题 12px
- [ ] 工具条高度与内边距一致，右侧按钮不换行
- [ ] 网格列宽只出现 320 / 280 / 200 / 96 之一（固定尺寸网格除外）
- [ ] 卡片 hover 无位移、无缩放
- [ ] 深浅主题各截一张
- [ ] 窗口 900 / 1200 / 1600px 三档无横向滚动条
- [ ] 该页已无本地 `.btn` / `.grid` / `.card` / 手写 `.indicator` 定义
