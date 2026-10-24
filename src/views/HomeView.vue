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
import type { ServerStatus } from "../types";
import { IconClose, IconFolder, IconGlobe, IconPlay, IconBookOpen } from "../components/icons";
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
  <div id="home-root" class="home" data-tauri-drag-region>
    <OnboardingBar @launch="launchSelected" />

    <section v-if="validPins.length" class="pin-block">
      <div class="pin-grid">
        <div v-for="p in validPins" :key="p.id" class="pin-card glass" @click="openInstance(p)">
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
            <button class="btn primary" :disabled="pinLaunching === p.id" @click.stop="launchPin(p)">
              <IconPlay /> {{ pinLaunching === p.id ? t("home.pin.launching") : t("home.pin.launch") }}
            </button>
          </div>
        </div>
      </div>
    </section>

    <section ref="sectionEl" class="section">
      <div v-if="!instances.instances.length" class="empty glass">
        <p>{{ t("home.empty.noInstance") }}</p>
        <div class="home-empty-actions">
          <button class="btn primary" @click="router.push('/instances')">{{ t("home.empty.createFirst") }}</button>
          <button class="btn ghost" @click="onboarding.open">
            <IconBookOpen /> {{ t("home.empty.onboarding") }}
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.home {
  display: flex;
  flex-direction: column;
  gap: 16px;
  min-height: 100%;
}
.btn {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  border: none;
  border-radius: 10px;
  padding: 9px 18px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: all 0.15s;
  font-family: inherit;
}
.btn.primary {
  background: linear-gradient(135deg, var(--accent), var(--accent-deep));
  color: var(--accent-contrast);
}
.btn.primary:hover {
  filter: brightness(1.08);
}
.btn.primary:disabled {
  opacity: 0.5;
  cursor: default;
}
.btn.ghost {
  background: var(--w-06);
  color: var(--text-1);
  border: 1px solid var(--border);
}
.btn.ghost:hover {
  background: var(--w-10);
  transform: none;
}
/* 常驻实例卡片固定在窗口底部（sticky footer）。
    #app-content 是统一滚动容器，.section 用 sticky bottom:0 贴底，
    内容少时 min-height:100% 撑满，内容多时滚动始终可见。 */
.section {
  position: sticky;
  bottom: 0;
  width: 100%;
  max-width: 1080px;
  margin-top: auto;
  margin-left: auto;
  margin-right: auto;
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
/* 离开首页时 .section 往下滑出。
   取消整页 overflow:hidden 让位移可见；
   translateY(64px) 抵消整页 translateY(-32px) 后净 +32px 往下 */
.slide-forward-leave-active {
  overflow: visible;
}
.section > .glass.leaving {
  transition: transform 0.22s ease-in, opacity 0.22s ease-in;
  transform: translateY(64px);
  opacity: 0;
}
/* 固定快捷启动 */
.pin-block {
  margin-top: 8px;
}
.pin-sub {
  color: var(--text-3);
  font-size: 12px;
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
  animation: home-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
.pin-icon {
  width: 46px;
  height: 46px;
  border-radius: 12px;
  flex-shrink: 0;
  background: linear-gradient(135deg, var(--accent-22), var(--accent-08));
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 20px;
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
  font-size: 15px;
  font-weight: 600;
  margin-bottom: 6px;
}
.pin-meta {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}
.pin-type {
  border-radius: 6px;
  padding: 0 6px;
  font-weight: 600;
  flex-shrink: 0;
}
.pin-type.server {
  background: rgba(90, 176, 255, 0.15);
  color: #5ab0ff;
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
  font-size: 12px;
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
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.15);
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
.pin-actions .btn {
  padding: 8px 14px;
  font-size: 13px;
}
.pin-unpin {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  color: var(--text-3);
  border-radius: 8px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
}
.pin-unpin:hover {
  background: var(--w-08);
  color: var(--danger);
}
.section-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  margin-bottom: 14px;
}
.section-head h2 {
  font-size: 17px;
  margin: 0;
}
.link-btn {
  background: none;
  border: none;
  color: var(--text-3);
  cursor: pointer;
  font-size: 13px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.link-btn:hover {
  color: var(--accent);
}
.empty {
  padding: 48px 24px;
  text-align: center;
  color: var(--text-3);
  display: flex;
  flex-direction: column;
  gap: 12px;
  align-items: center;
}
.home-empty-actions {
  display: flex;
  gap: 10px;
  align-items: center;
}
</style>
