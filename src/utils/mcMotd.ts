/**
 * Minecraft 服务器描述（MOTD）解析。
 *
 * MC 客户端用「节号 § + 代码字符」的旧式格式化码给 MOTD 上色/加粗等，
 * server.properties 里也常以 `&` 代替 §（服务端启动时会转成 §）。这里同时
 * 接受 § (U+00A7) 与 `&` 两种前缀，把原始字符串拆成带样式的文本段，供
 * `McMotd.vue` 渲染成和游戏内服务器列表一致的样子。
 */

/** 一段已解析的 MOTD 文本：携带 MC 格式化码对应的内联样式。 */
export interface MotdSpan {
  text: string;
  /** 十六进制颜色；未设颜色时为 undefined（继承默认文本色）。 */
  color?: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strikethrough?: boolean;
  obfuscated?: boolean;
}

/** MC 旧式颜色码 → 十六进制（与 Java 版配色一致）。 */
const MC_COLORS: Record<string, string> = {
  "0": "#000000",
  "1": "#0000AA",
  "2": "#00AA00",
  "3": "#00AAAA",
  "4": "#AA0000",
  "5": "#AA00AA",
  "6": "#FFAA00",
  "7": "#AAAAAA",
  "8": "#555555",
  "9": "#5555FF",
  a: "#55FF55",
  b: "#55FFFF",
  c: "#FF5555",
  d: "#FF55FF",
  e: "#FFFF55",
  f: "#FFFFFF",
};

/** 匹配 § 或 & 紧跟一个合法格式化码字符。 */
const CODE_RE = /[§&]([0-9a-fk-orA-FK-OR])/g;

/**
 * 把一行 MOTD 文本拆成带样式的段。颜色码会同时重置粗体/斜体等格式（与 MC 一致），
 * 格式码只改自己那一项，`r` 全部重置。
 */
export function parseMcMotdLine(line: string): MotdSpan[] {
  const spans: MotdSpan[] = [];
  let color: string | undefined;
  let bold = false;
  let italic = false;
  let underline = false;
  let strikethrough = false;
  let obfuscated = false;
  let buf = "";

  const flush = () => {
    if (buf) {
      spans.push({ text: buf, color, bold, italic, underline, strikethrough, obfuscated });
      buf = "";
    }
  };

  CODE_RE.lastIndex = 0;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = CODE_RE.exec(line)) !== null) {
    if (m.index > last) buf += line.slice(last, m.index);
    flush();
    const code = m[1].toLowerCase();
    if (code in MC_COLORS) {
      // 颜色码会重置所有格式（MC 行为）
      color = MC_COLORS[code];
      bold = italic = underline = strikethrough = obfuscated = false;
    } else if (code === "k") obfuscated = true;
    else if (code === "l") bold = true;
    else if (code === "m") strikethrough = true;
    else if (code === "n") underline = true;
    else if (code === "o") italic = true;
    else if (code === "r") {
      color = undefined;
      bold = italic = underline = strikethrough = obfuscated = false;
    }
    last = CODE_RE.lastIndex;
  }
  if (last < line.length) buf += line.slice(last);
  flush();
  return spans;
}

/**
 * 解析完整 MOTD（可含 `\n` 多行），返回每行的段数组。空行保留为空数组，
 * 由渲染层决定是否占位。
 */
export function parseMcMotd(input: string): MotdSpan[][] {
  if (!input) return [[]];
  return input.split("\n").map((line) => parseMcMotdLine(line));
}

/** 是否含 `§k` 魔咒（混淆）段——渲染层据此决定是否开随机字符动画。 */
export function motdHasObfuscated(lines: MotdSpan[][]): boolean {
  return lines.some((line) => line.some((s) => s.obfuscated));
}
