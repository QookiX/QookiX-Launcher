<script setup lang="ts">
import { useRoute } from "vue-router";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import AppIcon from "./AppIcon.vue";
import { useOnboarding } from "../composables/useOnboarding";
import { useInstancesStore } from "../stores/instances";
import { useSettingsStore } from "../stores/settings";
import { useTasksStore } from "../stores/tasks";
import { useAccountsStore } from "../stores/accounts";
import { usePinsStore } from "../stores/pins";
import { useI18n } from "vue-i18n";
import { useMessage } from "naive-ui";
import {
  IconChevronDown,
  IconClock,
  IconClose,
  IconCompass,
  IconGrid,
  IconHome,
  IconNewspaper,
  IconSkin,
  IconStop,
  IconTool,
  IconUsers,
} from "./icons";
import { loaderBadge } from "../utils/format";

const route = useRoute();
const instances = useInstancesStore();
const settingsStore = useSettingsStore();
const tasks = useTasksStore();
const accounts = useAccountsStore();
const pins = usePinsStore();
const message = useMessage();
const { t } = useI18n();

// 新手引导期间禁用单项 hover 展开（避免高亮框错位）；平时悬停单项滑出名称
const { tourActive } = useOnboarding();

// —— 启动区：当前实例 + 启动按钮 ——
// 常驻实例选择沿用首页的 localStorage 记忆（qookix.home.selected），
// 首页与 Dock 共享同一份选择，任一处切换另一处同步。
const STORAGE_KEY = "qookix.home.selected";
const selectedId = ref<string | null>(null);

const lastPlayed = computed(() =>
  [...instances.instances].sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0))[0] ?? null
);

function resolveDefault(): string | null {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved && instances.get(saved)) return saved;
  return (lastPlayed.value ?? instances.instances[0])?.id ?? null;
}

const selected = computed(() =>
  selectedId.value ? (instances.get(selectedId.value) ?? null) : null
);

watch(
  () => instances.instances,
  () => {
    if (!selectedId.value) selectedId.value = resolveDefault();
  },
  { immediate: true }
);

watch(selectedId, (id) => {
  if (id) localStorage.setItem(STORAGE_KEY, id);
});

const switching = ref(false);

function pickInstance(id: string) {
  selectedId.value = id;
  switching.value = false;
}

// 点击启动区以外任意位置时收起实例切换列表
function onDocClick(e: MouseEvent) {
  if (!switching.value) return;
  const zone = e.target as HTMLElement;
  if (!zone.closest(".launch-zone")) switching.value = false;
}
onMounted(() => document.addEventListener("click", onDocClick));
onBeforeUnmount(() => document.removeEventListener("click", onDocClick));

async function launchSelected() {
  const target = selected.value;
  if (!target) {
    message.info(t("home.message.noInstance"));
    return;
  }
  if (!accounts.accounts.length) {
    message.warning(t("home.message.addAccount"));
    accounts.showManager = true;
    return;
  }
  try {
    await instances.launch(target.id);
    localStorage.setItem("qookix:onboarding:launched", "1");
    message.success(t("home.message.launched", { name: target.name }));
  } catch (e) {
    message.error(String(e));
  }
}

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
  // 设置 / 下载入口已移至标题栏，不再显示在 Dock
  return list.filter((n) => n.name === "home" || !hidden.has(n.name));
});

