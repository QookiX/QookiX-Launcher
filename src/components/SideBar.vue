<script setup lang="ts">
import { useRoute } from "vue-router";
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import AccountChip from "./AccountChip.vue";
import AppIcon from "./AppIcon.vue";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import { useOnboarding } from "../composables/useOnboarding";
import { setSidebarRevealed } from "../composables/useSidebarUI";
import { useTasksStore } from "../stores/tasks";
import { useInstancesStore } from "../stores/instances";
import { useSettingsStore } from "../stores/settings";
import { usePinsStore } from "../stores/pins";
import { useMessage } from "naive-ui";
import { useI18n } from "vue-i18n";
import {
  IconClock,
  IconCompass,
  IconGrid,
  IconHome,
  IconClose,
  IconNewspaper,
  IconSkin,
  IconStop,
  IconTool,
  IconUsers,
} from "./icons";

const route = useRoute();
const tasks = useTasksStore();
const instances = useInstancesStore();
const settingsStore = useSettingsStore();
const message = useMessage();
const { t } = useI18n();

// —— 悬停展开的悬浮岛侧边栏 ——
// 默认常驻显示为折叠窄栏（60px，仅图标），鼠标悬停 120ms 后展开为
// 完整面板（216px，带文字标签），移开 200ms 后收回窄栏。
const peek = ref(false);
// 新手引导期间强制保持展开，避免引导中途侧边栏收回
const tourActive = useOnboarding().tourActive;
const tourPinned = ref(false);
watch(tourActive, (active) => (tourPinned.value = active));

const revealed = computed(() => peek.value || tourPinned.value);
// 同步到共享 UI 状态：App.vue 读取它决定内容区是否给折叠窄栏让位
watch(revealed, (v) => setSidebarRevealed(v), { immediate: true });

let showTimer = 0;
let hideTimer = 0;
/** 悬停 120ms 后展开（防误触：鼠标穿过往内容时不会展开）。 */
function armShow() {
  clearTimeout(hideTimer);
  if (peek.value || tourPinned.value) return;
  showTimer = window.setTimeout(() => (peek.value = true), 120);
}
/** 移出 200ms 后收回（留容错时间，避免在面板内移动时收起）。 */
function scheduleHide() {
  clearTimeout(showTimer);
  if (tourPinned.value) return;
  hideTimer = window.setTimeout(() => (peek.value = false), 200);
}
onBeforeUnmount(() => {
  clearTimeout(showTimer);
  clearTimeout(hideTimer);
  tracking = false;
  cancelAnimationFrame(trackRaf);
});

const sidebarRef = ref<HTMLElement | null>(null);

// 新闻可隐藏：settings.show_news 为 false 时不显示该导航项（默认显示）
const nav = computed(() => {
  const hidden = new Set(settingsStore.settings?.hidden_nav_items ?? []);
  const list = [
    { name: "home", label: t("nav.home"), icon: IconHome, to: "/" },
    { name: "browse", label: t("nav.browse"), icon: IconCompass, to: "/browse" },
    { name: "instances", label: t("nav.instances"), icon: IconGrid, to: "/instances" },
    { name: "multiplayer", label: t("nav.multiplayer"), icon: IconUsers, to: "/multiplayer" },
    { name: "skins", label: t("nav.skins"), icon: IconSkin, to: "/skins" },
    { name: "toolbox", label: t("nav.toolbox"), icon: IconTool, to: "/toolbox" },
    { name: "timemachine", label: t("nav.timemachine"), icon: IconClock, to: "/timemachine" },
  ];
  if (settingsStore.settings?.show_news ?? true) {
    list.push({ name: "news", label: t("nav.news"), icon: IconNewspaper, to: "/news" });
  }
  // 设置入口已移至标题栏（暗黑模式按钮右侧），不再显示在侧边栏
  return list.filter((n) => n.name === "home" || !hidden.has(n.name));
});

