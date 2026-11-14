<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { useRouter, onBeforeRouteLeave } from "vue-router";
import { useI18n } from "vue-i18n";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useInstancesStore } from "../stores/instances";
import { useAccountsStore } from "../stores/accounts";
import { usePinsStore, type PinItem } from "../stores/pins";
import { useSettingsStore } from "../stores/settings";
import { latencyInfo } from "../utils/format";
import { api } from "../api";
import { supportsQuickPlay } from "../version";
import { useMessage } from "naive-ui";
import AppIcon from "../components/AppIcon.vue";
import OnboardingBar from "../components/OnboardingBar.vue";
import HomeLayout from "../components/home/HomeLayout.vue";
import InstanceCard from "../components/InstanceCard.vue";
import { useHomeLayout } from "../home/useHomeLayout";
import type { ServerStatus } from "../types";
import {
  IconClose,
  IconFolder,
  IconGlobe,
  IconPlay,
  IconBookOpen,
  IconGrid,
  IconChevronRight,
} from "../components/icons";
import { useOnboarding } from "../composables/useOnboarding";

const router = useRouter();
const { t } = useI18n();
const instances = useInstancesStore();
const accounts = useAccountsStore();
const message = useMessage();
const pinsStore = usePinsStore();
const settingsStore = useSettingsStore();
const onboarding = useOnboarding();
const pinLaunching = ref<string>("");
const pinStatus = ref<Record<string, ServerStatus>>({});

// 自定义主页（home.json）：开关与文件操作都在「设置 → 外观 → 自定义主页」，
// 这里只消费共享状态并渲染；home.json 里的 reload 事件仍会触发重载。
const {
  layout: customLayout,
  path: customPath,
  error: customError,
  enabled: customEnabled,
  showRecent: showRecentInstances,
  ensureLoaded: ensureCustomLayout,
  reload: reloadCustomLayout,
} = useHomeLayout();

async function reloadCustom() {
  if (await reloadCustomLayout()) message.success(t("home.layout.reloaded"));
  else message.error(t("home.layout.parseError", { error: customError.value }));
}

const STORAGE_KEY = "qookix.home.selected";

const lastPlayed = computed(() =>
  [...instances.instances].sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0))[0] ?? null
);

// 常驻实例：默认取上次选择 / 最近游玩 / 第一个实例
const selectedId = ref<string | null>(null);

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

const hasAccount = computed(() => accounts.accounts.length > 0);

// 新手引导条「启动游戏」步骤的处理：启动当前选中实例
// （实例选择与启动的主入口已移至底部 Dock 的分割按钮）。
async function launchSelected() {
  const target = selected.value;
  if (!target) {
    message.info(t("home.message.noInstance"));
    router.push("/instances");
    return;
  }
  if (!hasAccount.value) {
    message.warning(t("home.message.addAccount"));
    accounts.showManager = true;
    return;
  }
  try {
    await instances.launch(target.id);
    // 新手引导第 3 步：首次启动成功即完成
    localStorage.setItem("qookix:onboarding:launched", "1");
    message.success(t("home.message.launched", { name: target.name }));
  } catch (e) {
    message.error(String(e));
  }
}

// —— 固定快捷启动 ——
// 只展示固定到首页的项（侧边栏固定不在此显示）；实例被删除后自动隐藏对应固定项
const validPins = computed(() =>
  pinsStore.items.filter((p) => p.target === "home" && instances.get(p.instanceId))
);

/**
 * 自定义主页（home.json）是否正在接管首页内容。
 * 关掉时首页必须有兜底内容——否则「有实例、没固定项」的用户进来只看到一张背景图。
 */
const hasCustomHome = computed(() => !!(customEnabled.value && customLayout.value));

/** 兜底内容：最近游玩的实例，最多 6 个（多到能填满两行，又不至于像第二个实例页） */
const recentInstances = computed(() =>
  [...instances.instances]
    .sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0))
    .slice(0, 6)
);

function pinIconSrc(p: PinItem): string | undefined {
  if (p.type !== "server") return undefined;
  const fav = pinStatus.value[p.id]?.favicon;
  if (fav) return fav;
  if (p.icon) return `data:image/png;base64,${p.icon}`;
  return undefined;
}

function worldIconSrc(p: PinItem): string | undefined {
  if (!p.icon) return undefined;
  if (p.icon.startsWith("http://") || p.icon.startsWith("https://") || p.icon.startsWith("data:")) return p.icon;
  return convertFileSrc(p.icon);
}