// —— Dock 固定实例 ——
// 与「固定到首页」共用 pins store，但只取 target === "sidebar" 的实例项，
// 两者互不干扰；名称/图标跟随实例列表实时更新。
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
  // 实例详情/创建页是「实例」的子页面，进入后保持「实例」高亮
  if (n.to === "/instances") {
    return (
      route.path.startsWith("/instances") ||
      route.path.startsWith("/instance/") ||
      route.path === "/create"
    );
  }
  return route.path.startsWith(n.to);
}

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
  <aside id="app-sidebar" class="dock" :class="{ touring: tourActive }">
    <nav class="nav-list">
      <router-link
        v-for="n in nav"
        :key="n.name"
        :to="n.to"
        class="nav-item"
        :data-nav="n.name"
        :class="{ active: isActive(n) }"
      >
        <span class="nav-icon-cell">
          <component :is="n.icon" class="nav-icon" />
        </span>
        <span class="nav-tip">{{ n.label }}</span>
      </router-link>
    </nav>

    <template v-if="pinnedInstances.length">
      <div class="dock-divider"></div>
      <div class="dock-pins">
        <router-link
          v-for="p in pinnedInstances"
          :key="p.id"
          :to="`/instance/${p.instanceId}`"
          class="nav-item pin-item"
          :class="{ active: route.path.startsWith(`/instance/${p.instanceId}`) }"
        >
          <div class="pin-icon-wrap">
            <AppIcon :name="p.icon" />
          </div>
          <span class="pin-label">{{ p.name }}</span>
          <button class="pin-unpin" :title="t('sidebar.unpin')" @click.prevent.stop="unpin(p.id)">
            <IconClose />
          </button>
        </router-link>
      </div>
    </template>

    <button v-if="tasks.gameRunning" class="stop-all-btn" @click="stopAll">
      <span class="nav-icon-cell">
        <IconStop />
      </span>
      <span class="nav-tip">{{ t("sidebar.stopAll") }}</span>
    </button>
  </aside>

  <!-- —— 启动按钮（贴窗口右缘） ——
       一个整体按钮内部分割：左段窄条仅切换图标（点击弹实例列表），
       右段大图标 + 两行文字（上：启动游戏，下：实例名），点击直接启动。
       无卡片外壳，按钮自身即完整控件。 -->
  <div class="launch-zone">
    <button
      class="split-switch"
      :title="t('home.resident.switchInstance')"
      @click="switching = !switching"
    >
      <IconChevronDown class="split-switch-chev" :class="{ open: switching }" />
    </button>

    <button
      class="split-main"
      :disabled="!!instances.launchingId"
      :title="selected ? `${selected.name} · ${selected.mc_version}` : t('home.message.noInstance')"
      @click="launchSelected"
    >
      <span class="split-icon">
        <AppIcon v-if="selected" :name="selected.icon" />
        <IconGrid v-else />
      </span>
      <span class="split-text">
        <span class="split-title">{{ instances.launchingId ? t("home.resident.launching") : t("home.resident.launchGame") }}</span>
        <span v-if="selected" class="split-name text-ellipsis">{{ selected.name }}</span>
        <span v-else class="split-name split-none">{{ t("home.empty.noInstance") }}</span>
      </span>
    </button>

    <!-- 实例切换弹出列表 -->
    <Transition name="inst-pop">
      <div v-if="switching" class="inst-pop">
        <button
          v-for="inst in instances.instances"
          :key="inst.id"
          class="inst-row"
          :class="{ active: selectedId === inst.id }"
          @click="pickInstance(inst.id)"
        >
          <span class="inst-row-icon"><AppIcon :name="inst.icon" /></span>
          <span class="inst-row-name text-ellipsis">{{ inst.name }}</span>
          <span class="inst-row-meta">{{ loaderBadge(inst.loader) }} · {{ inst.mc_version }}</span>
        </button>
        <div v-if="!instances.instances.length" class="inst-empty">{{ t("home.empty.noInstance") }}</div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
/* —— 底部 Dock 导航栏：常驻图标条，悬停单项向右滑出名称 ——
   横向图标条（高 60px，悬浮岛：四周留空 + 圆角 + 环绕投影 + 磨砂），
   不占满窗口宽：宽度随内容自适应、贴左放置（右侧留给其他元素）。
   无整体展开态：悬停单个按钮时按钮自身横向扩宽，页面名称从图标右侧
   以弹性曲线滑出（max-width 过渡，图标槽位全程不动）。 */