// —— 侧边栏固定实例 ——
// 与「固定到首页」共用 pins store，但只取 target === "sidebar" 的实例项，
// 两者互不干扰；名称/图标跟随实例列表实时更新。
const pins = usePinsStore();
const pinnedInstances = computed(() => {
  const byId = new Map(instances.instances.map((i) => [i.id, i]));
  return pins.items
    .filter((p) => p.type === "instance" && p.target === "sidebar")
    .map((p) => {
      const inst = byId.get(p.instanceId);
      return {
        id: p.id,
        instanceId: p.instanceId,
        // 实例可能已被删除或改名/换图标，实时从实例列表取，缺失时回退 pin 快照
        name: inst?.name ?? p.instanceName ?? p.name,
        icon: inst?.icon ?? p.instanceIcon ?? null,
        exists: !!inst,
      };
    })
    // 已被删除的实例不再显示
    .filter((p) => p.exists);
});

function isActive(n: { to: string }) {
  if (n.to === "/") return route.path === "/";
  // 实例详情/创建页是「实例」的子页面，进入后保持「实例」高亮，
  // 否则滑动指示器会停留在上一个页面（没有任何 nav 项匹配 → 不更新）
  if (n.to === "/instances") {
    return (
      route.path.startsWith("/instances") ||
      route.path.startsWith("/instance/") ||
      route.path === "/create"
    );
  }
  return route.path.startsWith(n.to);
}

// Sliding active-highlight indicator
const navBox = ref<HTMLElement | null>(null);
const { indicatorStyle, refresh, snap, syncNow } = useSlidingIndicator(
  sidebarRef,
  // 排除 .pin-item（固定实例）与底部导航，索引顺序与 nav.value 一一对应
  () => Array.from(sidebarRef.value?.querySelectorAll<HTMLElement>(".nav-item:not(.pin-item)") ?? []),
  () => {
    // 用 nav.value（现在是 computed）。固定实例已被选择器排除，不会被计入，
    // 索引顺序保持稳定。下载入口已移至标题栏，这里不再处理底部导航。
    return nav.value.findIndex((n) => isActive(n));
  },
  { axis: "vertical" }
);
watch(
  () => route.path,
  () => nextTick(() => {
    // 宽度过渡进行中（tracking=true）时不做动画式 refresh：此刻测到的是
    // 过渡中间态几何，动画滑过去必然错位；逐帧跟踪循环全程接管，
    // 窗口结束时的 snap 会用最终几何精确校准。
    if (tracking) return;
    refresh();
  })
);
// —— 展开/收回期间逐帧跟踪指示器 ——
// 侧边栏 width/padding 过渡时导航项几何逐帧变化；指示器的 px 定位若停在
// 旧值、只在过渡结束后 snap 校准，动画全程尺寸不对、结束时瞬间跳变。
// 改为每帧 syncNow 重测重贴（零过渡直接落位）。
// 驱动方式：直接 watch revealed 状态，用时间窗口（300ms > 过渡时长 220ms）
// 覆盖整段过渡——不依赖 transitionstart/end 事件，事件在中途反向
// （cancel+重启）等场景下可能丢失，丢了跟踪就断、几何就停留在旧状态。
let tracking = false;
let trackRaf = 0;
watch(revealed, () => {
  cancelAnimationFrame(trackRaf);
  tracking = true;
  const until = performance.now() + 300;
  const step = () => {
    syncNow();
    if (performance.now() < until) {
      trackRaf = requestAnimationFrame(step);
    } else {
      tracking = false;
      snap(); // 最终精确校准
    }
  };
  trackRaf = requestAnimationFrame(step);
});
// 固定实例增删会改变侧边栏高度/布局，需重算指示器位置
watch(
  () => [pinnedInstances.value.length, nav.value.length],
  () => nextTick(() => refresh())
);

function unpin(id: string) {
  pins.remove(id);
}

async function stopAll() {
  try {
    await instances.stop();
    message.success(t("sidebar.stopAllDone"));
  } catch (e) {
    message.error(String(e));
  }
}
</script>

