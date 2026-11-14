import { computed, ref } from "vue";
import { api } from "../api";

export const titlebarThemeToggle = ref(true);

/** 本机布尔偏好：存 "1" 为开，其余一律当关（读不到 localStorage 时也不炸）。 */
function readFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "1";
  } catch {
    return false;
  }
}

function writeFlag(key: string, value: boolean) {
  try {
    if (value) localStorage.setItem(key, "1");
    else localStorage.removeItem(key);
  } catch {
    /* 存储不可用时只在本次会话生效 */
  }
}

// ---- 开发者模式 ----
/**
 * 开发者模式是本机偏好：连点「关于」页品牌 7 次触发一次后持久保留，
 * 关掉设置页、切换路由甚至重启应用都不用再重新触发，直到主动「退出开发者模式」。
 */
const DEV_MODE_KEY = "qookix.settings.devMode";

export const devMode = ref(readFlag(DEV_MODE_KEY));

export function setDevMode(value: boolean) {
  devMode.value = value;
  writeFlag(DEV_MODE_KEY, value);
}

// ---- 内网穿透（frp）----
/**
 * 内网穿透（联机页的「内网穿透」页签、设置页账号里的管理入口）默认隐藏。
 * 只有开发者模式下、在开发者页打开这个开关才显示；开关状态本身持久保留，
 * 退出开发者模式后功能会跟着藏起来，下次进开发者模式还是上次的选择。
 */
const FRP_KEY = "qookix.dev.showFrp";

export const frpEnabled = ref(readFlag(FRP_KEY));

export function setFrpEnabled(value: boolean) {
  frpEnabled.value = value;
  writeFlag(FRP_KEY, value);
}

/** 内网穿透相关 UI 是否可见：开发者模式 + 开关同时成立。 */
export const showFrpFeatures = computed(() => devMode.value && frpEnabled.value);

/**
 * 是否允许按 F12（及 Ctrl/Cmd+Shift+I / J / C）打开浏览器开发者工具。
 * 本机偏好，默认关闭；只有开发者页的开关能放行。
 */
const DEVTOOLS_KEY = "qookix.dev.allowDevTools";

export const allowDevTools = ref(readFlag(DEVTOOLS_KEY));

export function setAllowDevTools(value: boolean) {
  allowDevTools.value = value;
  writeFlag(DEVTOOLS_KEY, value);
}

function isDevToolsHotkey(e: KeyboardEvent): boolean {
  if (e.key === "F12") return true;
  return (
    (e.ctrlKey || e.metaKey) &&
    e.shiftKey &&
    ["I", "J", "C"].includes(e.key.toUpperCase())
  );
}

let hotkeyInstalled = false;

/**
 * 全局 DevTools 快捷键拦截。
 *
 * `tauri.conf.json` 的 `devtools: false` 已在 WebView 层关掉 F12 / 右键「检查」
 * （WebView2 的 SetAreDevToolsEnabled(false)），这里再做一层兜底：始终吃掉这些
 * 键的默认行为，只有开关放行时才通过后端命令程序化打开 DevTools。
 * 两层一起保证「默认不允许」，开关打开后也不需要重新建窗口。
 */
export function installDevToolsHotkey() {
  if (hotkeyInstalled) return;
  hotkeyInstalled = true;
  document.addEventListener(
    "keydown",
    (e) => {
      if (!isDevToolsHotkey(e)) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (allowDevTools.value) void api.openDevTools().catch(() => {});
    },
    true,
  );
}
