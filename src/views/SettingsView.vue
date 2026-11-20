<script setup lang="ts">
import { motionScrollBehavior, prefersReducedMotion } from "../composables/useMotionPreference";
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useMessage } from "naive-ui";
import { useSettingsStore } from "../stores/settings";
import { useInstancesStore } from "../stores/instances";
import {
  IconCpu,
  IconDownload,
  IconFile,
  IconGlobe,
  IconHardDrive,
  IconImage,
  IconUsers,
  IconSearch,
  IconSliders,
  IconTerminal,
} from "../components/icons";
import CloudSyncDialog from "../components/CloudSyncDialog.vue";
import { useI18n } from "vue-i18n";
import { version as appVersion } from "../../package.json";
import { devMode, setDevMode as setDevModeFlag } from "../composables/useDevFlags";
import SettingsGeneral from "./settings/SettingsGeneral.vue";
import SettingsAppearance from "./settings/SettingsAppearance.vue";
import SettingsJava from "./settings/SettingsJava.vue";
import SettingsDownloads from "./settings/SettingsDownloads.vue";
import SettingsContent from "./settings/SettingsContent.vue";
import SettingsAccounts from "./settings/SettingsAccounts.vue";
import SettingsStorage from "./settings/SettingsStorage.vue";
import SettingsAbout from "./settings/SettingsAbout.vue";
import SettingsDeveloper from "./settings/SettingsDeveloper.vue";

const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();
const instances = useInstancesStore();
const cloudOpen = ref(false);
const changelogRevision = ref(0);
const tab = ref<SettingsTab>("general");

/**
 * 退出开发者模式时侧栏不再有「开发者」，把还停在该面板的视图切回「关于」。
 * 状态本身存在 useDevFlags（跨页共享），内网穿透的可见性也依赖它。
 */
let brandClickCount = 0;
let brandClickTimer: ReturnType<typeof setTimeout> | null = null;

function setDevMode(value: boolean) {
  setDevModeFlag(value);
  if (!value && tab.value === "dev") tab.value = "about";
}

function onBrandClick() {
  brandClickCount++;
  if (brandClickTimer) clearTimeout(brandClickTimer);
  brandClickTimer = setTimeout(() => { brandClickCount = 0; }, 1500);
  if (brandClickCount >= 7) {
    brandClickCount = 0;
    const next = !devMode.value;
    setDevMode(next);
    message.info(next ? t("settings.dev.enabled") : t("settings.dev.disabled"));
  }
}

const tabs = computed(() => {
  const list: { key: SettingsTab; label: string; icon: typeof IconSliders }[] = [
    { key: "general", label: t("settings.tab.general"), icon: IconSliders },
    { key: "appearance", label: t("settings.tab.appearance"), icon: IconImage },
    { key: "java", label: t("settings.tab.java"), icon: IconCpu },
    { key: "download", label: t("settings.tab.download"), icon: IconDownload },
    { key: "content", label: t("settings.tab.content"), icon: IconGlobe },
    { key: "accounts", label: t("settings.tab.accounts"), icon: IconUsers },
    { key: "storage", label: t("settings.tab.storage"), icon: IconHardDrive },
    { key: "about", label: t("settings.tab.about"), icon: IconFile },
  ];
  if (devMode.value) {
    list.push({ key: "dev", label: t("settings.tab.dev"), icon: IconTerminal });
  }
  return list;
});

// 面板切换方向（同主导航 slide-forward/back 的思路）：按目标页在侧边栏中的
// 排序决定新面板从下升起还是从上落下。watcher 在渲染前更新方向，Transition
// 换 key 触发过渡时读到的就是新方向。
const paneDir = ref<"forward" | "back">("forward");
watch(tab, (nv, ov) => {
  if (nv === ov) return;
  const order = (k: string) => tabs.value.findIndex((tp) => tp.key === k);
  paneDir.value = order(nv) > order(ov) ? "forward" : "back";
});
const paneTransitionName = computed(() => `settings-pane-${paneDir.value}`);

const searchQuery = ref("");

