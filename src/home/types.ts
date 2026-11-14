/**
 * 自定义主页布局的数据结构。
 *
 * 设计目标是「PCL 风格」：用户在数据目录下的 `home.json` 里用纯 JSON
 * （允许 `//` 与 `/* *\/` 注释）描述首页由哪些卡片、文本、按钮组成，
 * 改完点一下「重新加载」就能看到效果，不需要动源码。
 *
 * 所有节点都支持 `if` / `unless` 做条件显示，条件值取自布局里的 variables。
 */

export type HomeAlign = "left" | "center" | "right";
export type HomeButtonVariant = "default" | "primary" | "ghost" | "danger";
export type HomeHintTheme = "blue" | "yellow" | "red";

/** 按钮 / 列表项点击后能做的事。字段按 type 取用，未用到的可省略。 */
export interface HomeAction {
  /** open-url 打开网页 / copy 复制文本 / dialog 弹窗 / launch 启动实例
   *  navigate 站内跳转 / open-path 用系统程序打开文件 / set-var 改变量 / reload 重载布局 */
  type:
    | "open-url"
    | "copy"
    | "dialog"
    | "launch"
    | "navigate"
    | "open-path"
    | "set-var"
    | "reload";
  /** open-url */
  url?: string;
  /** copy */
  text?: string;
  /** dialog */
  title?: string;
  content?: string;
  /** launch：实例名（省略则用最近游玩的实例）、world 存档名、server 服务器地址 */
  instance?: string;
  world?: string;
  server?: string;
  /** navigate：站内路由，如 /browse */
  to?: string;
  /** open-path：绝对路径，支持 {变量} */
  path?: string;
  /** set-var：变量名与值 */
  name?: string;
  value?: string;
}

interface HomeNodeBase {
  /** 条件显示：变量为真才渲染，`!name` 取反 */
  if?: string;
  /** 与 if 相反：变量为假才渲染 */
  unless?: string;
}

export interface HomeCardNode extends HomeNodeBase {
  type: "card";
  title?: string;
  /** 可折叠；默认 false */
  collapsible?: boolean;
  /** 初始是否折叠 */
  collapsed?: boolean;
  children?: HomeNode[];
}

export interface HomeTextNode extends HomeNodeBase {
  type: "text";
  text: string;
  /** 字号（px），默认 13 */
  size?: number;
  bold?: boolean;
  /** 次要文字（灰色） */
  muted?: boolean;
  /** 自定义颜色，如 "#8c7721" 或 "var(--accent)"；优先于 muted */
  color?: string;
  align?: HomeAlign;
  /** 限制最大宽度（px） */
  width?: number;
}

export interface HomeHintNode extends HomeNodeBase {
  type: "hint";
  text: string;
  theme?: HomeHintTheme;
}

export interface HomeImageNode extends HomeNodeBase {
  type: "image";
  /** http(s) 链接、data URL，或本地绝对路径 */
  src: string;
  /** 加载失败时的备用地址 */
  fallback?: string;
  height?: number;
  width?: number;
  align?: HomeAlign;
}

export interface HomeButtonNode extends HomeNodeBase {
  type: "button";
  text: string;
  variant?: HomeButtonVariant;
  tooltip?: string;
  /** 固定宽度（px）；不填按文字自适应 */
  width?: number;
  align?: HomeAlign;
  disabled?: boolean;
  /** 可写单个事件，或事件数组（按顺序连续执行） */
  onClick?: HomeAction | HomeAction[];
}

/** 一行或多行按钮，自动换行并保持等宽 */
export interface HomeButtonsNode extends HomeNodeBase {
  type: "buttons";
  align?: HomeAlign;
  /** 每行最少放几个（默认按可用宽度自动排布） */
  columns?: number;
  children?: HomeNode[];
}

/** 等宽的网格布局，columns 写法同 CSS grid-template-columns 的单项 */
export interface HomeGridNode extends HomeNodeBase {
  type: "grid";
  columns?: string[];
  children?: HomeNode[];
}

export interface HomeListItemNode extends HomeNodeBase {
  type: "list-item";
  logo?: string;
  title: string;
  info?: string;
  onClick?: HomeAction | HomeAction[];
}

export interface HomeDividerNode extends HomeNodeBase {
  type: "divider";
}

export interface HomeSpacerNode extends HomeNodeBase {
  type: "spacer";
  /** 高度（px），默认 8 */
  size?: number;
}

/** 横向排列（按钮、图片混排时用） */
export interface HomeRowNode extends HomeNodeBase {
  type: "row";
  align?: HomeAlign;
  gap?: number;
  children?: HomeNode[];
}

export type HomeNode =
  | HomeCardNode
  | HomeTextNode
  | HomeHintNode
  | HomeImageNode
  | HomeButtonNode
  | HomeButtonsNode
  | HomeGridNode
  | HomeListItemNode
  | HomeDividerNode
  | HomeSpacerNode
  | HomeRowNode;

export interface HomeLayout {
  /** 布局版本，预留 */
  version?: number;
  /** 页面标题（可选，仅作注释用） */
  title?: string;
  /** 自定义变量，可被按钮 set-var 修改，也可直接写死 */
  variables?: Record<string, string | number | boolean>;
  nodes: HomeNode[];
}

/** 变量表统一按字符串保存，渲染时再按需转换 */
export type HomeVars = Record<string, string>;