.dock {
  position: fixed;
  bottom: 10px;
  left: 10px;
  /* 宽度不设 right：内容自适应，贴左悬浮 */
  height: 60px;
  max-width: calc(100vw - 20px);
  z-index: 45;
  display: flex;
  align-items: center;
  gap: 6px;
  /* 四周留白统一 6px：60 - 6×2 = 48px 内容高，与 nav-item 宽度一致，
     图标在正方形格子里完全居中，上下左右边距严格相等 */
  padding: 6px;
  border-radius: 18px;
  background: var(--panel);
  backdrop-filter: blur(var(--glass-blur, 8px));
  -webkit-backdrop-filter: blur(var(--glass-blur, 8px));
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.18);
}
.nav-list {
  display: flex;
  align-items: center;
  gap: 4px;
}
/* —— 固定实例区（Dock 右段） —— */
.dock-divider {
  width: 1px;
  align-self: stretch;
  margin: 4px;
  background: var(--border);
  flex-shrink: 0;
}
.dock-pins {
  display: flex;
  align-items: center;
  gap: 4px;
}
.pin-icon-wrap {
  width: 48px;
  height: 48px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  overflow: hidden;
}
/* 背景高亮被图标挡住，改用内描边表示选中、亮度表示悬停 */
.pin-item.active .pin-icon-wrap {
  box-shadow: inset 0 0 0 2px var(--accent);
}
.pin-item:hover .pin-icon-wrap {
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
.pin-unpin {
  /* 内联在 grid 第三列：默认 display:none 不占空间（避免固定实例右侧多出块），
     hover 时恢复 inline-flex 为 22×22 正方形，随列展开滑出。 */
  grid-column: 3;
  display: none;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
  opacity: 0;
  transform: scale(0.6);
  transition:
    transform 0.3s cubic-bezier(0.34, 1.56, 0.64, 1) 0.05s,
    opacity 0.2s ease 0.06s,
    color 0.14s,
    background 0.14s;
}
.pin-item:hover .pin-unpin {
  display: inline-flex;
  opacity: 1;
  transform: scale(1);
}
.pin-unpin:hover {
  color: var(--danger);
  background: var(--danger-12);
}
/* —— 悬停展开：图标右边滑出页面名称（CSS Grid 0fr→1fr 过渡） ——
   三列网格：[图标槽 48px | 文字列 0fr | 右留白 0px]
   hover 时 → [48px | 1fr | 12px]，grid-template-columns 原生插值：
   - 0fr→1fr 从 0 到内容实际宽度全程连续，无 max-width 方案的空跑
     死区（max-width 从上限收回到内容宽之前按钮不动 = 收回卡顿根源）
   - 图标槽恒 48px 第一列，图标坐标全程零移动
   - 右留白独立第三列同步过渡，收起后零残余宽度
   - 文字本体 translateX 侧滑：展开从图标后方滑入、收起向后滑出被裁
     （配合 opacity），是真正的侧滑消失而非原地淡出
   弹性曲线 cubic-bezier(0.34, 1.56, 0.64, 1)：Material 标准 spring，
   过冲 1.56，展开时轻微冲过头再回弹；收回用快速收尾曲线。 */
.nav-item {
  position: relative;
  display: inline-grid;
  grid-template-columns: 48px 0fr 0px;
  align-items: center;
  height: 48px;
  padding: 0;
  border-radius: 10px;
  color: var(--text-2);
  text-decoration: none;
  white-space: nowrap;
  transition:
    grid-template-columns 0.38s cubic-bezier(0.34, 1.56, 0.64, 1),
    color 0.14s,
    background 0.14s;
}
.nav-item:active {
  transform: scale(0.96);
}
.nav-item:hover {
  background: var(--panel-hover);
  color: var(--text-1);
  grid-template-columns: 48px 1fr 12px;
}
/* 激活项常驻展开：当前页面的标签不收回——Dock 任何时候都能看到
   「你在哪」，其他项仍保持悬停展开。展开态与 hover 一致，
   悬停激活项时 hover 的背景色覆盖（背景仍会响应）。 */
.nav-item.active {
  color: var(--accent);
  grid-template-columns: 48px 1fr 12px;
}
.nav-item.active .nav-tip {
  opacity: 1;
  transform: translateX(0);
  /* 常驻展开无需出场动画延迟：进入激活态时文字随按钮一起滑出 */
  transition:
    transform 0.36s cubic-bezier(0.34, 1.56, 0.64, 1) 0.03s,
    opacity 0.18s ease 0.05s;
}
/* 图标槽：恒 48×48（第一列），图标居中——默认态按钮即 48px 方格 */
.nav-icon-cell {
  grid-column: 1;
  width: 48px;
  height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
}
.nav-icon {
  font-size: 18px;
}
/* 文字（第二列）：0fr 时列宽 0，内容被裁；translateX 侧滑滑入/滑出 */
.nav-tip {
  grid-column: 2;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 13px;
  font-weight: 600;
  opacity: 0;
  transform: translateX(-12px);
  transition:
    transform 0.3s cubic-bezier(0.55, 0, 0.55, 0.2),
    opacity 0.22s ease;
}
.nav-item:hover .nav-tip {
  opacity: 1;
  transform: translateX(0);
  transition:
    transform 0.36s cubic-bezier(0.34, 1.56, 0.64, 1) 0.03s,
    opacity 0.18s ease 0.05s;
}
/* 新手引导态：展开禁用，避免 driver.js 高亮框随 hover 扩宽错位 */
.dock.touring .nav-item:hover {
  grid-template-columns: 48px 0fr 0px;
}
/* —— 固定实例条目：默认纯图标，悬停展开名称与取消固定按钮 ——
   pin-item 继承 .nav-item 的三列网格，hover 时第三列弹开容纳取消固定
   按钮；active（当前页面）态保持纯图标，不常驻展开。 */
.pin-item {
  overflow: hidden;
}
.pin-item .pin-icon-wrap {
  grid-column: 1;
  width: 48px;
  height: 48px;
}
/* 悬停展开：第三列 28px 容纳 22px 按钮并留右侧间距 */
.pin-item:hover,
.pin-item.active:hover {
  grid-template-columns: 48px 1fr 28px;
}
/* 激活态默认纯图标（覆盖 .nav-item.active 常驻展开），悬停时仍可展开取消固定 */
.pin-item.active {
  grid-template-columns: 48px 0fr 0px;
}
/* 实例名称（第二列）：默认 display:none 不占空间（避免 padding 撑开 pin-item 宽度），
   hover 时恢复显示，同 .nav-tip 的侧滑滑入/滑出 */
.pin-label {
  grid-column: 2;
  display: none;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 13px;
  font-weight: 600;
  padding-left: 8px;
  padding-right: 14px;
  opacity: 0;
  transform: translateX(-12px);
  transition:
    transform 0.3s cubic-bezier(0.55, 0, 0.55, 0.2),
    opacity 0.22s ease;
}
.pin-item:hover .pin-label {
  display: block;
  opacity: 1;
  transform: translateX(0);
  transition:
    transform 0.36s cubic-bezier(0.34, 1.56, 0.64, 1) 0.03s,
    opacity 0.18s ease 0.05s;
}
.stop-all-btn {
  position: relative;
  display: inline-grid;
  grid-template-columns: 48px 0fr 0px;
  align-items: center;
  height: 48px;
  padding: 0;
  border: 1px solid var(--danger-40);
  border-radius: 10px;
  background: var(--danger-12);
  color: var(--danger-text);
  font-size: 16px;
  white-space: nowrap;
  cursor: pointer;
  transition:
    grid-template-columns 0.38s cubic-bezier(0.34, 1.56, 0.64, 1),
    background 0.14s;
  flex-shrink: 0;
}
.stop-all-btn:hover {
  background: var(--danger-26);
  grid-template-columns: 48px 1fr 12px;
}
.stop-all-btn .nav-tip {
  grid-column: 2;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  opacity: 0;
  transform: translateX(-12px);
  transition:
    transform 0.3s cubic-bezier(0.55, 0, 0.55, 0.2),
    opacity 0.22s ease;
}
.stop-all-btn:hover .nav-tip {
  opacity: 1;
  transform: translateX(0);
  transition:
    transform 0.36s cubic-bezier(0.34, 1.56, 0.64, 1) 0.03s,
    opacity 0.18s ease 0.05s;
}

/* —— 启动按钮（贴窗口右缘）：一个整体按钮，内部分割 ——
   布局：[⌄ | 🖼  启动游戏]
              图标  ────────
                   实例名
   左段窄条（仅切换箭头，点击弹实例列表）+ 右段主体（大图标 +
   两行文字：上启动游戏 / 下实例名，点击直接启动）。
   主题色渐变铺满整按钮，中缝半透明描边分割两段；无卡片外壳。 */
.launch-zone {
  position: fixed;
  bottom: 10px;
  right: 10px;
  z-index: 44;
  display: flex;
  align-items: stretch;
  height: 60px;
  border-radius: 14px;
  background: linear-gradient(135deg, var(--accent), var(--accent-deep));
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.22);
  transition: filter 0.15s, transform 0.1s ease;
}
.launch-zone:hover {
  filter: brightness(1.07);
}
/* 左段：切换窄条（整高，仅下拉箭头） */
.split-switch {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  border: none;
  border-radius: 14px 0 0 14px;
  background: transparent;
  color: var(--accent-contrast);
  cursor: pointer;
  box-shadow: inset 0 0 0 var(--accent-12);
  transition: background 0.14s;
}
.split-switch:hover {
  background: rgba(0, 0, 0, 0.12);
}
.split-switch-chev {
  font-size: 15px;
  transition: transform 0.22s cubic-bezier(0.34, 1.3, 0.5, 1);
}
.split-switch-chev.open {
  transform: rotate(180deg);
}
/* 右段：主体（图标 + 两行文字），点击启动 */
.split-main {
  display: flex;
  align-items: center;
  gap: 11px;
  padding: 0 18px 0 13px;
  border: none;
  border-radius: 0 14px 14px 0;
  background: transparent;
  color: var(--accent-contrast);
  font-family: inherit;
  white-space: nowrap;
  cursor: pointer;
  /* 中缝分割线：左缘半透明深色描边 */
  box-shadow: inset 1px 0 0 rgba(0, 0, 0, 0.18);
}
.split-main:active:not(:disabled) {
  transform: scale(0.98);
}
.split-main:disabled {
  cursor: default;
  opacity: 0.75;
}
.launch-zone:has(.split-main:disabled) {
  filter: saturate(0.6) brightness(0.9);
}
.split-icon {
  width: 38px;
  height: 38px;
  flex-shrink: 0;
  border-radius: 10px;
  overflow: hidden;
  background: rgba(255, 255, 255, 0.18);
  display: flex;
  align-items: center;
  justify-content: center;
}
.split-icon :deep(.app-icon),
.split-icon :deep(img),
.split-icon :deep(svg) {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.split-text {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 1px;
  min-width: 0;
}
.split-title {
  font-size: 14px;
  font-weight: 700;
  line-height: 1.2;
}
.split-name {
  font-size: 11px;
  font-weight: 500;
  opacity: 0.85;
  max-width: 200px;
  line-height: 1.2;
}
.split-none {
  font-style: normal;
  opacity: 0.7;
}
/* 实例切换弹出列表：从按钮上方弹出，右对齐（贴右缘时不向右溢出）。
   ⚠ 两个坑的叠加：
   1. --panel 是低透明度色（深色主题仅 4% 白，为磨砂卡片设计），
      单独用作浮层背景必然透底；
   2. 父容器 .launch-zone 有 filter（hover 提亮）——filter 会隔离
      合成层，子元素的 backdrop-filter 采样不到真实页面背景，磨砂
      失效后只剩半透明底 = 依然透底。
   结论：浮层直接用实底色（bg-1 是纯实色，混 8% 白比背景亮一档），
      不依赖 backdrop-filter，任何环境下都不透底。 */
.inst-pop {
  position: absolute;
  bottom: calc(100% + 12px);
  right: 0;
  width: 300px;
  max-height: 320px;
  overflow-y: auto;
  padding: 6px;
  border-radius: 14px;
  background: color-mix(in srgb, var(--bg-1) 92%, #ffffff 8%);
  border: 1px solid var(--border);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.28);
  z-index: 5;
}
.inst-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 8px 10px;
  border: none;
  border-radius: 9px;
  background: transparent;
  color: var(--text-2);
  font-family: inherit;
  font-size: 13px;
  cursor: pointer;
  text-align: left;
  transition: background 0.12s, color 0.12s;
}
.inst-row:hover {
  background: var(--w-06);
  color: var(--text-1);
}
.inst-row.active {
  background: var(--accent-soft);
  color: var(--accent);
}
.inst-row-icon {
  width: 30px;
  height: 30px;
  flex-shrink: 0;
  border-radius: 7px;
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}
.inst-row-icon :deep(.app-icon),
.inst-row-icon :deep(img),
.inst-row-icon :deep(svg) {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.inst-row-name {
  font-weight: 600;
  flex: 1;
  min-width: 0;
}
.inst-row-meta {
  font-size: 11px;
  color: var(--text-3);
  flex-shrink: 0;
}
.inst-empty {
  padding: 18px 12px;
  text-align: center;
  color: var(--text-3);
  font-size: 12px;
}
/* 弹出动画：向上浮出 + 弹性 */
.inst-pop-enter-active {
  transition: opacity 0.2s ease, transform 0.26s cubic-bezier(0.34, 1.3, 0.5, 1);
}
.inst-pop-leave-active {
  transition: opacity 0.14s ease, transform 0.18s ease-in;
}
.inst-pop-enter-from {
  opacity: 0;
  transform: translateY(8px);
}
.inst-pop-leave-to {
  opacity: 0;
  transform: translateY(4px);
}
</style>