type SettingItem = { id: string; cardId: string; tab: SettingsTab; label: string; cardTitle: string };
const settingItemsDef: { id: string; cardId: string; tab: SettingsTab; labelKey: string; cardTitleKey: string }[] = [
  { id: "g-beh-close", cardId: "general-behavior", tab: "general", labelKey: "settings.general.closeWindow", cardTitleKey: "settings.general.behavior" },
  { id: "g-beh-update", cardId: "general-behavior", tab: "general", labelKey: "settings.general.autoUpdate", cardTitleKey: "settings.general.behavior" },
  { id: "g-datadir", cardId: "general-datadir", tab: "general", labelKey: "settings.general.dataDir", cardTitleKey: "settings.general.dataDir" },
  { id: "a-theme", cardId: "appearance-theme", tab: "appearance", labelKey: "settings.theme.label", cardTitleKey: "settings.theme.label" },
  { id: "a-theme-color", cardId: "appearance-theme", tab: "appearance", labelKey: "settings.appearance.themeColor", cardTitleKey: "settings.theme.label" },
  { id: "a-lang", cardId: "appearance-language", tab: "appearance", labelKey: "settings.language.label", cardTitleKey: "settings.language.label" },
  { id: "a-iface-hero", cardId: "appearance-interface", tab: "appearance", labelKey: "settings.appearance.homeHero", cardTitleKey: "settings.appearance.interface" },
  { id: "a-iface-news", cardId: "appearance-interface", tab: "appearance", labelKey: "settings.appearance.sidebarNews", cardTitleKey: "settings.appearance.interface" },
  { id: "a-iface-nav-browse", cardId: "appearance-interface", tab: "appearance", labelKey: "settings.appearance.nav.browse", cardTitleKey: "settings.appearance.interface" },
  { id: "a-iface-nav-instances", cardId: "appearance-interface", tab: "appearance", labelKey: "settings.appearance.nav.instances", cardTitleKey: "settings.appearance.interface" },
  { id: "a-iface-nav-multiplayer", cardId: "appearance-interface", tab: "appearance", labelKey: "settings.appearance.nav.multiplayer", cardTitleKey: "settings.appearance.interface" },
  { id: "a-iface-nav-skins", cardId: "appearance-interface", tab: "appearance", labelKey: "settings.appearance.nav.skins", cardTitleKey: "settings.appearance.interface" },
  { id: "a-iface-nav-toolbox", cardId: "appearance-interface", tab: "appearance", labelKey: "settings.appearance.nav.toolbox", cardTitleKey: "settings.appearance.interface" },
  { id: "a-iface-nav-timemachine", cardId: "appearance-interface", tab: "appearance", labelKey: "settings.appearance.nav.timemachine", cardTitleKey: "settings.appearance.interface" },
  { id: "a-home-layout", cardId: "appearance-home-layout", tab: "appearance", labelKey: "settings.appearance.homeLayout.title", cardTitleKey: "settings.appearance.homeLayout.title" },
  { id: "a-bg", cardId: "appearance-background", tab: "appearance", labelKey: "settings.appearance.background", cardTitleKey: "settings.appearance.background" },
  { id: "a-bg-blur", cardId: "appearance-background", tab: "appearance", labelKey: "settings.appearance.backgroundBlur", cardTitleKey: "settings.appearance.background" },
  { id: "a-bg-dim", cardId: "appearance-background", tab: "appearance", labelKey: "settings.appearance.backgroundDim", cardTitleKey: "settings.appearance.background" },
  { id: "a-glass", cardId: "appearance-glass", tab: "appearance", labelKey: "settings.appearance.glassStrength", cardTitleKey: "settings.appearance.glassCard" },
  { id: "j-runtime", cardId: "java-runtime", tab: "java", labelKey: "settings.java.runtime", cardTitleKey: "settings.java.runtime" },
  { id: "j-memory", cardId: "java-memory", tab: "java", labelKey: "settings.java.memory", cardTitleKey: "settings.java.memory" },
  { id: "j-jvmargs", cardId: "java-jvmargs", tab: "java", labelKey: "settings.java.jvmArgs", cardTitleKey: "settings.java.jvmArgs" },
  { id: "j-gameargs", cardId: "java-gameargs", tab: "java", labelKey: "settings.java.gameArgs", cardTitleKey: "settings.java.gameArgs" },
  { id: "d-threads", cardId: "download-parallel", tab: "download", labelKey: "settings.download.threadsLabel", cardTitleKey: "settings.download.parallel" },
  { id: "d-chunk", cardId: "download-parallel", tab: "download", labelKey: "settings.download.chunkThreadsLabel", cardTitleKey: "settings.download.parallel" },
  { id: "d-mirror", cardId: "download-mirror", tab: "download", labelKey: "settings.download.mirror", cardTitleKey: "settings.download.mirror" },
  { id: "d-center", cardId: "download-center", tab: "download", labelKey: "settings.download.center", cardTitleKey: "settings.download.center" },
  { id: "c-curseforge", cardId: "content-curseforge", tab: "content", labelKey: "settings.content.curseforgeKey", cardTitleKey: "settings.content.curseforgeKey" },
  { id: "c-mirror", cardId: "content-mirror", tab: "content", labelKey: "settings.content.mirror", cardTitleKey: "settings.content.mirror" },
  { id: "c-proxy", cardId: "content-proxy", tab: "content", labelKey: "settings.content.proxy", cardTitleKey: "settings.content.proxy" },
  { id: "c-translate", cardId: "content-translate", tab: "content", labelKey: "settings.content.translate", cardTitleKey: "settings.content.translate" },
  { id: "c-autobody", cardId: "content-translate", tab: "content", labelKey: "settings.content.autoBody", cardTitleKey: "settings.content.translate" },
  { id: "ac-third", cardId: "accounts-thirdparty", tab: "accounts", labelKey: "settings.accounts.thirdParty", cardTitleKey: "settings.accounts.thirdParty" },
  { id: "s-cloud", cardId: "storage-cloud", tab: "storage", labelKey: "settings.storage.cloudSave", cardTitleKey: "settings.storage.cloudSave" },
  { id: "s-stats", cardId: "storage-stats", tab: "storage", labelKey: "settings.storage.stats", cardTitleKey: "settings.storage.stats" },
  { id: "ab-overview", cardId: "about-showcase", tab: "about", labelKey: "settings.about.overview", cardTitleKey: "settings.about.overview" },
  { id: "ab-devs", cardId: "about-devs", tab: "about", labelKey: "settings.about.developers", cardTitleKey: "settings.about.developers" },
  { id: "ab-coop", cardId: "about-coop", tab: "about", labelKey: "settings.about.coopTitle", cardTitleKey: "settings.about.coopTitle" },
  { id: "ab-diag", cardId: "about-update", tab: "about", labelKey: "settings.about.diagReport", cardTitleKey: "settings.about.updateCard" },
  { id: "ab-update", cardId: "about-update", tab: "about", labelKey: "settings.about.checkUpdate", cardTitleKey: "settings.about.updateCard" },
  { id: "ab-upsrc", cardId: "about-update", tab: "about", labelKey: "settings.about.updateSource", cardTitleKey: "settings.about.updateCard" },
  { id: "ab-changelog", cardId: "about-changelog", tab: "about", labelKey: "settings.about.changelogTitle", cardTitleKey: "settings.about.changelogTitle" },
{ id: "ab-license", cardId: "about-license", tab: "about", labelKey: "settings.about.license", cardTitleKey: "settings.about.license" },
  { id: "ab-onboarding", cardId: "about-onboarding", tab: "about", labelKey: "settings.about.onboarding", cardTitleKey: "settings.about.onboarding" },
  { id: "ab-deps", cardId: "about-deps", tab: "about", labelKey: "settings.about.depsTitle", cardTitleKey: "settings.about.depsTitle" },
];

