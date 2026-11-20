<script setup lang="ts">
import { vSegmented } from "../../composables/vSegmented";
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { NSlider, useMessage, useDialog } from "naive-ui";
import { openPath as openSystemPath } from "@tauri-apps/plugin-opener";
import { open } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useSettingsStore } from "../../stores/settings";
import { api } from "../../api";
import { useSlidingIndicator } from "../../composables/useSlidingIndicator";
import { useHomeLayout } from "../../home/useHomeLayout";
import { BACKGROUND_SCENES, resolveScene } from "../../components/background/scenes";
import { BG_QUALITIES, resolveQuality, type BgQualityId } from "../../components/background/quality";
import { IconExternal, IconRefresh } from "../../components/icons";
import ColorPicker from "../../components/ColorPicker.vue";
import { useI18n } from "vue-i18n";
import { SUPPORTED_LOCALES } from "../../i18n";

const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();
const dialog = useDialog();

const NAV_TOGGLE_ITEMS = [
  "browse",
  "instances",
  "multiplayer",
  "skins",
  "toolbox",
  "timemachine",
] as const;
function navVisible(name: string): boolean {
  return !(settings.settings?.hidden_nav_items ?? []).includes(name);
}
function toggleNav(name: string) {
  const cur = new Set(settings.settings?.hidden_nav_items ?? []);
  if (cur.has(name)) cur.delete(name);
  else cur.add(name);
  settings.patch({ hidden_nav_items: Array.from(cur) });
}

// 主题 seg 滑动高亮
const themeSegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: themeSegStyle, refresh: refreshThemeSeg } = useSlidingIndicator(
  themeSegRef,
  () => Array.from(themeSegRef.value?.querySelectorAll<HTMLElement>(".seg button") ?? []),
  () => (settings.settings?.theme === "light" ? 1 : 0),
  { axis: "horizontal" }
);
watch(() => settings.settings?.theme, () => nextTick(() => refreshThemeSeg()));

// 主题色预设
const themeColorPresets = [
  "#e89a4b",
  "#ff6b35",
  "#e5534b",
  "#ec4899",
  "#8b5cf6",
  "#5aa2f0",
  "#22d3ee",
  "#4ec9a0",
  "#f5c518",
];
const colorPickerShow = ref(false);
function onCustomColorPick(hex: string) {
  settings.patch({ theme_color: hex });
}
const isCustomThemeColor = computed(() => {
  const cur = (settings.settings?.theme_color ?? "").toLowerCase();
  return cur !== "" && !themeColorPresets.some((c) => c.toLowerCase() === cur);
});

// 语言 seg 滑动高亮（简体中文 / English）
const langSegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: langSegStyle, refresh: refreshLangSeg } = useSlidingIndicator(
  langSegRef,
  () => Array.from(langSegRef.value?.querySelectorAll<HTMLElement>(".seg button") ?? []),
  () => Math.max(0, SUPPORTED_LOCALES.findIndex((l) => l.value === settings.settings?.language)),
  { axis: "horizontal" }
);
watch(() => settings.settings?.language, () => nextTick(() => refreshLangSeg()));

// ---- 自定义主页（home.json）----
// 与首页共用同一份状态：这里改开关 / 重载文件，首页立刻跟着变。
const {
  enabled: homeLayoutEnabled,
  path: homeLayoutPath,
  error: homeLayoutError,
  loading: homeLayoutLoading,
  showRecent: homeShowRecent,
  setEnabled: setHomeLayoutEnabled,
  setShowRecent: setHomeShowRecent,
  ensureLoaded: ensureHomeLayout,
  reload: reloadHomeLayoutState,
  reset: resetHomeLayoutState,
} = useHomeLayout();

function updateHomePreference(save: () => void) {
  try {
    save();
  } catch (error) {
    message.error(String(error));
  }
}

async function editHomeLayoutFile() {
  if (!homeLayoutPath.value) return;
  try {
    await openSystemPath(homeLayoutPath.value);
  } catch (e) {
    message.error(t("home.layout.openFailed", { error: String(e) }));
  }
}

async function reloadHomeLayout() {
  if (await reloadHomeLayoutState()) message.success(t("home.layout.reloaded"));
  else message.error(t("home.layout.parseError", { error: homeLayoutError.value }));
}

