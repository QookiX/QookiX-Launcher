<script setup lang="ts">
import { onMounted, onBeforeUnmount, computed, watch, ref, provide } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { darkTheme, lightTheme, NConfigProvider, NDialogProvider, NLoadingBarProvider, NMessageProvider, NNotificationProvider } from "naive-ui";
import { api } from "./api";
import { resolveScene } from "./components/background/scenes";
import { resolveQuality } from "./components/background/quality";
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
import { prefersReducedMotion } from "./composables/useMotionPreference";
import { useSettingsStore } from "./stores/settings";
import { initDeepLink } from "./composables/deepLink";
import { MessageBridge, notifyError } from "./composables/notify";
import { useTauriListenerScope } from "./composables/tauriListeners";
import { useOnboarding } from "./composables/useOnboarding";
import i18n from "./i18n";
import { useI18n } from "vue-i18n";
import { useRoute } from "vue-router";
import type { AppLocale } from "./i18n";

import { useAccountsStore } from "./stores/accounts";
import { useInstancesStore } from "./stores/instances";
import { useTasksStore } from "./stores/tasks";
import { usePinsStore } from "./stores/pins";
import { useQookixStore } from "./stores/qookix";
import {
  buildDarkOverrides,
  buildLightOverrides,
  DEFAULT_ACCENT,
  buildActionPalette,
  rgba,
  ACCENT_ALPHAS,
} from "./theme";

const subscriptions = useTauriListenerScope();
const { listen } = subscriptions;

const settings = useSettingsStore();
const accounts = useAccountsStore();
const instances = useInstancesStore();
const tasks = useTasksStore();
const pins = usePinsStore();
const qookix = useQookixStore();
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

watch([isDark, prefersReducedMotion], ([dark, reduced], previous) => {
  const root = document.documentElement;
  if (themeTransitionTimer) clearTimeout(themeTransitionTimer);
  if (themeReady && !reduced && previous[0] !== dark) {
    // 切换瞬间临时开启全局颜色过渡，结束后移除，避免影响性能与 hover 动画
    root.classList.add("theme-transition");
    themeTransitionTimer = setTimeout(() => {
      root.classList.remove("theme-transition");
      themeTransitionTimer = null;
    }, 320);
  } else {
    root.classList.remove("theme-transition");
  }
  root.classList.toggle("light", !dark);
  themeReady = true;
}, { immediate: true });

onBeforeUnmount(() => {
  if (themeTransitionTimer) clearTimeout(themeTransitionTimer);
});

let visiblePath = route.path;
let browseScrollTop = 0;
let browseReturnFocus: HTMLElement | undefined;

function onPageBeforeLeave() {
  if (visiblePath !== "/browse") return;
  const container = document.getElementById("app-content");
  if (!container) return;
  browseScrollTop = container.scrollTop;
  const focused = document.activeElement;
  browseReturnFocus = focused instanceof HTMLElement && container.contains(focused) ? focused : undefined;
}

function onPageBeforeEnter() {
  visiblePath = route.path;
  const container = document.getElementById("app-content");
  if (container) container.scrollTop = 0;
}

// 过渡结束后：WebView2 在 out-in 转场时，旧页面 transform 合成层可能残留，
// 新页面 .glass 的 backdrop-filter 采样到 stale 暗色合成层 → 内容发暗。
// 强制 reflow + 瞬间摘/加 backdrop-filter 让渲染器重新采样背景。
function onPageAfterEnter() {
  if (visiblePath === "/browse") {
    const container = document.getElementById("app-content");
    if (container) container.scrollTop = browseScrollTop;
    if (browseReturnFocus?.isConnected) browseReturnFocus.focus({ preventScroll: true });
  }
  const root = document.documentElement;
  root.classList.add("bf-reset");
  void document.getElementById("app-content")?.offsetHeight; // 强制 reflow
  requestAnimationFrame(() => root.classList.remove("bf-reset"));
}

// 将主题色应用到 CSS 变量上（accent / accent-deep / accent-soft / 各级 alpha）
watch(accentColor, (hex) => {
  const root = document.documentElement;
  const { fill, ink, deep, hover } = buildActionPalette(hex);
  root.style.setProperty("--accent", fill);
  root.style.setProperty("--accent-deep", deep);
  root.style.setProperty("--accent-hover", hover);
  root.style.setProperty("--accent-soft", rgba(hex, 0.14));
  root.style.setProperty("--accent-contrast", ink);
  for (const a of ACCENT_ALPHAS) {
    root.style.setProperty(`--accent-${String(Math.round(a * 100)).padStart(2, "0")}`, rgba(hex, a));
  }
}, { immediate: true });

/**
 * 背景类型：
 * - `gradient`：纯色渐变（默认）
 * - `image`：用户选的自定义图片
 * - `scene`：实时渲染的动态场景，具体是哪一个由 `background_scene` 决定
 *
 * 后端 `background_mode` 为空串表示「自动」——老版本配置没有这个字段，
 * 有图就用图、没图用渐变，升级后外观不会突变。
 */
const bgMode = computed<"gradient" | "image" | "scene">(() => {
  const s = settings.settings;
  if (!s) return "gradient";
  const m = s.background_mode;
  if (m === "gradient" || m === "image" || m === "scene") return m;
  return s.background_image ? "image" : "gradient";
});

const showImageBg = computed(() => bgMode.value === "image" && !!settings.settings?.background_image);
const showSceneBg = computed(() => bgMode.value === "scene");

