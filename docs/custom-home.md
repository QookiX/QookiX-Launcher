# 自定义主页（home.json）

> 类似 PCL 的自定义主页：用一份可编辑的 JSON 描述首页由哪些卡片、文本、按钮组成。
> **默认关闭**，需先在「设置 → 外观 → 自定义主页」打开「启用自定义主页」；
> 改完文件后点同页的「重新加载」即可生效，不用改源码、不用重启。
> 首页本身不放任何配置按钮，所有开关和文件操作都在设置页。

---

## 1. 文件在哪

| 项 | 说明 |
|---|---|
| 路径 | 数据目录根下的 `home.json` |
| 配置入口 | 「设置 → 外观 → 自定义主页」 |
| 默认状态 | **关闭**，首页显示固定布局；到设置页打开开关后才渲染 `home.json` |
| 打开方式 | 设置页「编辑布局文件」（调用系统默认程序打开） |
| 生效方式 | 设置页「重新加载」 |
| 恢复默认 | 设置页「恢复默认」（删除文件后重新生成带注释的模板，需二次确认） |
| 临时关闭 | 设置页「启用自定义主页」开关（记在本机偏好，不影响文件） |

数据目录可以在「设置 → 常规 → 数据目录」里查看，默认是 `%APPDATA%\QookiX-Launcher`。

文件不存在时，启动器会在首次进入首页或设置页时写入一份带注释的完整示例。**把文件改坏了直接删掉**，下次会重新生成。

---

## 2. 文件结构

```jsonc
{
  "version": 1,
  "variables": { "showTip": true },   // 自定义变量，可被按钮修改
  "nodes": [                          // 页面从上到下依次渲染
    { "type": "card", "title": "标题", "children": [ /* 子节点 */ ] }
  ]
}
```

- 允许 `//` 行注释与 `/* */` 块注释，也容忍多余的尾逗号（解析前会自动清理）。
- `nodes` 也可以直接写成数组（省略外层对象）。

---

## 3. 节点类型

| type | 作用 | 主要字段 |
|---|---|---|
| `card` | 玻璃卡片 | `title`、`collapsible`、`collapsed`、`children` |
| `text` | 文本 | `text`、`size`、`bold`、`muted`、`color`、`align`、`width` |
| `hint` | 提示条 | `text`、`theme`（`blue` / `yellow` / `red`） |
| `image` | 图片 | `src`、`fallback`、`height`、`width`、`align` |
| `button` | 按钮 | `text`、`variant`（`primary` / `ghost` / `danger`）、`tooltip`、`disabled`、`onClick` |
| `buttons` | 一排按钮（自动换行） | `align`、`children` |
| `grid` | 等宽网格 | `columns`、`children` |
| `row` | 横向排列 | `align`、`gap`、`children` |
| `list-item` | 两行列表项 | `logo`、`title`、`info`、`onClick` |
| `divider` | 分割线 | — |
| `spacer` | 空白 | `size`（像素，默认 8） |

通用字段：

- `if`：变量为真才渲染，写 `"showTip"` 或 `"!showTip"`。
- `unless`：变量为假才渲染。
- `align`：`left` / `center` / `right`。

`grid.columns` 按 CSS `grid-template-columns` 逐项写，例如：

```json
{ "type": "grid", "columns": ["1fr", "1.6fr", "150px"], "children": [ /* 三个按钮 */ ] }
```

只接受 `auto` 与「数字 + `fr` / `px` / `%` / `em` / `rem`」，写错会回落到自适应栅格。

---

## 4. 事件（onClick）

`onClick` 可以是单个事件对象，也可以是数组（按顺序连续执行）。