/** 删除 home.json 并重新生成模板：有代价，必须二次确认 */
function resetHomeLayout() {
  dialog.warning({
    title: t("settings.appearance.homeLayout.resetTitle"),
    content: t("settings.appearance.homeLayout.resetConfirm"),
    positiveText: t("common.confirm"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      if (await resetHomeLayoutState()) {
        message.success(t("settings.appearance.homeLayout.resetDone"));
      } else {
        message.error(t("home.layout.parseError", { error: homeLayoutError.value }));
      }
    },
  });
}

async function pickBackground() {
  try {
    const picked = await open({
      multiple: false,
      filters: [{ name: t("settings.imageFilter"), extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp"] }],
    });
    if (!picked || typeof picked !== "string") return;
    const path = await api.importBackgroundImage(picked);
    await settings.patch({ background_image: path });
  } catch (e) {
    message.error(String(e));
  }
}

const bgPreviewUrl = computed(() => {
  const p = settings.settings?.background_image;
  return p ? convertFileSrc(p) : "";
});

// —— 背景类型：渐变 / 图片 / 3D 动态场景 ——
// 后端 background_mode 为空串表示「自动」（老配置没有该字段）：有图用图，无图渐变。
const BG_MODES = ["gradient", "image", "scene"] as const;
type BgMode = (typeof BG_MODES)[number];
const bgMode = computed<BgMode>(() => {
  const s = settings.settings;
  if (!s) return "gradient";
  const m = s.background_mode;
  if (m === "gradient" || m === "image" || m === "scene") return m;
  return s.background_image ? "image" : "gradient";
});
const bgModeSegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: bgModeSegStyle, refresh: refreshBgModeSeg } = useSlidingIndicator(
  bgModeSegRef,
  () => Array.from(bgModeSegRef.value?.querySelectorAll<HTMLElement>(".seg button") ?? []),
  () => Math.max(0, BG_MODES.indexOf(bgMode.value)),
  { axis: "horizontal" }
);
// 换模式会重排卡片（图片模式多出预览、场景模式才渲染场景网格与画质行），
// 两个 seg 都要重新量：画质那套此前可能根本不在 DOM 里。
watch(bgMode, () =>
  nextTick(() => {
    refreshBgModeSeg();
    refreshBgQualitySeg();
  })
);

// 当前动态场景 id（空值回落默认）；场景清单来自 scenes.ts，新增场景不用改这里
const bgScene = computed(() => resolveScene(settings.settings?.background_scene).id);
async function setBgScene(id: string) {
  await settings.patch({ background_scene: id });
}

// 动态场景画质档（空值回落均衡档）
const bgQualityId = computed(() => resolveQuality(settings.settings?.background_quality).id);
const bgQualitySegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: bgQualitySegStyle, refresh: refreshBgQualitySeg } = useSlidingIndicator(
  bgQualitySegRef,
  () => Array.from(bgQualitySegRef.value?.querySelectorAll<HTMLElement>(".seg button") ?? []),
  () => Math.max(0, BG_QUALITIES.findIndex((item) => item.id === bgQualityId.value)),
  { axis: "horizontal" }
);
watch(bgQualityId, () => nextTick(() => refreshBgQualitySeg()));
async function setBgQuality(id: BgQualityId) {
  await settings.patch({ background_quality: id });
}

async function setBgMode(mode: BgMode) {
  // 选「图片」但还没选过图时直接把选择框弹出来，否则点了没有任何反应
  if (mode === "image" && !settings.settings?.background_image) {
    await pickBackground();
    if (!settings.settings?.background_image) return;
  }
  await settings.patch({ background_mode: mode });
}

function clearBackgroundImage() {
  // 图没了还留在图片模式会只剩底色，顺手退回渐变
  settings.patch({ background_image: null, background_mode: "gradient" });
}

onMounted(() => { void ensureHomeLayout(); });
</script>