// 场景组件一律异步加载：开着渐变或图片背景的用户不会为 three.js 付出任何首屏成本
const bgScene = computed(() => resolveScene(settings.settings?.background_scene));
// 画质档位同时决定像素比、帧率与着色器细节，改档要把组件重建一遍才生效
const bgQuality = computed(() => resolveQuality(settings.settings?.background_quality));

const bgStyle = computed(() => {
  const s = settings.settings;
  if (!s) return {} as Record<string, string>;
  const dim = s.background_dim ?? 0;
  const out: Record<string, string> = {};
  if (showSceneBg.value) {
    // 动态场景复用「遮罩」滑块控制可读性；浅色模式用更轻的白遮罩，
    // 否则白天场景（蓝天白云）会被压成一片惨白
    out["--bg-dim"] = String(dim / 100);
    out["--bg-dim-light"] = String((dim / 100) * 0.22);
    return out;
  }
  if (!showImageBg.value) return out;
  out["--bg-image"] = `url("${convertFileSrc(s.background_image!)}")`;
  out["--bg-blur"] = `${s.background_blur}px`;
  out["--bg-dim"] = String(dim / 100);
  out["--bg-dim-light"] = String((dim / 100) * 0.12);
  return out;
});

// 非渐变背景（图片或 3D 场景）都要：隐藏首页纹理、玻璃面板加实底、标题加阴影
const hasRichBg = computed(() => showImageBg.value || showSceneBg.value);

// 无背景图 / 无场景时，装饰纹理（点阵 + 光带）只在首页出现；其他页面只留底色渐变。
const showTexture = computed(() => route.path === "/" && !hasRichBg.value);

watch(() => settings.settings?.glass_blur, (v) => {
  if (v != null) document.documentElement.style.setProperty("--glass-blur", `${v}px`);
}, { immediate: true });

watch(hasRichBg, (v) => document.documentElement.classList.toggle("has-bg", v), { immediate: true });
watch(showSceneBg, (v) => document.documentElement.classList.toggle("has-scene", v), { immediate: true });

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
  await pins.init().catch((error) => notifyError(String(error)));
  void initDeepLink(subscriptions);
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

onBeforeUnmount(() => tasks.disposeListeners());
onMounted(async () => {
  void tasks.init().catch((error) => console.error("Failed to subscribe to task events:", error));
  void boot();
  // QookiX 可选账号：读登录态 + 注册后台推送事件（未登录时是空操作）
  void qookix.init();
  // 对照探针：验证「前端 → log_debug → 落盘」这条日志通道本身可用
  api.logDebug("[fe] App 启动，日志通道自检");
  try {
    await listen<{ instanceId: string; error: string }>(
      "share-import://game-install-failed",
      (ev) => {
        notifyError(`游戏本体自动安装失败：${ev.payload.error}`);
      }
    );
  } catch {
    /* 监听不可用不影响主流程 */
  }
  try {
    await listen<{ instanceId: string; message: string }>(
      "tray://launch-failed",
      (ev) => {
        notifyError(`启动失败：${ev.payload.message}`);
      }
    );
  } catch {
    /* 监听不可用不影响主流程 */
  }
});
</script>

<template>
  <n-config-provider :theme="activeTheme" :theme-overrides="themeOverrides">
    <n-loading-bar-provider>
      <LoadingBarBridge>
        <n-dialog-provider>
          <!-- 悬停时暂停自动消失：长错误信息要能读完、点一下复制 -->
          <n-message-provider :keep-alive-on-hover="true">
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
                <!-- key 里带上画质档：pixelRatio / antialias 这类参数只在初始化时生效，
                     不重建组件切档就没反应 -->
                <component
                  :is="bgScene.component"
                  v-if="showSceneBg"
                  :key="`${bgScene.id}-${bgQuality.id}`"
                  :dark="isDark"
                  :accent="accentColor"
                  :quality="bgQuality"
                />
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
                        :duration="prefersReducedMotion ? 0 : { enter: 260, leave: 200 }"
                        @before-leave="onPageBeforeLeave"
                        @before-enter="onPageBeforeEnter"
                        @after-enter="onPageAfterEnter"
                      >
                        <!-- 缓存的页面：切回来不再重新挂载 + 重新拉一遍数据。
                             只放「纯 store 驱动 / 无定时器」的页面；MultiplayerView
                             有内网穿透的 1.5s 轮询，缓存在后台会一直空转，
                             要缓存得先补 onDeactivated 停表。 -->
                        <KeepAlive :include="['BrowseView', 'InstancesView', 'ToolboxView']">
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
  /* 外层永不滚动，所有页面滚动只发生在 .content 里，避免标题栏被顶出视口 */
  overflow: hidden;
}
.body {
  flex: 1;
  display: flex;
  min-height: 0;
}
.content {
  /* 内容区留白：抽成变量，页面里的吸顶条（如发现页 .browse-head）要按它
     反向偏移才能贴到滚到最顶，改这里一处即可，不用再去各页面同步数字。 */
  --content-pad-top: 22px;
  --content-pad-x: 26px;
  flex: 1;
  overflow-y: auto;
  /* 页面左右滑入的转场（slide-* 用 position:relative + left）会让页面横向溢出，
     不裁掉的话 WebView2 会给这个滚动容器加一条横向滚动条 */
  overflow-x: hidden;
  /* 底部留出常驻 Dock 的空间（60px 高 + 10px 悬浮间距），内容不被遮挡 */
  padding: var(--content-pad-top) var(--content-pad-x) 80px;
}
</style>