const allSettingItems = computed<SettingItem[]>(() =>
  settingItemsDef.map((d) => ({
    id: d.id,
    cardId: d.cardId,
    tab: d.tab,
    label: t(d.labelKey),
    cardTitle: t(d.cardTitleKey),
  })),
);

const settingGroups = computed(() => {
  const q = searchQuery.value.trim().toLowerCase();
  return tabs.value.map((tb) => {
    const all = allSettingItems.value.filter((it) => it.tab === tb.key);
    const items = q
      ? all.filter(
          (it) =>
            it.label.toLowerCase().includes(q) ||
            it.cardTitle.toLowerCase().includes(q),
        )
      : all;
    return { key: tb.key, label: tb.label, icon: tb.icon, items };
  });
});

function highlightSegments(text: string, q: string): { text: string; hit: boolean }[] {
  if (!q) return [{ text, hit: false }];
  const idx = text.toLowerCase().indexOf(q.toLowerCase());
  if (idx < 0) return [{ text, hit: false }];
  return [
    { text: text.slice(0, idx), hit: false },
    { text: text.slice(idx, idx + q.length), hit: true },
    { text: text.slice(idx + q.length), hit: false },
  ].filter((s) => s.text.length > 0);
}

const hasResults = computed(() => settingGroups.value.some((g) => g.items.length));

const activeItemId = ref("");
const skipPaneTransition = ref(false);

function scrollToSetting(id: string) {
  const el = document.getElementById(id);
  if (!el) return;
  const scroller = document.getElementById("app-content");
  if (scroller) {
    const sr = scroller.getBoundingClientRect();
    const tr = el.getBoundingClientRect();
    const offset =
      tr.top - sr.top + scroller.scrollTop - (scroller.clientHeight - tr.height) / 2;
    scroller.scrollTo({ top: Math.max(0, offset), behavior: motionScrollBehavior() });
  } else {
    el.scrollIntoView({ behavior: motionScrollBehavior(), block: "center" });
  }
  el.classList.add("setting-flash");
  setTimeout(() => el.classList.remove("setting-flash"), 1200);
}
function selectSetting(item: SettingItem) {
  activeItemId.value = item.id;
  if (tab.value !== item.tab) {
    skipPaneTransition.value = true;
    tab.value = item.tab;
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        skipPaneTransition.value = false;
        scrollToSetting(item.cardId);
      });
    });
  } else {
    scrollToSetting(item.cardId);
  }
}
let saveTimer: ReturnType<typeof setTimeout> | null = null;

