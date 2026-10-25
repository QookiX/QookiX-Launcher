<script setup lang="ts">
import { onMounted, onBeforeUnmount, computed, watch, ref, provide } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { darkTheme, lightTheme, NConfigProvider, NDialogProvider, NLoadingBarProvider, NMessageProvider, NNotificationProvider } from "naive-ui";
import { api } from "./api";
import TitleBar from "./components/TitleBar.vue";
import SideBar from "./components/SideBar.vue";
import DownloadsDrawer from "./components/DownloadsDrawer.vue";
import AccountDrawer from "./components/AccountDrawer.vue";
import LoadingBarBridge from "./components/LoadingBarBridge.vue";
import LaunchProgress from "./components/LaunchProgress.vue";
import CrashDialog from "./components/CrashDialog.vue";
import CloseConfirmDialog from "./components/CloseConfirmDialog.vue";
import UpdaterCheck from "./components/UpdaterCheck.vue";
import ForceUpdateOverlay from "./components/ForceUpdateOverlay.vue";
import SplashScreen from "./components/SplashScreen.vue";
import ModpackDropOverlay from "./components/ModpackDropOverlay.vue";
import { transitionName } from "./composables/usePageTransition";
import { useSettingsStore } from "./stores/settings";
import { initDeepLink } from "./composables/deepLink";
import { MessageBridge, notifyError } from "./composables/notify";
import { listen } from "@tauri-apps/api/event";
import { useOnboarding } from "./composables/useOnboarding";
import i18n from "./i18n";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import type { AppLocale } from "./i18n";

import { useAccountsStore } from "./stores/accounts";
import { useInstancesStore } from "./stores/instances";
import { useTasksStore } from "./stores/tasks";
import { usePinsStore } from "./stores/pins";
import {
  buildDarkOverrides,
  buildLightOverrides,
  DEFAULT_ACCENT,
  darken,
  lighten,
  rgba,
  accentContrast,
  ACCENT_ALPHAS,
} from "./theme";

const settings = useSettingsStore();
const accounts = useAccountsStore();
const instances = useInstancesStore();
const tasks = useTasksStore();
const pins = usePinsStore();
const onboarding = useOnboarding();
const { t } = useI18n();
const route = useRoute();

function applyLocale(lang: string | undefined | null) {
  if (lang === "en-US" || lang === "zh-CN") {
    i18n.global.locale.value = lang as AppLocale;
  }
}

watch(() => settings.settings?.language, (lang) => applyLocale(lang));

// 主题判定：缓存最后一次从 settings 读到的 theme 值。
// 当 settings 被重新拉取/patch 时，如果后端返回的对象恰好缺 theme 字段（IPC 序列化
// 异常、部分写入等），裸的 settings.settings?.theme !== "light" 会把 undefined 当成
// 非 "light" → isDark 翻成 true → 浅色模式瞬间切暗，且 .light 类被摘掉后 watch 不再
// 触发，状态永久卡死直到刷新。缓存上一次的 theme 可避免这种翻转。
const lastTheme = ref<string | undefined>(undefined);
const isDark = computed(() => {
  const t = settings.settings?.theme;
  if (t !== undefined) lastTheme.value = t;
  return (t ?? lastTheme.value) !== "light";
});
const activeTheme = computed(() => (isDark.value ? darkTheme : lightTheme));

const accentColor = computed(() => settings.settings?.theme_color || DEFAULT_ACCENT);
const themeOverrides = computed(() =>
  isDark.value ? buildDarkOverrides(accentColor.value) : buildLightOverrides(accentColor.value),
);

// 首次应用主题不做过渡（否则页面加载时会闪一下颜色）
let themeReady = false;
let themeTransitionTimer: ReturnType<typeof setTimeout> | null = null;

watch(isDark, () => {
  const root = document.documentElement;
  if (themeReady) {
    // 切换瞬间临时开启全局颜色过渡，结束后移除，避免影响性能与 hover 动画
    root.classList.add("theme-transition");
    if (themeTransitionTimer) clearTimeout(themeTransitionTimer);
    themeTransitionTimer = setTimeout(() => {
      root.classList.remove("theme-transition");
      themeTransitionTimer = null;
    }, 320);
  }
  root.classList.toggle("light", !isDark.value);
  themeReady = true;
}, { immediate: true });