<template>
  <aside
    ref="sidebarRef"
    id="app-sidebar"
    class="sidebar"
    :class="{ peeking: revealed }"
    @mouseenter="armShow"
    @mouseleave="scheduleHide"
  >
    <div class="indicator" :style="indicatorStyle"></div>
    <nav ref="navBox" class="nav">
      <router-link
        v-for="n in nav"
        :key="n.name"
        :to="n.to"
        class="nav-item"
        :data-nav="n.name"
        :class="{ active: isActive(n) }"
        :title="!revealed ? n.label : undefined"
      >
        <component :is="n.icon" class="nav-icon" />
        <span v-if="revealed" class="nav-label">{{ n.label }}</span>
      </router-link>
    </nav>

    <!-- 固定实例：独立类名 .pin-item，避免被滑动指示器计入 .nav-item -->
    <div v-if="pinnedInstances.length" class="pin-section">
      <!-- 与上方导航之间只有这一条分割线（.pin-section 不再自带 border-top） -->
      <div class="pin-divider"></div>
      <div v-if="revealed" class="pin-section-title">{{ t("sidebar.pinned") }}</div>
      <div class="pin-list">
        <router-link
          v-for="p in pinnedInstances"
          :key="p.id"
          :to="`/instance/${p.instanceId}`"
          class="nav-item pin-item"
          :class="{ active: route.path.startsWith(`/instance/${p.instanceId}`) }"
          :title="p.name"
        >
          <div class="pin-icon-wrap">
            <AppIcon :name="p.icon" />
          </div>
          <span v-if="revealed" class="nav-label pin-label">{{ p.name }}</span>
          <button
            v-if="revealed"
            class="pin-unpin"
            :title="t('sidebar.unpin')"
            @click.prevent.stop="unpin(p.id)"
          >
            <IconClose />
          </button>
        </router-link>
      </div>
    </div>

    <div class="side-foot">
      <button v-if="tasks.gameRunning" class="stop-all-btn" @click="stopAll">
        <IconStop />
        <span v-if="revealed">{{ t("sidebar.stopAll") }}</span>
      </button>
      <AccountChip :collapsed="!revealed" />
    </div>
  </aside>
</template>

<style scoped>
/* —— 悬浮岛侧边栏：默认折叠窄栏（仅图标），悬停展开 ——
   常驻显示为 60px 宽的图标栏（悬浮岛：四周留空 + 圆角 + 环绕投影，
   不挤压内容布局），鼠标悬停 120ms 后平滑展开为 216px 完整面板，
   移开 200ms 后收回窄栏。展开动画用 width（触发 layout，
   不创建合成层，backdrop-filter 磨砂全程采样正常）。 */