<template>
  <div v-if="settings.settings" class="settings-body">
    <div class="settings-pane">
      <div class="card glass" id="appearance-theme">
        <h3>{{ t("settings.theme.label") }}</h3>
        <div class="choice-row">
          <span>{{ t("settings.theme.label") }}</span>
          <div ref="themeSegRef" v-segmented="{ label: t('settings.theme.label') }" class="seg qx-seg qx-seg--sm">
            <div class="qx-seg__indicator" :style="themeSegStyle"></div>
            <button
              :class="{ active: settings.settings.theme === 'dark' }"
              @click="settings.patch({ theme: 'dark' })"
            >
              {{ t("settings.theme.dark") }}
            </button>
            <button
              :class="{ active: settings.settings.theme === 'light' }"
              @click="settings.patch({ theme: 'light' })"
            >
              {{ t("settings.theme.light") }}
            </button>
          </div>
        </div>
        <div class="appearance-divider"></div>
        <div class="choice-row">
          <span>{{ t("settings.appearance.themeColor") }}</span>
          <div class="theme-color-row">
            <button
              v-for="c in themeColorPresets"
              :key="c"
              type="button"
              class="color-swatch"
              :class="{ active: settings.settings.theme_color === c }"
              :style="{ background: c }"
              :title="c"
              @click="settings.patch({ theme_color: c })"
            ></button>
            <button
              type="button"
              class="color-custom"
              :class="{ active: isCustomThemeColor }"
              :title="t('settings.appearance.customColor')"
              @click="colorPickerShow = true"
            >
              <span class="color-custom-ring" :style="{ background: settings.settings.theme_color }"></span>
            </button>
            <ColorPicker
              :show="colorPickerShow"
              :color="settings.settings.theme_color"
              :presets="themeColorPresets"
              @update:show="colorPickerShow = $event"
              @update:color="onCustomColorPick"
            />
          </div>
        </div>
      </div>
      <div class="card glass" id="appearance-language">
        <h3>{{ t("settings.language.label") }}</h3>
        <div class="choice-row">
          <div class="choice-info">
            <span class="choice-label">{{ t("settings.language.label") }}</span>
            <p class="choice-hint">{{ t("settings.language.desc") }}</p>
          </div>
          <div ref="langSegRef" v-segmented="{ label: t('settings.language.label') }" class="seg lang-seg qx-seg qx-seg--sm">
            <div class="qx-seg__indicator" :style="langSegStyle"></div>
            <button
              v-for="l in SUPPORTED_LOCALES"
              :key="l.value"
              :class="{ active: settings.settings.language === l.value }"
              @click="settings.patch({ language: l.value })"
            >
              {{ l.label }}
            </button>
          </div>
        </div>
      </div>
      <div class="card glass" id="appearance-interface">
        <h3>{{ t("settings.appearance.interface") }}</h3>
        <div class="choice-row">
          <div class="choice-info">
            <span class="choice-label">{{ t("settings.appearance.sidebarNews") }}</span>
            <p class="choice-hint">{{ t("settings.appearance.sidebarNewsHint") }}</p>
          </div>
          <button
            class="toggle"
            :class="{ on: settings.settings.show_news ?? true }"
            role="switch"
            :aria-checked="settings.settings.show_news ?? true"
            @click="settings.patch({ show_news: !(settings.settings.show_news ?? true) })"
          >
            <span class="knob"></span>
          </button>
        </div>
        <div class="appearance-divider"></div>
        <div v-for="name in NAV_TOGGLE_ITEMS" :key="name" class="choice-row">
          <div class="choice-info">
            <span class="choice-label">{{ t(`settings.appearance.nav.${name}`) }}</span>
            <p class="choice-hint">{{ t(`settings.appearance.nav.${name}Hint`) }}</p>
          </div>
          <button
            class="toggle"
            :class="{ on: navVisible(name) }"
            role="switch"
            :aria-checked="navVisible(name)"
            @click="toggleNav(name)"
          >
            <span class="knob"></span>
          </button>
        </div>
      </div>

      <div class="card glass" id="appearance-home-layout">
        <h3>{{ t("settings.appearance.homeLayout.title") }}</h3>
        <div class="choice-row">
          <div class="choice-info">
            <span class="choice-label">{{ t("settings.appearance.homeLayout.recent") }}</span>
            <p class="choice-hint">{{ t("settings.appearance.homeLayout.recentHint") }}</p>
          </div>
          <button
            class="toggle"
            :class="{ on: homeShowRecent }"
            role="switch"
            :aria-checked="homeShowRecent"
            @click="updateHomePreference(() => setHomeShowRecent(!homeShowRecent))"
          >
            <span class="knob"></span>
          </button>
        </div>
        <div class="appearance-divider"></div>
        <div class="choice-row">
          <div class="choice-info">
            <span class="choice-label">{{ t("settings.appearance.homeLayout.enable") }}</span>
            <p class="choice-hint">{{ t("settings.appearance.homeLayout.enableHint") }}</p>
          </div>
          <button
            class="toggle"
            :class="{ on: homeLayoutEnabled }"
            role="switch"
            :aria-checked="homeLayoutEnabled"
            @click="updateHomePreference(() => setHomeLayoutEnabled(!homeLayoutEnabled))"
          >
            <span class="knob"></span>
          </button>
        </div>
        <div class="appearance-divider"></div>
        <div class="dir-row">
          <code class="mono dir">{{ homeLayoutPath || "home.json" }}</code>
        </div>
        <div class="settings-actions">
          <button class="qx-btn qx-btn--sm" :disabled="!homeLayoutPath" @click="editHomeLayoutFile">
            <IconExternal /> {{ t("settings.appearance.homeLayout.edit") }}
          </button>
          <button class="qx-btn qx-btn--sm" :disabled="homeLayoutLoading" @click="reloadHomeLayout">
            <IconRefresh /> {{ t("settings.appearance.homeLayout.reload") }}
          </button>
          <button class="qx-btn qx-btn--sm qx-btn--danger" :disabled="homeLayoutLoading" @click="resetHomeLayout">
            {{ t("settings.appearance.homeLayout.reset") }}
          </button>
        </div>
        <div v-if="homeLayoutError" class="qx-hint qx-hint--red home-layout-error">
          {{ t("home.layout.parseError", { error: homeLayoutError }) }}
        </div>
        <p class="hint">{{ t("settings.appearance.homeLayout.fileHint") }}</p>
      </div>

      <div class="card glass" id="appearance-background">
        <h3>{{ t("settings.appearance.background") }}</h3>
        <div class="bg-mode-row">
          <div ref="bgModeSegRef" v-segmented="{ label: t('settings.appearance.background') }" class="seg qx-seg qx-seg--sm">
            <div class="qx-seg__indicator" :style="bgModeSegStyle"></div>
            <button
              :class="{ active: bgMode === 'gradient' }"
              @click="setBgMode('gradient')"
            >
              {{ t("settings.appearance.bgGradient") }}
            </button>
            <button :class="{ active: bgMode === 'image' }" @click="setBgMode('image')">
              {{ t("settings.appearance.bgImage") }}
            </button>
            <button :class="{ active: bgMode === 'scene' }" @click="setBgMode('scene')">
              {{ t("settings.appearance.bgScene") }}
            </button>
          </div>
        </div>

        <!-- 图片模式 -->
        <template v-if="bgMode === 'image'">
          <div v-if="settings.settings.background_image" class="bg-preview">
            <img :src="bgPreviewUrl" :alt="t('settings.appearance.backgroundPreview')" />
          </div>
          <div class="choice-row">
            <span class="hint-inline">
              {{
                settings.settings.background_image
                  ? t("settings.appearance.imageSelected")
                  : t("settings.appearance.imageNone")
              }}
            </span>
            <div class="bg-actions">
              <button class="mini-btn" @click="pickBackground">
                {{
                  settings.settings.background_image
                    ? t("settings.appearance.changeImage")
                    : t("settings.appearance.selectImage")
                }}
              </button>
              <button
                v-if="settings.settings.background_image"
                class="mini-btn"
                @click="clearBackgroundImage"
              >
                {{ t("settings.appearance.clear") }}
              </button>
            </div>
          </div>
          <div v-if="settings.settings.background_image" class="tune-block">
            <div class="tune-row">
              <label>{{ t("settings.appearance.backgroundBlur") }}</label>
              <NSlider
                v-model:value="settings.settings.background_blur"
                class="tune-slider"
                :min="0"
                :max="50"
                :step="1"
                :tooltip="false"
              />
              <span class="tune-val">{{ settings.settings.background_blur }} px</span>
            </div>
            <div class="tune-row">
              <label>{{ t("settings.appearance.backgroundDim") }}</label>
              <NSlider
                v-model:value="settings.settings.background_dim"
                class="tune-slider"
                :min="0"
                :max="100"
                :step="5"
                :tooltip="false"
              />
              <span class="tune-val">{{ settings.settings.background_dim }}%</span>
            </div>
          </div>
        </template>

        <!-- 动态场景模式：先选场景，再调遮罩 -->
        <template v-else-if="bgMode === 'scene'">
          <div class="scene-grid" role="radiogroup" :aria-label="t('settings.appearance.bgScene')">
            <button
              v-for="s in BACKGROUND_SCENES"
              :key="s.id"
              type="button"
              role="radio"
              :aria-checked="bgScene === s.id"
              class="scene-card"
              :class="{ active: bgScene === s.id }"
              @click="setBgScene(s.id)"
            >
              <!-- 缩略图是手绘剪影，不跑真实渲染：开设置页就为几张预览图初始化 WebGL 不划算 -->
              <span class="scene-thumb" :class="`scene-thumb--${s.thumb}`">
                <svg v-if="s.thumb === 'island'" viewBox="0 0 128 72" aria-hidden="true">
                  <circle class="sc-sun" cx="96" cy="25" r="10" />
                  <rect class="sc-sea" y="46" width="128" height="26" />
                  <path class="sc-land" d="M26 47c9-17 18-23 26-23s18 9 27 23z" />
                  <path class="sc-tree" d="M86 47l5-10 5 10z" />
                </svg>
                <svg v-else viewBox="0 0 128 72" aria-hidden="true">
                  <circle class="sc-sun sc-sun--warm" cx="34" cy="27" r="9" />
                  <path class="sc-cloud" d="M74 26c4-7 13-7 16 0 5-1 9 2 9 6H66c0-4 4-6 8-6z" />
                  <path class="sc-cloud sc-cloud--far" d="M16 41c4-6 12-6 15 0 5-1 8 2 8 5H8c0-4 4-5 8-5z" />
                  <path class="sc-tower" d="M22 26h3v21h-3zM103 26h3v21h-3z" />
                  <path class="sc-cable" d="M23.5 27c18 11 63 11 81 0" fill="none" />
                  <rect class="sc-deck" y="46" width="128" height="2.4" />
                  <rect class="sc-train" x="50" y="39" width="27" height="6.4" rx="1" />
                </svg>
              </span>
              <span class="scene-meta">
                <span class="scene-label">{{ t(s.labelKey) }}</span>
                <span class="scene-desc">{{ t(s.descKey) }}</span>
              </span>
            </button>
          </div>
          <div class="tune-block">
            <div class="tune-row">
              <label>{{ t("settings.appearance.backgroundDim") }}</label>
              <NSlider
                v-model:value="settings.settings.background_dim"
                class="tune-slider"
                :min="0"
                :max="100"
                :step="5"
                :tooltip="false"
              />
              <span class="tune-val">{{ settings.settings.background_dim }}%</span>
            </div>
            <div class="tune-row">
              <label>{{ t("settings.appearance.bgQuality") }}</label>
              <div ref="bgQualitySegRef" v-segmented="{ label: t('settings.appearance.bgQuality') }" class="seg qx-seg qx-seg--sm bg-quality-seg">
                <div class="qx-seg__indicator" :style="bgQualitySegStyle"></div>
                <button
                  v-for="item in BG_QUALITIES"
                  :key="item.id"
                  :class="{ active: bgQualityId === item.id }"
                  @click="setBgQuality(item.id)"
                >
                  {{ t(item.labelKey) }}
                </button>
              </div>
            </div>
          </div>
          <p class="hint">{{ t("settings.appearance.bgQualityHint") }}</p>
          <p class="hint">{{ t("settings.appearance.bgSceneHint") }}</p>
        </template>

        <p v-else class="hint">{{ t("settings.appearance.bgGradientHint") }}</p>
      </div>
      <div class="card glass" id="appearance-glass">
        <h3>{{ t("settings.appearance.glassCard") }}</h3>
        <div class="tune-row">
          <label>{{ t("settings.appearance.glassStrength") }}</label>
          <NSlider
            v-model:value="settings.settings.glass_blur"
            class="tune-slider"
            :min="0"
            :max="30"
            :step="1"
            :tooltip="false"
          />
          <span class="tune-val">{{ settings.settings.glass_blur }} px</span>
        </div>
        <p class="hint">{{ t("settings.appearance.glassHint") }}</p>
      </div>
    </div>
  </div>
</template>
