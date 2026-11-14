import type { HomeLayout, HomeNode, HomeVars } from "./types";

/** 布局文件解析失败时抛这个，页面会把 message 直接展示给用户。 */
export class HomeLayoutError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "HomeLayoutError";
  }
}

/* -------------------------------------------------------------------------- */
/* 默认布局                                                                    */
/* -------------------------------------------------------------------------- */

/**
 * 首次运行时会写到数据目录下的 `home.json`。
 * 刻意保留注释：用户打开文件就能看到写法说明，删掉整个文件即可恢复默认。
 */
export const DEFAULT_LAYOUT_SOURCE = String.raw`/* ============================================================================
   QookiX 自定义主页布局文件
   ----------------------------------------------------------------------------
   这个文件决定首页显示哪些卡片、文字和按钮，改完到「设置 → 外观 → 自定义主页」
   点「重新加载」即可生效（也可以在那里关闭自定义主页、恢复默认模板）。
   如果你把文件改坏了，删掉它，启动器会重新生成这份默认内容。

   · 文件是 JSON 格式，但允许双斜杠行注释和 C 风格块注释，方便你写备注。
   · 根节点是 { "variables": {...}, "nodes": [...] }，nodes 里每一项都是一个组件。
   · 可用组件（写在 type 里）：
       card       卡片，可带 title / collapsible / collapsed，children 放内容
       text       文本，可设 size / bold / muted / color / align / width
       hint       提示条，theme 可选 blue / yellow / red
       image      图片，src 支持网址或本地绝对路径，可设 height / width / align
       button     按钮，variant 可选 primary / ghost / danger，onClick 触发事件
       buttons    一排按钮，自动换行
       grid       等宽网格，columns 写 CSS 的列宽，如 ["1fr", "1.6fr", "150px"]
       row        横向排列
       list-item  列表项，logo / title / info，带 onClick 才可点击
       divider    分割线
       spacer     空白，size 为高度像素
   · 每个组件都可以加 "if": "变量名" 或 "!变量名" 控制显示；变量取自 variables，
     按钮的 set-var 事件可以实时改变它。
   · 文本里可以写 {user}（当前账号）、{instance}（当前实例）、{date}、{time}、
     {homePath}（本文件路径）等占位符，会被自动替换。
   ========================================================================== */

{
  "version": 1,
  "variables": {
    "showTip": true
  },
  "nodes": [
    {
      "type": "card",
      "title": "欢迎回来，{user}",
      "children": [
        {
          "type": "text",
          "text": "现在是 {date} {time}，当前实例：{instance}。"
        },
        {
          "type": "hint",
          "theme": "blue",
          "text": "这份主页是数据目录下的 home.json 渲染出来的。想改成自己的样子，看最后一张卡片。"
        },
        {
          "type": "spacer",
          "size": 12
        },
        {
          "type": "buttons",
          "align": "left",
          "children": [
            {
              "type": "button",
              "text": "启动游戏",
              "variant": "primary",
              "onClick": { "type": "launch" }
            },
            {
              "type": "button",
              "text": "发现内容",
              "onClick": { "type": "navigate", "to": "/browse" }
            },
            {
              "type": "button",
              "text": "工具箱",
              "onClick": { "type": "navigate", "to": "/toolbox" }
            }
          ]
        }
      ]
    },
    {
      "type": "card",
      "title": "快捷操作",
      "collapsible": true,
      "collapsed": true,
      "children": [
        {
          "type": "text",
          "text": "按钮可以干很多事：打开网页、复制文本、启动实例、站内跳转、弹出提示，甚至连续触发多个事件。"
        },
        {
          "type": "buttons",
          "align": "left",
          "children": [
            {
              "type": "button",
              "text": "复制召唤命令",
              "tooltip": "复制一条会在每个玩家头顶生成苦力怕的指令",
              "onClick": {
                "type": "copy",
                "text": "execute as @a at @s run summon minecraft:creeper ~ ~2 ~"
              }
            },
            {
              "type": "button",
              "text": "打开 Minecraft Wiki",
              "onClick": { "type": "open-url", "url": "https://zh.minecraft.wiki/" }
            },
            {
              "type": "button",
              "text": "弹个窗",
              "onClick": {
                "type": "dialog",
                "title": "自定义主页",
                "content": "这个弹窗由 home.json 里的 dialog 事件触发。"
              }
            }
          ]
        }
      ]
    },
    {
      "type": "card",
      "title": "变量与条件显示",
      "children": [
        {
          "type": "text",
          "text": "按钮可以修改变量，带 if 的组件会跟着出现或消失——这就是 PCL 里「修改变量 + 刷新页面」的玩法。"
        },
        {
          "type": "buttons",
          "align": "left",
          "children": [
            {
              "type": "button",
              "text": "显示提示",
              "if": "!showTip",
              "onClick": { "type": "set-var", "name": "showTip", "value": "true" }
            },
            {
              "type": "button",
              "text": "隐藏提示",
              "if": "showTip",
              "onClick": { "type": "set-var", "name": "showTip", "value": "false" }
            }
          ]
        },
        {
          "type": "hint",
          "theme": "yellow",
          "if": "showTip",
          "text": "提示已显示。变量会记在本机，下次打开首页仍然有效。"
        }
      ]
    },
    {
      "type": "card",
      "title": "定制这份主页",
      "children": [
        {
          "type": "list-item",
          "title": "编辑 home.json",
          "info": "改完到「设置 → 外观 → 自定义主页」点「重新加载」",
          "onClick": { "type": "open-path", "path": "{homePath}" }
        },
        {
          "type": "divider"
        },
        {
          "type": "text",
          "muted": true,
          "text": "文件路径：{homePath}。在「设置 → 外观 → 自定义主页」点「恢复默认」，会重新生成一份带注释的模板。"
        }
      ]
    }
  ]
}
`;