onBeforeUnmount(() => {
  if (themeTransitionTimer) clearTimeout(themeTransitionTimer);
});

// 页面转场 out-in：旧页面离开后、新页面进入前（此时容器瞬时空白）归零滚动容器。
// 不能在 router.afterEach 里归零——那会在旧页面仍可见时就跳到顶，造成"先跳顶再转场"的突兀感。
function onPageBeforeEnter() {
  const el = document.getElementById("app-content");
  if (el) el.scrollTop = 0;
}

// 过渡结束后：WebView2 在 out-in 转场时，旧页面 transform 合成层可能残留，
// 新页面 .glass 的 backdrop-filter 采样到 stale 暗色合成层 → 内容发暗。
// 强制 reflow + 瞬间摘/加 backdrop-filter 让渲染器重新采样背景。
function onPageAfterEnter() {
  const root = document.documentElement;
  root.classList.add("bf-reset");
  void document.getElementById("app-content")?.offsetHeight; // 强制 reflow
  requestAnimationFrame(() => root.classList.remove("bf-reset"));
}

// 将主题色应用到 CSS 变量上（accent / accent-deep / accent-soft / 各级 alpha）
watch(accentColor, (hex) => {
  const root = document.documentElement;
  root.style.setProperty("--accent", hex);
  root.style.setProperty("--accent-deep", darken(hex));
  root.style.setProperty("--accent-hover", lighten(hex));
  root.style.setProperty("--accent-soft", rgba(hex, 0.14));
  root.style.setProperty("--accent-contrast", accentContrast(hex));
  for (const a of ACCENT_ALPHAS) {
    root.style.setProperty(`--accent-${String(Math.round(a * 100)).padStart(2, "0")}`, rgba(hex, a));
  }
}, { immediate: true });

const bgStyle = computed(() => {
  const s = settings.settings;
  if (!s?.background_image) return {} as Record<string, string>;
  return {
    "--bg-image": `url("${convertFileSrc(s.background_image)}")`,
    "--bg-blur": `${s.background_blur}px`,
    "--bg-dim": String(s.background_dim / 100),
    "--bg-dim-light": String((s.background_dim / 100) * 0.12),
  } as Record<string, string>;
});

// 无背景图时，装饰纹理（点阵 + 光带）只在首页出现；其他页面只留底色渐变。
const showTexture = computed(() => route.path === "/" && !settings.settings?.background_image);

watch(() => settings.settings?.glass_blur, (v) => {
  if (v != null) document.documentElement.style.setProperty("--glass-blur", `${v}px`);
}, { immediate: true });

watch(() => settings.settings?.background_image, (p) => {
  document.documentElement.classList.toggle("has-bg", !!p);
}, { immediate: true });

// 供 TitleBar 的“新建分组”按钮触发实例页的分组对话框
const groupDialogRequest = ref(0);
provide("groupDialogRequest", groupDialogRequest);

// —— 启动加载流程 ——
// 在 splash 期间预加载核心数据，让首屏直接有数据可渲染；
// 各 store 的 load() 已是「已有数据则后台静默刷新」模式，这里首次加载会真正拉取。
const bootProgress = ref(0);
const bootStatus = ref(t("boot.starting"));
const booted = ref(false);

async function boot() {
  bootStatus.value = t("boot.loadingSettings");
  try {
    await settings.load();
  } catch {
    /* 设置加载失败不阻塞启动 */
  }
  applyLocale(settings.settings?.language);
  bootProgress.value = 24;

  bootStatus.value = t("boot.readingData");
  await Promise.all([
    instances.load().catch(() => {}),
    accounts.load().catch(() => {}),
  ]);
  bootProgress.value = 72;

  bootStatus.value = t("boot.initTasks");
  tasks.init();
  await pins.init().catch(() => {});
  void initDeepLink();
  bootProgress.value = 92;

  bootStatus.value = t("boot.almostReady");
  bootProgress.value = 100;
  // 让 100% 短暂展示后再淡出，避免进度条一闪而过
  setTimeout(() => {
    booted.value = true;
    // 首次启动且未完成新手向导时，待 splash 淡出后自动弹出
    if (settings.settings && !settings.settings.onboarding_completed) {
      setTimeout(() => onboarding.open(), 700);
    }
  }, 340);
}