async function pingPin(p: PinItem) {
  if (p.type !== "server" || !p.address) return;
  try {
    const st = await api.pingServer(p.address);
    pinStatus.value = { ...pinStatus.value, [p.id]: st };
  } catch {
    pinStatus.value = {
      ...pinStatus.value,
      [p.id]: { online: false, address: p.address, name: null, version: null, players_online: null, players_max: null, motd: null, favicon: null, latency_ms: null, error: null },
    };
  }
}

async function launchPin(p: PinItem) {
  if (!hasAccount.value) {
    message.warning(t("home.message.addAccount"));
    accounts.showManager = true;
    return;
  }
  pinLaunching.value = p.id;
  if (p.type === "world" && !supportsQuickPlay(p.mcVersion)) {
    message.info(t("home.message.quickPlayUnsupported", { version: p.mcVersion }));
  }
  try {
    await instances.launch(p.instanceId, p.world, p.address);
    const msg =
      p.type === "server" ? t("home.message.joiningServer", { name: p.name })
      : p.type === "world" ? t("home.message.enteringWorld", { name: p.name })
      : t("home.message.launchingInstance", { name: p.name });
    message.success(msg);
  } catch (e) {
    message.error(String(e));
  } finally {
    pinLaunching.value = "";
  }
}

function unpin(p: PinItem) {
  pinsStore.remove(p.id);
}

function openInstance(p: PinItem) {
  if (p.type === "instance") {
    router.push(`/instance/${p.instanceId}`);
  } else {
    router.push({ path: `/instance/${p.instanceId}`, query: { tab: "saves" } });
  }
}

function pinTypeLabel(kind: PinItem["type"]): string {
  return kind === "server" ? t("home.pin.typeServer") : kind === "world" ? t("home.pin.typeWorld") : t("home.pin.typeInstance");
}

onMounted(() => {
  instances.load();
  accounts.load();
  settingsStore.load();
  void ensureCustomLayout();
  pinsStore.items.filter((p) => p.type === "server").forEach((p) => pingPin(p));
  // pin-card 逐个进入：每个延迟递增 60ms；section 在最后一个 pin-card 之后
  requestAnimationFrame(() => {
    const cards = document.querySelectorAll<HTMLElement>(".pin-card");
    cards.forEach((card, i) => {
      card.style.animationDelay = `${0.32 + i * 0.06}s`;
    });
    if (sectionEl.value) {
      const sectionGlass = sectionEl.value.querySelector<HTMLElement>(".glass");
      if (sectionGlass) {
        sectionGlass.style.animationDelay = `${0.32 + cards.length * 0.06 + 0.1}s`;
      }
    }
  });
});

// 离开首页时让 .section 往下滑出：先取消进场 animation 让 transform 回到默认值，
// reflow 后下一帧加 leaving class 触发 transition（translateY 抵消整页上移后净往下）
const sectionEl = ref<HTMLElement>();
onBeforeRouteLeave(() => {
  const glass = sectionEl.value?.querySelector<HTMLElement>(".glass");
  if (!glass) return;
  glass.style.animation = "none";
  void glass.offsetHeight;
  requestAnimationFrame(() => {
    glass.classList.add("leaving");
  });
});
</script>