/* -------------------------------------------------------------------------- */
/* 解析                                                                        */
/* -------------------------------------------------------------------------- */

/** 去掉 // 与 /* *\/ 注释，但跳过字符串内部（URL 里的 // 不会被误删）。 */
export function stripJsonComments(src: string): string {
  let out = "";
  let inString = false;
  let escaped = false;
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    if (inString) {
      out += c;
      if (escaped) escaped = false;
      else if (c === "\\") escaped = true;
      else if (c === '"') inString = false;
      continue;
    }
    if (c === '"') {
      inString = true;
      out += c;
      continue;
    }
    if (c === "/" && src[i + 1] === "/") {
      while (i < src.length && src[i] !== "\n") i++;
      out += "\n";
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      i += 2;
      while (i < src.length && !(src[i] === "*" && src[i + 1] === "/")) i++;
      i++;
      continue;
    }
    out += c;
  }
  return out;
}

/** 容忍用户手写时多出来的尾逗号（同样是字符串感知的）。 */
function stripTrailingCommas(src: string): string {
  let out = "";
  let inString = false;
  let escaped = false;
  for (let i = 0; i < src.length; i++) {
    const c = src[i];
    if (inString) {
      out += c;
      if (escaped) escaped = false;
      else if (c === "\\") escaped = true;
      else if (c === '"') inString = false;
      continue;
    }
    if (c === '"') {
      inString = true;
      out += c;
      continue;
    }
    if (c === ",") {
      let j = i + 1;
      while (j < src.length && /\s/.test(src[j])) j++;
      if (src[j] === "}" || src[j] === "]") continue;
    }
    out += c;
  }
  return out;
}

const NODE_TYPES = [
  "card",
  "text",
  "hint",
  "image",
  "button",
  "buttons",
  "grid",
  "row",
  "list-item",
  "divider",
  "spacer",
] as const;

function normalizeNodes(raw: unknown, where: string): HomeNode[] {
  if (raw == null) return [];
  if (!Array.isArray(raw)) throw new HomeLayoutError(`${where} 应该是数组`);
  return raw.map((item, i) => normalizeNode(item, `${where}[${i}]`));
}

function normalizeNode(raw: unknown, where: string): HomeNode {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    throw new HomeLayoutError(`${where} 应该是一个对象，比如 { "type": "text", "text": "..." }`);
  }
  const obj = raw as Record<string, unknown>;
  const type = String(obj.type ?? "");
  if (!(NODE_TYPES as readonly string[]).includes(type)) {
    throw new HomeLayoutError(
      `${where} 的 type「${type || "(空)"}」无法识别，可用：${NODE_TYPES.join(" / ")}`,
    );
  }
  if ((type === "text" || type === "hint" || type === "button") && typeof obj.text !== "string") {
    throw new HomeLayoutError(`${where} 缺少 text 字段`);
  }
  if (type === "image" && typeof obj.src !== "string") {
    throw new HomeLayoutError(`${where} 缺少 src 字段`);
  }
  if (type === "list-item" && typeof obj.title !== "string") {
    throw new HomeLayoutError(`${where} 缺少 title 字段`);
  }
  const node = { ...obj, type } as HomeNode;
  if ("children" in obj) {
    (node as { children?: HomeNode[] }).children = normalizeNodes(obj.children, `${where}.children`);
  }
  return node;
}