// 全局监听：分享包导入后游戏本体自动安装失败（后端无法直接弹 toast）
let unlistenShareErr: (() => void) | null = null;
onMounted(async () => {
  void boot();
  // 对照探针：验证「前端 → log_debug → 落盘」这条日志通道本身可用
  api.logDebug("[fe] App 启动，日志通道自检");
  try {
    unlistenShareErr = await listen<{ instanceId: string; error: string }>(
      "share-import://game-install-failed",
      (ev) => {
        notifyError(`游戏本体自动安装失败：${ev.payload.error}`);
      }
    );
  } catch {
    /* 监听不可用不影响主流程 */
  }
});
onBeforeUnmount(() => {
  unlistenShareErr?.();
  unlistenShareErr = null;
});
</script>

<template>
  <n-config-provider :theme="activeTheme" :theme-overrides="themeOverrides">
    <n-loading-bar-provider>
      <LoadingBarBridge>
        <n-dialog-provider>
          <n-message-provider>
            <MessageBridge />
            <n-notification-provider>
                <!-- data-tauri-drag-region：让「背景」也能拖动窗口（标题栏之外）。
                     Tauri 2 的语义（src/window/scripts/drag.js）：
                       - 无值 / "true" → 只有**直接**点在带属性的这个元素上才拖（子元素不受影响）
                       - "deep"        → 整个子树都可拖（卡片、按钮等 div 点击会被吞掉，故不用）
                     .app-bg / .body 带裸属性：它们自身的留白落点可以拽窗口。
                     .content（滚动容器）**不带** drag-region：否则滚动条落在拖拽区上，
                     按下滚动条会被 Tauri 拦截成拖窗口而无法滚屏。窗口拖动靠标题栏 + 侧栏留白。 -->
                <div class="app app-bg" data-tauri-drag-region :class="{ light: !isDark, 'show-texture': showTexture }" :style="bgStyle">
                <TitleBar />
                <div class="body" data-tauri-drag-region>
                  <SideBar />
                  <main id="app-content" class="content">
                    <router-view v-slot="{ Component, route }">
                      <!-- duration 必须显式给：out-in 靠 transitionend 判断"旧页面退场完毕"，
                           一旦某次没收到结束事件（元素被替换 / 过渡被打断），这个 out-in
                           会**永久卡住**——之后点任何页面都"没反应"、切页空白。 -->
                      <Transition
                        :name="transitionName"
                        mode="out-in"
                        :duration="{ enter: 260, leave: 200 }"
                        @before-enter="onPageBeforeEnter"
                        @after-enter="onPageAfterEnter"
                      >
                        <KeepAlive :include="['BrowseView']">
                          <component :is="Component" :key="route.path" />
                        </KeepAlive>
                      </Transition>
                    </router-view>
                  </main>
                </div>
                </div>
                <LaunchProgress />
                <CrashDialog />
                <CloseConfirmDialog />
                <DownloadsDrawer />
                <AccountDrawer />
                <UpdaterCheck />
                <ForceUpdateOverlay />
                <ModpackDropOverlay />
                <SplashScreen :progress="bootProgress" :status="bootStatus" :done="booted" />
              </n-notification-provider>
          </n-message-provider>
        </n-dialog-provider>
      </LoadingBarBridge>
    </n-loading-bar-provider>
  </n-config-provider>
</template>

<style scoped>
.app {
  height: 100vh;
  display: flex;
  flex-direction: column;
}
.body {
  flex: 1;
  display: flex;
  min-height: 0;
}
.content {
  flex: 1;
  overflow-y: auto;
  /* 底部留出常驻 Dock 的空间（60px 高 + 10px 悬浮间距），内容不被遮挡 */
  padding: 22px 26px 80px;
}
</style>