<template>
  <div
    id="home-root"
    class="home qx-page"
    :class="{
      'has-custom': customEnabled && customLayout,
      'is-empty': showRecentInstances && !instances.instances.length,
    }"
    data-tauri-drag-region
  >
    <OnboardingBar @launch="launchSelected" />

    <div v-if="customEnabled && customError" class="qx-hint qx-hint--red">
      {{ t("home.layout.parseError", { error: customError }) }}
      {{ t("home.layout.goSettings") }}
    </div>

    <HomeLayout
      v-if="customEnabled && customLayout"
      :layout="customLayout"
      :home-path="customPath"
      @reload="reloadCustom"
    />

    <!-- 兜底内容：没开自定义主页时，首页要有东西可看。
         但「有东西」不等于强制——设置页可以把这块关掉，只留背景。 -->
    <section v-if="!hasCustomHome && showRecentInstances && recentInstances.length" class="home-default">
      <div class="hd-head">
        <h2 class="qx-section-title">{{ t("home.default.recentTitle") }}</h2>
        <button class="hd-all" @click="router.push('/instances')">
          {{ t("home.default.viewAll") }}
          <IconChevronRight />
        </button>
      </div>
      <div class="qx-grid qx-grid--card">
        <InstanceCard v-for="inst in recentInstances" :key="inst.id" :instance="inst" />
      </div>
    </section>

    <section v-if="showRecentInstances && validPins.length" class="pin-block">
      <div class="hd-head">
        <h2 class="qx-section-title">{{ t("home.default.pinTitle") }}</h2>
      </div>
      <div class="pin-grid">
        <div v-for="p in validPins" :key="p.id" class="pin-card glass clickable" @click="openInstance(p)">
          <div class="pin-icon">
            <template v-if="p.type === 'server'">
              <img v-if="pinIconSrc(p)" :src="pinIconSrc(p)" class="pin-icon-img" alt="" />
              <IconGlobe v-else />
            </template>
            <template v-else-if="p.type === 'world'">
              <img v-if="worldIconSrc(p)" :src="worldIconSrc(p)" class="pin-icon-img" alt="" />
              <IconFolder v-else />
            </template>
            <template v-else>
              <AppIcon :name="p.instanceIcon" />
            </template>
          </div>
          <div class="pin-info">
            <div class="pin-title text-ellipsis">{{ p.name }}</div>
            <div class="pin-meta">
              <span class="pin-type" :class="p.type">{{ pinTypeLabel(p.type) }}</span>
              <span v-if="p.type === 'instance'" class="pin-inst text-ellipsis">{{ p.mcVersion }} · {{ p.loader }}</span>
              <span v-else class="pin-inst text-ellipsis">{{ p.instanceName }}</span>
            </div>
            <div v-if="p.type === 'server'" class="pin-status">
              <span v-if="pinStatus[p.id]" :class="['latency', latencyInfo(pinStatus[p.id].latency_ms).tier]">
                <span class="bars">
                  <i v-for="n in 5" :key="n" :class="{ on: n <= latencyInfo(pinStatus[p.id].latency_ms).count }"></i>
                </span>
                <span v-if="pinStatus[p.id].latency_ms != null">{{ pinStatus[p.id].latency_ms }} ms</span>
                <span v-else-if="!pinStatus[p.id].online">{{ t("home.pin.offline") }}</span>
                <span v-else>…</span>
              </span>
              <span v-if="pinStatus[p.id]?.players_online != null" class="players">
                {{ t("home.pin.playersOnline", { count: pinStatus[p.id].players_online }) }}
              </span>
            </div>
          </div>
          <div class="pin-actions">
            <button class="pin-unpin" :title="t('home.pin.unpinTitle')" @click.stop="unpin(p)">
              <IconClose />
            </button>
            <button class="qx-btn qx-btn--primary" :disabled="pinLaunching === p.id" @click.stop="launchPin(p)">
              <IconPlay /> {{ pinLaunching === p.id ? t("home.pin.launching") : t("home.pin.launch") }}
            </button>
          </div>
        </div>
      </div>
    </section>

    <section v-if="showRecentInstances && !instances.instances.length" ref="sectionEl" class="section">
      <div class="qx-empty glass">
        <div class="empty-icon"><IconGrid /></div>
        <p>{{ t("home.empty.noInstance") }}</p>
        <div class="home-empty-actions">
          <button class="qx-btn qx-btn--primary" @click="router.push('/instances')">{{ t("home.empty.createFirst") }}</button>
          <button class="qx-btn" @click="onboarding.open">
            <IconBookOpen /> {{ t("home.empty.onboarding") }}
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.home {
  /* 骨架（纵向间距）来自 .qx-page，这里只补首页专属的定位上下文 */
  position: relative;
}
/* 有自定义主页时，底部「常驻实例」不再吸底：内容变长后吸底卡会盖住上面的自定义卡片。
   无自定义主页时保持原有吸底行为。 */
.home.has-custom .section {
  position: static;
  margin-top: 0;
}
/* 常驻实例卡片固定在窗口底部（sticky footer）。
    #app-content 是统一滚动容器，.section 用 sticky bottom:0 贴底，
    内容少时 min-height:100% 撑满，内容多时滚动始终可见。 */
.section {
  position: sticky;
  bottom: 0;
  width: 100%;
  margin-top: auto;
  padding-top: 8px;
  z-index: 1;
}
/* 进场动画在 .section > .glass 上而非 .section 上：
   父元素 transform 会隔离子元素 backdrop-filter，自身 transform 则不会 */