/** 解析布局文本（含注释 / 尾逗号容错）。失败时抛 HomeLayoutError。 */
export function parseHomeLayout(text: string): HomeLayout {
  if (!text.trim()) throw new HomeLayoutError("布局文件是空的");
  const body = stripTrailingCommas(stripJsonComments(text));
  let data: unknown;
  try {
    data = JSON.parse(body);
  } catch (e) {
    throw new HomeLayoutError(`JSON 语法错误：${(e as Error).message}`);
  }
  // 允许用户只写一个节点数组
  const root = Array.isArray(data) ? { nodes: data } : data;
  if (!root || typeof root !== "object") throw new HomeLayoutError("根节点必须是对象或数组");
  const obj = root as Record<string, unknown>;
  const nodes = normalizeNodes(obj.nodes, "nodes");
  if (!nodes.length) throw new HomeLayoutError("nodes 里没有任何组件");

  const variables: HomeVars = {};
  if (obj.variables && typeof obj.variables === "object" && !Array.isArray(obj.variables)) {
    for (const [k, v] of Object.entries(obj.variables as Record<string, unknown>)) {
      variables[k] = String(v);
    }
  }
  return {
    version: typeof obj.version === "number" ? obj.version : 1,
    title: typeof obj.title === "string" ? obj.title : undefined,
    variables,
    nodes,
  };
}

/* -------------------------------------------------------------------------- */
/* 变量与占位符                                                                */
/* -------------------------------------------------------------------------- */

const USER_VARS_KEY = "qookix.home.vars";
/** 是否启用自定义主页（本地偏好，不进设置文件）；默认关闭，只有显式写 "1" 才开启 */
export const HOME_CUSTOM_KEY = "qookix.home.custom";
/**
 * 首页「最近游玩」兜底内容是否显示（本地偏好，不进设置文件）。
 * 默认显示，只有显式写 "0" 才关闭——喜欢干净空背景的用户可以把它关掉。
 */
export const HOME_RECENT_KEY = "qookix.home.recent";

export function loadUserVars(): HomeVars {
  try {
    const raw = localStorage.getItem(USER_VARS_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: HomeVars = {};
    for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) out[k] = String(v);
    return out;
  } catch {
    return {};
  }
}

export function saveUserVars(vars: HomeVars) {
  try {
    localStorage.setItem(USER_VARS_KEY, JSON.stringify(vars));
  } catch {
    /* 存储不可用时变量只在本次会话生效 */
  }
}

/** 恢复默认布局时一并清掉用户改过的变量 */
export function clearUserVars() {
  try {
    localStorage.removeItem(USER_VARS_KEY);
  } catch {
    /* 忽略 */
  }
}

/** 把 `{name}` 换成变量值；未知占位符原样保留，方便用户发现自己写错了。 */
export function interpolate(text: string, vars: HomeVars): string {
  if (!text) return "";
  return text.replace(/\{(\w+)\}/g, (whole, key: string) =>
    key in vars ? vars[key] : whole,
  );
}

/** 条件求值：支持 "name" / "{name}" / "!name"，也允许直接写 true/false。 */
export function evalCond(expr: string | undefined, vars: HomeVars): boolean {
  if (!expr) return true;
  let e = expr.trim();
  if (e.startsWith("{") && e.endsWith("}")) e = e.slice(1, -1);
  let negate = false;
  if (e.startsWith("!")) {
    negate = true;
    e = e.slice(1).trim();
  }
  const raw = e in vars ? vars[e] : e;
  const truthy = !["", "0", "false", "null", "undefined", "off", "no"].includes(raw.toLowerCase());
  return negate ? !truthy : truthy;
}

export function nodeVisible(node: HomeNode, vars: HomeVars): boolean {
  if (node.if && !evalCond(node.if, vars)) return false;
  if (node.unless && evalCond(node.unless, vars)) return false;
  return true;
}

export function formatDate(d = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

export function formatTime(d = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