async function save() {
  try {
    await settings.save();
  } catch (e) {
    message.error(String(e));
  }
}

watch(
  () => settings.settings,
  (current, previous) => {
    // Store acknowledgements replace the settings object. In-place form edits
    // debounce together even while the user moves between panels.
    if (!current || current !== previous) return;
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      saveTimer = null;
      void save();
    }, 500);
  },
  { deep: true, flush: "sync" }
);

onMounted(() => { void settings.load().catch((error) => message.error(String(error))); });
onBeforeUnmount(() => {
  if (saveTimer) {
    clearTimeout(saveTimer);
    void save();
  }
  if (brandClickTimer) clearTimeout(brandClickTimer);
});

const panels = {
  general: SettingsGeneral,
  appearance: SettingsAppearance,
  java: SettingsJava,
  download: SettingsDownloads,
  content: SettingsContent,
  accounts: SettingsAccounts,
  storage: SettingsStorage,
  about: SettingsAbout,
  dev: SettingsDeveloper,
} as const;
type SettingsTab = keyof typeof panels;
const activePanel = computed(() => panels[tab.value]);
</script>

<template>
  <div v-if="settings.settings" class="settings-view" data-tauri-drag-region>
    <div class="settings-side">
      <div class="nav-search-card glass">
        <IconSearch class="nav-search-icon" />
        <input
          v-model="searchQuery"
          type="text"
          class="nav-search-input"
          :placeholder="t('settings.search.placeholder')"
        />
      </div>
      <aside id="settings-nav" class="settings-nav">
        <nav class="nav-list">
          <template v-if="searchQuery.trim()">
            <div v-if="!hasResults" class="nav-empty">{{ t('settings.search.noResult') }}</div>
            <template v-for="g in settingGroups" :key="g.key">
              <div v-if="g.items.length" class="nav-group">
                <div class="nav-group-head">
                  <component :is="g.icon" class="nav-group-icon" />
                  <span>{{ g.label }}</span>
                </div>
                <button
                  v-for="item in g.items"
                  :key="item.id"
                  class="nav-result"
                  :class="{ active: activeItemId === item.id }"
                  @click="selectSetting(item)"
                >
                  <span class="nav-result-title">
                    <template v-for="(seg, i) in highlightSegments(item.label, searchQuery.trim())" :key="i">
                      <mark v-if="seg.hit" class="hl">{{ seg.text }}</mark>
                      <template v-else>{{ seg.text }}</template>
                    </template>
                  </span>
                  <span class="nav-result-sub">
                    <template v-for="(seg, i) in highlightSegments(item.cardTitle, searchQuery.trim())" :key="i">
                      <mark v-if="seg.hit" class="hl">{{ seg.text }}</mark>
                      <template v-else>{{ seg.text }}</template>
                    </template>
                  </span>
                </button>
              </div>
            </template>
          </template>
          <template v-else>
            <button
              v-for="t in tabs"
              :key="t.key"
              class="nav-item"
              :class="{ active: tab === t.key }"
              @click="tab = t.key"
            >
              <component :is="t.icon" class="nav-icon" />
              <span>{{ t.label }}</span>
            </button>
          </template>
        </nav>
      </aside>
      <div class="nav-brand glass" @click="onBrandClick">
        <img src="/app-icon.png" class="nav-brand-logo" draggable="false" alt="" />
        <div class="nav-brand-info">
          <span class="nav-brand-name">QookiX Launcher</span>
          <span class="nav-brand-ver">v{{ appVersion }}</span>
        </div>
      </div>
    </div>
    <Transition :name="paneTransitionName" :css="!skipPaneTransition && !prefersReducedMotion" :mode="skipPaneTransition || prefersReducedMotion ? undefined : 'out-in'">
      <KeepAlive>
        <component
          :is="activePanel"
          :key="tab"
          v-bind="tab === 'accounts' ? { cloudOpen } : tab === 'about' ? { changelogRevision } : {}"
          @open-cloud="cloudOpen = true"
          @open-accounts="tab = 'accounts'"
          @set-dev-mode="setDevMode"
          @reload-changelog="changelogRevision++"
        />
      </KeepAlive>
    </Transition>
    <!-- 云存档浏览（浏览模式：不绑定实例，恢复时在弹窗内选择实例） -->
    <CloudSyncDialog
      v-model:show="cloudOpen"
      instance-id=""
      world=""
      game-version=""
      @restored="instances.load(true)"
    />
  </div>
</template>

<style src="./settings/settings.css"></style>