.sidebar {
  position: fixed;
  top: 50px; /* 标题栏 40px + 10px 悬浮间距 */
  bottom: 10px;
  left: 10px;
  width: 60px;
  z-index: 45;
  display: flex;
  flex-direction: column;
  /* 折叠态四周留白统一 6px：60 - 6×2 = 48px 内容宽，与 nav-item
     高度一致，图标在正方形格子里完全居中，上下左右边距严格相等 */
  padding: 6px;
  border: 1px solid var(--border);
  border-radius: 18px;
  background: color-mix(in srgb, var(--bg-1) 84%, transparent);
  backdrop-filter: blur(var(--glass-blur, 8px));
  -webkit-backdrop-filter: blur(var(--glass-blur, 8px));
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.18);
  transition: width 0.22s cubic-bezier(0.22, 1, 0.36, 1), padding 0.22s cubic-bezier(0.22, 1, 0.36, 1);
  overflow: hidden;
}
.sidebar.peeking {
  width: 216px;
  padding: 14px 10px 12px;
}
/* 背景图片模式下与卡片一致：用半透明面板色 + 模糊，透出背景图 */
:global(.has-bg) .sidebar {
  background: var(--panel);
}
.nav {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
/* —— 固定实例区域 —— */
.pin-section {
  margin-top: 14px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  overflow-y: auto;
  max-height: 40vh;
  /* 隐藏滚动条但保留滚轮滚动 */
  scrollbar-width: none;
  -ms-overflow-style: none;
}
.pin-section::-webkit-scrollbar {
  display: none;
}
.pin-section-title {
  padding: 0 14px 2px;
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.05em;
  color: var(--text-3);
}
.pin-divider {
  height: 1px;
  margin: 0 10px 8px;
  background: var(--border);
  flex-shrink: 0;
}
.pin-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
/* 注意：这里必须用 .pin-list 提高优先级，否则会被下方同权重的
   .nav-item { padding: 10px 14px } 覆盖（CSS 后写胜出） */
.pin-list .pin-item {
  padding: 4px 6px 4px 4px;
  gap: 10px;
}
/* 折叠态 pin 条目：纯图标 48×48（与折叠导航项等宽） */
.sidebar:not(.peeking) .pin-list .pin-item {
  padding: 0;
  gap: 0;
}
.pin-icon-wrap {
  width: 40px;
  height: 40px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  overflow: hidden;
}
/* 折叠态图标放大铺满 48×48；背景高亮被图标挡住，
   改用内描边表示选中、亮度表示悬停 */
.sidebar:not(.peeking) .pin-icon-wrap {
  width: 48px;
  height: 48px;
}
.sidebar:not(.peeking) .pin-list .pin-item.active .pin-icon-wrap {
  box-shadow: inset 0 0 0 2px var(--accent);
}
.sidebar:not(.peeking) .pin-list .pin-item:hover .pin-icon-wrap {
  filter: brightness(1.12);
}
.pin-icon-wrap :deep(.app-icon) {
  width: 100%;
  height: 100%;
}
/* 头像式展示，用 cover 保证任何比例的图片都能铺满方框（contain 会留白边） */
.pin-icon-wrap :deep(img),
.pin-icon-wrap :deep(svg) {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.pin-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.pin-unpin {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 5px;
  background: transparent;
  color: var(--text-3);
  opacity: 0;
  cursor: pointer;
  transition: opacity 0.12s, color 0.12s, background 0.12s;
}
.pin-item:hover .pin-unpin {
  opacity: 1;
}
.pin-unpin:hover {
  color: var(--accent);
  background: var(--accent-soft);
}
.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 0;
  justify-content: center;
  border-radius: 10px;
  color: var(--text-2);
  font-size: 14px;
  font-weight: 500;
  text-decoration: none;
  transition: color 0.14s, background 0.14s, border-color 0.14s, transform 0.1s ease;
  border: 1px solid transparent;
  white-space: nowrap;
  overflow: hidden;
  height: 48px;
  box-sizing: border-box;
}
.nav-item:active {
  transform: scale(0.96);
}
/* 展开态恢复常规内边距；折叠态保持图标居中（基础样式） */
.sidebar.peeking .nav-item {
  padding: 10px 14px;
  justify-content: flex-start;
}
.nav-item:hover {
  background: var(--panel-hover);
  color: var(--text-1);
}
.nav-item.active {
  color: var(--accent);
}
.indicator {
  position: absolute;
  left: 0;
  width: 100%;
  z-index: 1;
  border-radius: 10px;
  background: var(--accent-soft);
  border: 1px solid var(--accent-03);
  transition:
    top 0.28s cubic-bezier(0.22, 1, 0.36, 1),
    height 0.28s cubic-bezier(0.22, 1, 0.36, 1),
    opacity 0.18s;
  pointer-events: none;
}
/* 折叠态（图标栏）隐藏滑动指示器：激活项已由图标主题色标示，高亮块在
   窄栏里冗余；更关键的是彻底杜绝「收回后指示器残留展开态几何、溢出
   窄栏」的问题——不管过渡事件是否如期触发，折叠态下它一律不可见。
   !important 用于压过组件内联的 opacity:1（syncNow 每帧写入）。
   展开/收回瞬间 computed opacity 变化，走内联 transition 平滑淡入淡出。 */
.sidebar:not(.peeking) .indicator {
  opacity: 0 !important;
}
.nav-icon,
.nav-label {
  position: relative;
  z-index: 2;
}
.nav-icon {
  font-size: 18px;
}
.nav-label {
  animation: fade-in 0.2s ease;
}
@keyframes fade-in {
  from { opacity: 0; }
  to { opacity: 1; }
}
.side-foot {
  margin-top: auto;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.stop-all-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  width: 100%;
  height: 40px;
  box-sizing: border-box;
  padding: 0;
  border: 1px solid var(--danger-40);
  border-radius: 10px;
  background: var(--danger-12);
  color: var(--danger-text);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.14s;
}
.sidebar.peeking .stop-all-btn {
  justify-content: flex-start;
  padding: 0 14px;
}
.stop-all-btn:hover {
  background: var(--danger-26);
}
</style>