.section > .glass {
  animation: home-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) 0.40s backwards;
}
@keyframes home-card-enter {
  from {
    transform: translateY(28px);
    opacity: 0;
  }
  to {
    transform: none;
    opacity: 1;
  }
}
.section::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: -24px;
  bottom: 0;
  z-index: -1;
  pointer-events: none;
}
/* 离开首页时 .section 往右滑出。
   取消整页 overflow:hidden 让位移可见；
   translateX(64px) 抵消整页 translateX(-32px) 后净 +32px 往右 */
.slide-forward-leave-active {
  overflow: visible;
}
.section > .glass.leaving {
  transition: transform 0.22s ease-in, opacity 0.22s ease-in;
  transform: translateX(64px);
  opacity: 0;
}
/* —— 兜底内容：最近游玩 ——
   没开自定义主页时首页的默认主体。用 .qx-grid--card 与实例页同一套列宽，
   卡片直接复用 InstanceCard，交互（启动 / 目录 / 更多菜单）与实例页完全一致。 */
.home-default {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
}
.hd-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-5);
  padding: 0 2px;
}
.hd-all {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  /* 负外边距让文字右缘和卡片右缘对齐 */
  margin-right: -6px;
  padding: 4px 6px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--text-3);
  font-family: inherit;
  font-size: var(--fs-xs);
  cursor: pointer;
  transition:
    color var(--dur-sm) var(--ease-out),
    background var(--dur-sm) var(--ease-out);
}
.hd-all:hover {
  color: var(--accent);
  background: var(--w-06);
}
.hd-all svg {
  width: 13px;
  height: 13px;
}
/* 空态：没有实例时把「创建第一个实例」放到视觉中心，
   而不是按 .section 的吸底规则贴在窗口最下面 */
.home.is-empty .section {
  position: static;
  margin: auto 0;
}
/* 固定快捷启动 */
.pin-block {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  margin-top: 8px;
}
.pin-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 16px;
}
.pin-card {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 18px;
  cursor: pointer;
  box-shadow: var(--card-shadow);
  transition: transform 0.18s ease, box-shadow 0.18s ease, background 0.12s ease, border-color 0.12s ease;
  animation: home-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
/* 悬停：阴影加深，不位移；:not(:active) 排除按下态让 .clickable:active 的 scale(0.97) 接管按压 */
.pin-card:hover:not(:active) {
  box-shadow: var(--card-shadow-hover);
}
.pin-icon {
  width: 46px;
  height: 46px;
  border-radius: var(--r-md);
  flex-shrink: 0;
  background: linear-gradient(135deg, var(--accent-22), var(--accent-08));
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-lg);
  color: var(--accent);
  overflow: hidden;
}
.pin-icon-img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}
.pin-info {
  flex: 1;
  min-width: 0;
}
.pin-title {
  font-size: var(--fs-md);
  font-weight: 600;
  margin-bottom: 6px;
}
.pin-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-xs);
}
.pin-type {
  border-radius: var(--r-xs);
  padding: 0 6px;
  font-weight: 600;
  flex-shrink: 0;
}
.pin-type.server {
  background: rgba(90, 176, 255, 0.15);
  color: var(--text-2);
}
.pin-type.world {
  background: rgba(122, 208, 138, 0.15);
  color: var(--success);
}
.pin-type.instance {
  background: var(--accent-16);
  color: var(--accent);
}
.pin-inst {
  color: var(--text-3);
}
.pin-status {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 6px;
  font-size: var(--fs-xs);
}
.pin-status .latency {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.pin-status .bars {
  display: inline-flex;
  gap: 2px;
}
.pin-status .bars i {
  width: 4px;
  height: 10px;
  border-radius: var(--r-xs);
  background: var(--w-08);
}
.pin-status .bars i.on {
  background: currentColor;
}
.pin-status .latency.good {
  color: var(--success);
}
.pin-status .latency.mid {
  color: var(--warn);
}
.pin-status .latency.bad {
  color: var(--danger);
}
.pin-status .latency.off {
  color: var(--text-3);
}
.pin-status .players {
  color: var(--text-3);
}
.pin-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.pin-actions .qx-btn {
  height: 30px;
  padding: 0 12px;
}
.pin-unpin {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  color: var(--text-3);
  border-radius: var(--r-sm);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}
.pin-unpin:hover {
  background: var(--w-08);
  color: var(--danger);
}
.empty-icon {
  font-size: 36px;
  opacity: 0.6;
}
.home-empty-actions {
  display: flex;
  gap: 10px;
  align-items: center;
}
</style>