| type | 作用 | 字段 |
|---|---|---|
| `open-url` | 用浏览器打开网址 | `url` |
| `copy` | 复制文本到剪贴板 | `text` |
| `dialog` | 弹出提示框 | `title`、`content` |
| `launch` | 启动实例 | `instance`（实例名，省略则用最近游玩的）、`world`、`server` |
| `navigate` | 站内跳转 | `to`（如 `/browse`、`/instances`、`/settings`） |
| `open-path` | 用系统程序打开文件/文件夹 | `path` |
| `set-var` | 改变量并即时重渲染 | `name`、`value` |
| `reload` | 重新读取 home.json | — |

示例：一个按钮连续做三件事——修改变量、折叠卡片、刷新布局。

```json
{
  "type": "button",
  "text": "收起提示",
  "onClick": [
    { "type": "set-var", "name": "showTip", "value": "false" },
    { "type": "reload" }
  ]
}
```

---

## 5. 占位符

文本、提示、按钮文字、`title`、`url`、`path` 等字段都可以写占位符，渲染时自动替换。

| 占位符 | 含义 |
|---|---|
| `{user}` | 当前账号名（未登录时为「玩家」） |
| `{instance}` | 最近游玩的实例名 |
| `{date}` | 当天日期，如 `2026-11-13` |
| `{time}` | 当前时间，如 `21:08`（每分钟更新） |
| `{homePath}` | 本文件的绝对路径 |

自定义变量（写在 `variables` 里或被 `set-var` 修改）同样用 `{变量名}` 引用。
变量值会持久化在本机，下次打开首页仍然有效；`set-var` 只保存自定义变量，内置占位符不落盘。

---

## 6. 一个最小可用的例子

```jsonc
{
  "variables": { "showTip": true },
  "nodes": [
    {
      "type": "card",
      "title": "欢迎回来，{user}",
      "children": [
        { "type": "text", "text": "当前实例：{instance}，今天是 {date}。" },
        { "type": "hint", "theme": "blue", "text": "这一整页都来自 home.json。" },
        { "type": "buttons", "children": [
          { "type": "button", "text": "启动游戏", "variant": "primary", "onClick": { "type": "launch" } },
          { "type": "button", "text": "发现内容", "onClick": { "type": "navigate", "to": "/browse" } }
        ] }
      ]
    },
    {
      "type": "card",
      "title": "条件显示",
      "children": [
        { "type": "button", "text": "显示提示", "if": "!showTip",
          "onClick": { "type": "set-var", "name": "showTip", "value": "true" } },
        { "type": "button", "text": "隐藏提示", "if": "showTip",
          "onClick": { "type": "set-var", "name": "showTip", "value": "false" } },
        { "type": "hint", "theme": "yellow", "if": "showTip", "text": "提示出现啦。" }
      ]
    }
  ]
}
```

---

## 7. 实现位置（改代码时看这里）

| 文件 | 职责 |
|---|---|
| `src/home/types.ts` | 节点 / 事件 / 布局的 TypeScript 定义 |
| `src/home/layout.ts` | 默认模板、注释清洗、解析校验、变量与占位符、条件求值 |
| `src/home/useHomeLayout.ts` | 首页与设置页共享的状态：读取 / 保存 / 重载 / 恢复默认 / 启用开关 |
| `src/views/SettingsView.vue` | 配置入口（外观页的开关、编辑、重载、恢复默认） |
| `src/views/HomeView.vue` | 只消费共享状态并渲染布局 |
| `src/home/context.ts` | 渲染器上下文（递归组件注入用） |
| `src/components/home/HomeLayout.vue` | 布局入口 + 事件执行 + 变量表 |
| `src/components/home/HomeNode.vue` | 递归渲染单个节点 |
| `src-tauri/src/commands/home.rs` | `home.json` 的后端读写命令 |
| `src/styles/tokens.css` | 设计令牌（颜色 / 间距 / 圆角 / 字号 / 层级） |
| `src/styles/components.css` | `.qx-*` 可复用组件类 |

新增节点类型时：`types.ts` 加联合成员 → `layout.ts` 的 `NODE_TYPES` 与校验 → `HomeNode.vue` 加一个分支。
