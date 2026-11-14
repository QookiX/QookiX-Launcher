import { ref } from "vue";
import { api } from "../api";
import {
  clearUserVars,
  DEFAULT_LAYOUT_SOURCE,
  HOME_CUSTOM_KEY,
  HOME_RECENT_KEY,
  HomeLayoutError,
  parseHomeLayout,
} from "./layout";
import type { HomeLayout } from "./types";

/**
 * 自定义主页状态（模块级单例）。
 *
 * 首页负责渲染，设置页负责配置，两边必须看到同一份布局与同一个开关，
 * 所以状态放在模块作用域；`useHomeLayout()` 每次返回的是同一组 ref。
 *
 * 文件放在数据目录根的 `home.json`：首次读取不存在时会把 DEFAULT_LAYOUT_SOURCE
 * 写进去（带注释的模板），用户之后用任何编辑器改它即可。
 */
const layout = ref<HomeLayout | null>(null);
const path = ref("");
const source = ref("");
const error = ref("");
const loading = ref(false);
/** 是否启用自定义主页（本机偏好，不进设置文件）。默认关闭，只有显式开启过才是 "1"。 */
function readEnabled(): boolean {
  try {
    // 旧版本用 "0" 表示关闭、"1" 表示开启且默认开启；现在默认关闭，
    // 因此只认显式的 "1"，历史遗留的 "0" 与空值都按关闭处理。
    return localStorage.getItem(HOME_CUSTOM_KEY) === "1";
  } catch {
    return false;
  }
}
const enabled = ref(readEnabled());

/**
 * 首页「最近游玩」兜底内容是否显示（本机偏好，不进设置文件）。
 * 默认开启；只有显式存过 "0" 才关闭——有用户就是喜欢干净的空背景。
 */
function readShowRecent(): boolean {
  try {
    return localStorage.getItem(HOME_RECENT_KEY) !== "0";
  } catch {
    return true;
  }
}
const showRecent = ref(readShowRecent());
/** 是否已经成功读过一次；失败时保持 false，下次进设置页会再试 */
let loaded = false;

export function useHomeLayout() {
  async function load(): Promise<boolean> {
    loading.value = true;
    error.value = "";
    try {
      const info = await api.homeLayoutRead();
      path.value = info.path;
      let text = info.content;
      if (!info.exists) {
        text = DEFAULT_LAYOUT_SOURCE;
        // 写失败不阻塞渲染：本次仍按默认模板显示
        await api.homeLayoutSave(text).catch(() => {});
      }
      source.value = text;
      layout.value = parseHomeLayout(text);
      loaded = true;
      return true;
    } catch (e) {
      layout.value = null;
      error.value = e instanceof HomeLayoutError ? e.message : String(e);
      loaded = false;
      return false;
    } finally {
      loading.value = false;
    }
  }

  /** 只在还没成功加载过时读一次，供首页 / 设置页各自 onMounted 调用 */
  async function ensureLoaded(): Promise<boolean> {
    if (loaded || loading.value) return layout.value !== null;
    return load();
  }

  function setEnabled(value: boolean) {
    enabled.value = value;
    try {
      localStorage.setItem(HOME_CUSTOM_KEY, value ? "1" : "0");
    } catch {
      /* 存储不可用时开关只在本次会话生效 */
    }
  }

  function setShowRecent(value: boolean) {
    showRecent.value = value;
    try {
      localStorage.setItem(HOME_RECENT_KEY, value ? "1" : "0");
    } catch {
      /* 同上：只在本次会话生效 */
    }
  }

  /** 删除文件后重新生成默认模板 */
  async function reset(): Promise<boolean> {
    await api.homeLayoutReset().catch(() => {});
    clearUserVars();
    return load();
  }

  /** 用户手动改完文件后重载 */
  async function reload(): Promise<boolean> {
    return load();
  }

  return {
    layout,
    path,
    source,
    error,
    loading,
    enabled,
    setEnabled,
    showRecent,
    setShowRecent,
    ensureLoaded,
    load,
    reload,
    reset,
  };
}
