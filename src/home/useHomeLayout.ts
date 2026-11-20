import { defineStore, storeToRefs } from "pinia";
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
/** 首页与设置共享布局文件的读取、重载与恢复。 */
const useHomeLayoutStore = defineStore("homeLayout", () => {
  const layout = ref<HomeLayout | null>(null);
  const path = ref("");
  const source = ref("");
  const error = ref("");
  const loading = ref(false);
  const enabled = ref(readEnabled());
  const showRecent = ref(readShowRecent());
  let loaded = false;
  let reading: Promise<boolean> | undefined;
  let resetting: Promise<boolean> | undefined;

  async function readLayout(): Promise<boolean> {
    error.value = "";
    try {
      const info = await api.homeLayoutRead();
      path.value = info.path;
      const text = info.exists ? info.content : DEFAULT_LAYOUT_SOURCE;
      source.value = text;
      layout.value = parseHomeLayout(text);
      // 模板仍可在本次会话渲染，写入失败会交给调用方展示。
      if (!info.exists) await api.homeLayoutSave(text);
      loaded = true;
      return true;
    } catch (e) {
      if (e instanceof HomeLayoutError) layout.value = null;
      error.value = e instanceof HomeLayoutError ? e.message : String(e);
      loaded = false;
      return false;
    }
  }

  function load(): Promise<boolean> {
    if (resetting) return resetting;
    if (reading) return reading;
    loading.value = true;
    reading = readLayout().finally(() => {
      reading = undefined;
      if (!resetting) loading.value = false;
    });
    return reading;
  }

  /** 只在还没成功加载过时读一次，供首页 / 设置页各自 onMounted 调用 */
  function ensureLoaded(): Promise<boolean> {
    return resetting ?? reading ?? (loaded ? Promise.resolve(true) : load());
  }

  function setEnabled(value: boolean) {
    localStorage.setItem(HOME_CUSTOM_KEY, value ? "1" : "0");
    enabled.value = value;
  }

  function setShowRecent(value: boolean) {
    localStorage.setItem(HOME_RECENT_KEY, value ? "1" : "0");
    showRecent.value = value;
  }

  /** 删除文件后重新生成默认模板 */
  function reset(): Promise<boolean> {
    if (resetting) return resetting;
    loading.value = true;
    resetting = (async () => {
      await reading;
      try {
        await api.homeLayoutReset();
      } catch (e) {
        error.value = String(e);
        return false;
      }
      try {
        clearUserVars();
      } catch (e) {
        await readLayout();
        error.value = String(e);
        return false;
      }
      return readLayout();
    })().finally(() => {
      resetting = undefined;
      loading.value = false;
    });
    return resetting;
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
    reset,
  };
});

export function useHomeLayout() {
  const store = useHomeLayoutStore();
  return {
    ...storeToRefs(store),
    ensureLoaded: store.ensureLoaded,
    load: store.load,
    reload: store.load,
    reset: store.reset,
    setEnabled: store.setEnabled,
    setShowRecent: store.setShowRecent,
  };
}
