<script setup lang="ts">
import { computed, nextTick, onMounted, reactive, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { NButton, NDrawer, NDrawerContent, NSelect, useMessage, type SelectOption } from "naive-ui";
import { api } from "../api";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useSettingsStore } from "../stores/settings";
import InstallDialog from "../components/InstallDialog.vue";
import ProjectCard from "../components/ProjectCard.vue";
import SimplePagination from "../components/SimplePagination.vue";
import {
  IconAlignJustify,
  IconClose,
  IconGrid,
  IconLayers,
  IconList,
  IconNewspaper,
  IconSearch,
  IconSliders,
} from "../components/icons";
import { cnCfName, translateCategory } from "../utils/categories";
import { cacheGet, cacheSet, cacheGetPersistent, cacheSetPersistent } from "../utils/cache";
import { fmtDateShort as fmtDate, instanceLabel } from "../utils/format";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import { useNewsStore } from "../stores/news";
import type { Instance, ProjectDependency, ProjectHit } from "../types";

defineOptions({ name: "BrowseView" });

const message = useMessage();
const { t } = useI18n();
const settingsStore = useSettingsStore();
const route = useRoute();
const newsStore = useNewsStore();

// —— 新闻（已并入「发现」页：作为「数据包」右侧的第 6 个 tab，选中后替换下方内容） ——
const showNewsEntry = computed(() => settingsStore.settings?.show_news ?? true);
async function refreshNews() {
  try {
    await newsStore.load(true);
  } catch (e) {
    message.error(String(e));
  }
}
function newsDelay(i: number): string {
  const step = Math.min(0.045, 0.6 / Math.max(1, newsStore.news.length));
  return `${(0.06 + i * step).toFixed(3)}s`;
}
const provider = ref<"all" | "modrinth" | "curseforge">("all");
const query = ref(typeof route.query.q === "string" ? route.query.q : "");
const type = ref("mod");
// 同步「新闻标签激活」状态给标题栏（决定是否显示刷新按钮），并响应标题栏发来的刷新信号
watch(type, (v) => { newsStore.active = v === "news"; }, { immediate: true });
watch(() => newsStore.refreshSignal, () => { void refreshNews(); });
const category = ref("");
const page = ref(0);
const results = ref<ProjectHit[]>([]);
const cfError = ref("");
const cfCount = ref(0);
const total = ref(0);
const loading = ref(false);
const cfCategories = ref<{ id: number; name: string }[]>([]);
// 筛选状态（游戏版本 / 加载器 / 类别）+ 侧边抽屉
const gameVersion = ref("");
const loader = ref("");
const showFilter = ref(false);
const versionOptions = ref<{ label: string; value: string }[]>([]);
const loaderOptions = computed(() => [
  { label: t("browse.loader.all"), value: "" },
  { label: "Fabric", value: "fabric" },
  { label: "Forge", value: "forge" },
  { label: "NeoForge", value: "neoforge" },
  { label: "Quilt", value: "quilt" },
]);
// 来源 / 排序 / 每页数量 / 视图
const providerOptions = computed(() => [
  { label: t("browse.provider.all"), value: "all" },
  { label: "Modrinth", value: "modrinth" },
  { label: "CurseForge", value: "curseforge" },
]);
const sort = ref("downloads");
const pageSize = ref(20);
const view = ref<"grid" | "list" | "compact">("grid");
const sortOptions = computed(() => [
  { label: t("browse.sort.downloads"), value: "downloads" },
  { label: t("browse.sort.relevance"), value: "relevance" },
  { label: t("browse.sort.follows"), value: "follows" },
  { label: t("browse.sort.newest"), value: "newest" },
  { label: t("browse.sort.updated"), value: "updated" },
]);
const pageSizeOptions = computed(() => [
  { label: t("browse.pageSize.perPage", { count: 20 }), value: 20 },
  { label: t("browse.pageSize.perPage", { count: 40 }), value: 40 },
  { label: t("browse.pageSize.perPage", { count: 60 }), value: 60 },
]);

const types = computed(() => [
  { key: "mod", label: t("browse.type.mod") },
  { key: "modpack", label: t("browse.type.modpack") },
  { key: "resourcepack", label: t("browse.type.resourcepack") },
  { key: "shader", label: t("browse.type.shader") },
  { key: "datapack", label: t("browse.type.datapack") },
]);

const modrinthCategories: Record<string, string[]> = {
  mod: ["", "fabric", "forge", "quilt", "neoforge", "optimization", "library", "utility", "adventure", "magic", "tech", "decoration", "equipment", "food", "misc", "mobs", "storage", "worldgen"],
  modpack: ["", "fabric", "forge", "quilt", "neoforge", "adventure", "challenge", "combat", "hardcore", "magic", "mini-game", "multiplayer", "optimization", "pvp", "tech", "vanilla-plus"],
  resourcepack: ["", "16x", "32x", "64x", "128x", "256x", "512x", "faithful", "cursed", "modern", "semi-realistic", "simplistic", "themed"],
  shader: ["", "potato", "low", "medium", "high", "ultra", "path-tracing", "complementary", "realistic"],
  datapack: ["", "adventure", "challenge", "decoration", "magic", "minigame", "mobs", "optimization", "technology", "utility", "worldgen"],
};

const catOptions = ref<{ label: string; value: string }[]>([]);
const installTarget = ref<ProjectHit | null>(null);
const showInstall = ref(false);

// 实例选择：非整合包类型下可选择实例，自动筛选游戏版本和加载器
const instances = ref<Instance[]>([]);
const selectedInstanceId = ref<string | null>(null);
const instanceOptions = computed(() => [
  { label: t("browse.instance.none"), value: "" },
  ...instances.value.map((i) => ({
    label: instanceLabel(i),
    value: i.id,
  })),
]);
const showInstanceSelect = computed(() => type.value !== "modpack" && instances.value.length > 0);
const showLoaderFilter = computed(() => type.value === "mod" || type.value === "modpack");
const instanceSelectWidth = computed(() => {
  const inst = instances.value.find((i) => i.id === selectedInstanceId.value);
  if (!inst) return 140;
  const label = instanceLabel(inst);
  return Math.max(140, Math.min(label.length * 8 + 40, 300));
});
// 实例下拉：按名字 / MC 版本 / 加载器 任一匹配即可搜到对应实例，
// 避免只能靠滚动且无法用版本定位到实例的问题
function filterInstance(pattern: string, option: SelectOption) {
  const p = pattern.trim().toLowerCase();
  if (!p) return true;
  const inst = instances.value.find((i) => i.id === option.value);
  if (!inst) return String(option.label).toLowerCase().includes(p);
  return (
    String(inst.name).toLowerCase().includes(p) ||
    String(inst.mc_version).toLowerCase().includes(p) ||
    String(inst.loader).toLowerCase().includes(p)
  );
}
// 是否为可用的标准 MC 版本号（如 1.20.1）。导入实例若无法识别出真实版本，
// 其 mc_version 可能被落成实例名（如 "mod开发"），这种非法值不能注入版本筛选，
// 否则上游按不存在的版本搜索会什么都搜不到。
function isValidMcVersion(v: string): boolean {
  return /^\d+\.\d+/.test((v || "").trim());
}
// 版本筛选下拉：仅当选中实例的版本是合法 MC 版本号时，才把它并入版本下拉，
// 避免导入实例用了非"最新 40 个正式版"内的版本时版本筛选看不到/显示错乱
const displayVersionOptions = computed(() => {
  const inst = instances.value.find((i) => i.id === selectedInstanceId.value);
  if (!inst || !isValidMcVersion(inst.mc_version)) return versionOptions.value;
  const exists = versionOptions.value.some((o) => o.value === inst.mc_version);
  if (exists) return versionOptions.value;
  return [{ label: inst.mc_version, value: inst.mc_version }, ...versionOptions.value];
});

let searchSeq = 0;
async function search() {
  // 新闻 tab 不走内容搜索
  if (type.value === "news") return;
  const seq = ++searchSeq;
  // 缓存：内存（5 分钟，会话内）+ localStorage 持久化（6 小时，跨会话）。
  // 应用重启后内存清空，但 localStorage 仍在，打开发现页命中持久化缓存不调 API。
  const cacheKey = `browse:${provider.value}|${query.value}|${type.value}|${category.value}|${page.value}|${gameVersion.value}|${loader.value}|${sort.value}|${pageSize.value}`;
  const cached = cacheGet<{ hits: ProjectHit[]; total: number; cf_error?: string | null; cf_count?: number }>(cacheKey)
    ?? cacheGetPersistent<{ hits: ProjectHit[]; total: number; cf_error?: string | null; cf_count?: number }>(cacheKey);
  if (cached) {
    if (seq !== searchSeq) return;
    results.value = cached.hits;
    if (page.value === 0) total.value = cached.total;
    cfError.value = cached.cf_error ?? "";
    cfCount.value = cached.cf_count ?? 0;
    loading.value = false;
    return;
  }
  loading.value = true;
  try {
    const res = await api.browse(
      provider.value,
      query.value,
      type.value,
      category.value,
      page.value,
      gameVersion.value,
      loader.value,
      sort.value,
      pageSize.value
    );
    if (seq !== searchSeq) return;
    results.value = res.hits;
    if (page.value === 0) total.value = res.total;
    cfError.value = res.cf_error ?? "";
    cfCount.value = res.cf_count ?? 0;
    cacheSet(cacheKey, res, 5 * 60 * 1000);
    cacheSetPersistent(cacheKey, res, 6 * 60 * 60 * 1000);
  } catch (e) {
    if (seq !== searchSeq) return;
    message.error(String(e));
    results.value = [];
    cfError.value = "";
  } finally {
    if (seq === searchSeq) loading.value = false;
  }
}

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / pageSize.value)));
function onPage(p: number) {
  page.value = p - 1;
  search();
}

// 结果卡片逐个进场（学习首页 pin-card 的做法）：延迟由内联 style 下发给每张卡片。
// 数量多时自动收紧步长，保证整批级联在 ~0.7s 内跑完；loading 隐藏/显示、
// 换页/换筛选导致 key 更换时都会重新触发一次级联。
function cardDelay(i: number): string {
  const step = Math.min(0.045, 0.6 / Math.max(1, results.value.length));
  return `${(0.06 + i * step).toFixed(3)}s`;
}

const hasFilter = computed(() => !!category.value || !!gameVersion.value || !!loader.value);

function resetFilters() {
  category.value = "";
  gameVersion.value = "";
  loader.value = "";
  page.value = 0;
  search();
}

function catLabel(v: string) {
  return catOptions.value.find((o) => o.value === v)?.label ?? v;
}
function loaderLabel(v: string) {
  return loaderOptions.value.find((o) => o.value === v)?.label ?? v;
}

async function loadCfCategories() {
  if (provider.value !== "curseforge") return;
  const cacheKey = `cf-cats:${type.value}`;
  const cached = cacheGet<{ id: number; name: string }[]>(cacheKey)
    ?? cacheGetPersistent<{ id: number; name: string }[]>(cacheKey);
  if (cached) {
    cfCategories.value = cached;
    return;
  }
  try {
    cfCategories.value = (await api.curseforgeCategories(type.value)).categories;
    cacheSet(cacheKey, cfCategories.value, 10 * 60 * 1000);
    cacheSetPersistent(cacheKey, cfCategories.value, 24 * 60 * 60 * 1000);
  } catch {
    cfCategories.value = [];
  }
}

async function loadVersions() {
  const cacheKey = "versions:release";
  const cached = cacheGet<{ label: string; value: string }[]>(cacheKey)
    ?? cacheGetPersistent<{ label: string; value: string }[]>(cacheKey);
  if (cached) {
    versionOptions.value = cached;
    return;
  }
  try {
    const res = await api.getVersionManifest();
    const ids = res.versions
      .filter((v) => v.type === "release")
      .map((v) => v.id)
      .slice(0, 40);
    versionOptions.value = [
      { label: t("browse.version.all"), value: "" },
      ...ids.map((id) => ({ label: id, value: id })),
    ];
    cacheSet(cacheKey, versionOptions.value, 10 * 60 * 1000);
    cacheSetPersistent(cacheKey, versionOptions.value, 24 * 60 * 60 * 1000);
  } catch {
    versionOptions.value = [{ label: t("browse.version.all"), value: "" }];
  }
}

function rebuildOptions() {
  if (provider.value === "all" || provider.value === "modrinth") {
    catOptions.value = (modrinthCategories[type.value] ?? []).map((c) => ({
      label: c ? translateCategory(c) : t("browse.category.all"),
      value: c,
    }));
  } else {
    catOptions.value = [
      { label: t("browse.category.all"), value: "" },
      ...cfCategories.value.map((c) => ({ label: cnCfName(c.name), value: String(c.id) })),
    ];
  }
}

let debounce: ReturnType<typeof setTimeout> | null = null;
watch(query, () => {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(() => {
    page.value = 0;
    search();
  }, 450);
});

// 按分类保留 UI 状态：切回已访问过的分类直接恢复，不重新加载
interface TypeTab {
  category: string;
  page: number;
  loader: string;
  results: ProjectHit[];
  total: number;
  cfError: string;
  cfCount: number;
  catOptions: { label: string; value: string }[];
}
const typeTabs = reactive<Record<string, TypeTab>>({});
function saveTypeTab() {
  // 新闻 tab 无内容筛选状态，不保存
  if (type.value === "news") return;
  typeTabs[type.value] = {
    category: category.value,
    page: page.value,
    loader: loader.value,
    results: results.value,
    total: total.value,
    cfError: cfError.value,
    cfCount: cfCount.value,
    catOptions: catOptions.value,
  };
}
function restoreTypeTab(key: string): boolean {
  const tab = typeTabs[key];
  if (!tab) return false;
  category.value = tab.category;
  page.value = tab.page;
  loader.value = tab.loader;
  results.value = tab.results;
  total.value = tab.total;
  cfError.value = tab.cfError;
  cfCount.value = tab.cfCount;
  catOptions.value = tab.catOptions;
  return true;
}
function switchType(key: string) {
  if (key === type.value) return;
  saveTypeTab();
  type.value = key;
  // 切到新闻 tab 时拉取新闻（已有缓存则秒回）
  if (key === "news") void newsStore.load().catch((e) => message.error(String(e)));
}

watch([provider, type], async (n, o) => {
  if (n[0] !== o?.[0]) {
    for (const k in typeTabs) delete typeTabs[k];
  }
  // 新闻 tab 不触发内容搜索/分类重建
  if (type.value === "news") return;
  if (restoreTypeTab(type.value)) return;
  category.value = "";
  if (!showLoaderFilter.value) loader.value = "";
  page.value = 0;
  if (provider.value === "curseforge") await loadCfCategories();
  rebuildOptions();
  search();
});

watch(category, () => {
  page.value = 0;
  search();
});

watch([gameVersion, loader], () => {
  // 如果手动改了版本/加载器，且不再匹配所选实例，则切换为"不关联实例"
  const inst = instances.value.find((i) => i.id === selectedInstanceId.value);
  if (inst) {
    const versionMismatch = isValidMcVersion(inst.mc_version) && gameVersion.value !== inst.mc_version;
    const loaderMismatch = showLoaderFilter.value && loader.value !== (inst.loader === "vanilla" ? "" : inst.loader);
    if (versionMismatch || loaderMismatch) {
      suppressInstanceClear = true;
      selectedInstanceId.value = "";
    }
  }
  page.value = 0;
  search();
});

let suppressInstanceClear = false;
watch(selectedInstanceId, () => {
  if (suppressInstanceClear) {
    suppressInstanceClear = false;
    return;
  }
  const inst = instances.value.find((i) => i.id === selectedInstanceId.value);
  if (inst) {
    // 仅当识别出合法 MC 版本时才自动填充版本筛选；
    // 识别不出（mc_version 被落成实例名）时不注入非法版本，让用户手动选版本
    gameVersion.value = isValidMcVersion(inst.mc_version) ? inst.mc_version : "";
    if (showLoaderFilter.value) {
      loader.value = inst.loader === "vanilla" ? "" : inst.loader;
    }
  } else {
    gameVersion.value = "";
    loader.value = "";
  }
});

watch([sort, pageSize], () => {
  page.value = 0;
  search();
});

function syncProviderFromRoute() {
  const p = route.query.provider;
  if (p === "modrinth" || p === "curseforge" || p === "all") {
    provider.value = p;
  }
}

// 支持从实例模组列表的"在内容中心搜索"跳转：带上 q 与 provider，直接定位平台
watch(route, () => {
  if (route.name !== "browse") return;
  query.value = typeof route.query.q === "string" ? route.query.q : "";
  syncProviderFromRoute();
  search();
});

// ---- 描述翻译（自建服务，卡片选择式）----
const translateMode = ref(false);
const translatingSlugs = ref<string[]>([]);
const translatedDescs = ref<Record<string, string>>({});

function toggleTranslateMode() {
  translateMode.value = !translateMode.value;
  if (translateMode.value) message.info(t("browse.translate.modeHint"));
}

async function translateCard(p: ProjectHit) {
  const service = settingsStore.settings?.translate_provider ?? "default";
  // 百度网页模式：只用浏览器打开翻译页（带上英文描述），结果由用户自行查看，
  // 不做任何自动抓取。两个平台的内容都适用。
  if (service === "baidu_web") {
    const q = encodeURIComponent(p.description || p.title);
    try {
      await openUrl(`https://fanyi.baidu.com/mtpe-individual/transText?query=${q}&lang=en2zh`);
    } catch (e) {
      message.error(t("browse.translate.openBrowserFailed", { error: String(e) }));
    }
    return;
  }
  // 已翻译过的卡片再点一次：切回英文原文（再点可切回中文）
  if (translatedDescs.value[p.id] !== undefined) {
    const next = { ...translatedDescs.value };
    delete next[p.id];
    translatedDescs.value = next;
    return;
  }
  if (translatingSlugs.value.includes(p.id)) return;
  translatingSlugs.value = [...translatingSlugs.value, p.id];
  try {
    const r = await api.translateDescriptions(p.provider, [p.id]);
    const text = r.translations[p.id];
    if (text) {
      translatedDescs.value = { ...translatedDescs.value, [p.id]: text };
    } else if (r.rateLimited) {
      message.warning(t("browse.translate.busy"));
    } else {
      message.info(t("browse.translate.noTranslation"));
    }
  } catch (e) {
    message.error(String(e));
  } finally {
    translatingSlugs.value = translatingSlugs.value.filter((s) => s !== p.id);
  }
}

function openInstall(p: ProjectHit) {
  installTarget.value = p;
  showInstall.value = true;
}

async function openInstallDep(dep: ProjectDependency) {
  const provider = installTarget.value?.provider ?? "modrinth";
  try {
    const info = await api.projectInfo(provider, dep.projectId);
    installTarget.value = info;
  } catch {
    installTarget.value = {
      provider,
      id: dep.projectId,
      slug: dep.slug,
      title: dep.title,
      description: "",
      author: "",
      downloads: 0,
      follows: 0,
      icon_url: "",
      project_type: "mod",
      categories: [],
      latest_version: "",
      game_versions: [],
      updated: "",
      featured_image: "",
    };
  }
  showInstall.value = true;
}

// 类型卡片的滑动高亮指示器（先扩展包裹再收缩）
const typeBox = ref<HTMLElement | null>(null);
const { indicatorStyle: typeIndicatorStyle, refresh: refreshTypeIndicator } = useSlidingIndicator(
  typeBox,
  () => Array.from(typeBox.value?.querySelectorAll<HTMLElement>(".type-card button") ?? []),
  () => (type.value === "news" ? types.value.length : types.value.findIndex((tp) => tp.key === type.value)),
);
watch(type, () => nextTick(() => refreshTypeIndicator()));

// 视图切换的滑动高亮指示器（网格/列表/紧凑）
const viewBox = ref<HTMLElement | null>(null);
const { indicatorStyle: viewIndicatorStyle, refresh: refreshViewIndicator } = useSlidingIndicator(
  viewBox,
  () => Array.from(viewBox.value?.querySelectorAll<HTMLElement>(".view-switch button") ?? []),
  () => ["grid", "list", "compact"].indexOf(view.value),
  { axis: "horizontal" }
);
watch(view, () => nextTick(() => refreshViewIndicator()));

onMounted(async () => {
  try {
    instances.value = await api.listInstances();
  } catch {
    instances.value = [];
  }
  // 默认选择最近游玩的实例（排除原版，原版无法装 mod）
  if (type.value !== "modpack") {
    const sorted = [...instances.value]
      .filter((i) => i.loader !== "vanilla")
      .sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0));
    if (sorted.length > 0) selectedInstanceId.value = sorted[0].id;
  }
  rebuildOptions();
  loadVersions();
  syncProviderFromRoute();
  await search();
});
</script>

<template>
  <div id="browse-root" class="browse" data-tauri-drag-region>
    <div ref="typeBox" class="type-card glass">
      <div class="indicator" :style="typeIndicatorStyle"></div>
      <button
        v-for="tp in types"
        :key="tp.key"
        :class="{ active: type === tp.key }"
        @click="switchType(tp.key)"
      >
        {{ tp.label }}
      </button>
      <template v-if="showNewsEntry">
        <span class="type-divider"></span>
        <button class="news-tab" :class="{ active: type === 'news' }" @click="switchType('news')">
          <IconNewspaper class="news-tab-icon" />
          <span>{{ t("nav.news") }}</span>
        </button>
      </template>
    </div>

    <template v-if="type !== 'news'">
    <div class="toolbar glass">
      <div class="toolbar-row">
        <div id="browse-search" class="search-box">
          <IconSearch />
          <input v-model="query" :placeholder="t('browse.search.placeholder')" />
        </div>
        <n-select
          v-if="showInstanceSelect"
          v-model:value="selectedInstanceId"
          :options="instanceOptions"
          size="small"
          class="tb-select instance-select"
          :style="{ width: instanceSelectWidth + 'px' }"
          filterable
          :filter="filterInstance"
          :placeholder="t('browse.instance.searchPlaceholder')"
        />
      </div>
      <div class="toolbar-row">
        <n-select
          v-model:value="sort"
          :options="sortOptions"
          size="small"
          class="tb-select"
        />
        <n-select
          v-model:value="pageSize"
          :options="pageSizeOptions"
          size="small"
          class="tb-select page-size"
        />
        <div ref="viewBox" class="view-switch">
          <div class="indicator" :style="viewIndicatorStyle"></div>
          <button :class="{ active: view === 'grid' }" :title="t('browse.view.grid')" @click="view = 'grid'"><IconGrid /></button>
          <button :class="{ active: view === 'list' }" :title="t('browse.view.list')" @click="view = 'list'"><IconList /></button>
          <button :class="{ active: view === 'compact' }" :title="t('browse.view.compact')" @click="view = 'compact'"><IconAlignJustify /></button>
        </div>
        <button class="filter-btn" :class="{ on: hasFilter }" @click="showFilter = true">
          <IconSliders /> {{ t("browse.filter.button") }}
        </button>
        <n-select
          v-model:value="provider"
          :options="providerOptions"
          size="small"
          class="tb-select provider"
        />
        <button
          class="filter-btn translate-toggle"
          :class="{ on: translateMode }"
          @click="toggleTranslateMode"
        >
          <IconLayers /> {{ translateMode ? t("browse.translate.done") : t("browse.translate.toggle") }}
        </button>
      </div>
    </div>

    <div v-if="hasFilter" class="filter-tags glass">
      <span v-if="gameVersion" class="ftag">
        {{ t("browse.filter.versionTag", { value: gameVersion }) }}
        <button class="ftag-x" :title="t('browse.filter.remove')" @click="gameVersion = ''"><IconClose /></button>
      </span>
      <span v-if="loader" class="ftag">
        {{ t("browse.filter.loaderTag", { value: loaderLabel(loader) }) }}
        <button class="ftag-x" :title="t('browse.filter.remove')" @click="loader = ''"><IconClose /></button>
      </span>
      <span v-if="category" class="ftag">
        {{ t("browse.filter.categoryTag", { value: catLabel(category) }) }}
        <button class="ftag-x" :title="t('browse.filter.remove')" @click="category = ''"><IconClose /></button>
      </span>
      <button class="ftag ftag-clear" @click="resetFilters">{{ t("browse.filter.clearAll") }}</button>
    </div>

    <div v-if="provider === 'curseforge' && !cfCategories.length && !loading" class="cf-hint glass">
      {{ t("browse.cf.needApiKeyPre") }}
      <a href="https://console.curseforge.com" target="_blank">console.curseforge.com</a>
      {{ t("browse.cf.needApiKeyMid") }} <router-link to="/settings">{{ t("browse.cf.settings") }}</router-link> {{ t("browse.cf.needApiKeyPost") }}
    </div>

    <div v-if="provider === 'all' && cfError && !loading" class="cf-hint glass">
      {{ t("browse.cf.loadFailedPre", { error: cfError }) }} <router-link to="/settings">{{ t("browse.cf.settings") }}</router-link> {{ t("browse.cf.loadFailedPost") }}
    </div>

    <div v-show="loading" class="grid skel-grid">
      <div v-for="i in 8" :key="i" class="skel-card glass">
        <div class="skel-main">
          <div class="skel-icon"></div>
          <div class="skel-body">
            <div class="skel-line w-60"></div>
            <div class="skel-line w-32"></div>
            <div class="skel-line w-92"></div>
            <div class="skel-line w-70"></div>
            <div class="skel-cats">
              <div class="skel-tag"></div>
              <div class="skel-tag"></div>
            </div>
          </div>
        </div>
        <div class="skel-foot">
          <div class="skel-line w-40"></div>
          <div class="skel-btn"></div>
        </div>
      </div>
    </div>
    <div v-show="!loading && !results.length" class="center">
      <div class="center-icon"><IconSearch /></div>
      <p>{{ t("browse.search.noResult") }}</p>
    </div>
    <div v-show="!loading && results.length" class="grid" :class="`view-${view}`">
      <ProjectCard
        v-for="(p, i) in results"
        :key="p.provider + p.id"
        :project="p"
        :view="view"
        :translate-mode="translateMode"
        :translating="translatingSlugs.includes(p.id)"
        :translated-desc="translatedDescs[p.id] ?? null"
        :style="{ animationDelay: cardDelay(i) }"
        @install="openInstall"
        @translate="translateCard"
      />
    </div>

    <div v-if="total > 20" class="pager">
      <span class="pager-total">{{ t("browse.pager.total", { count: total }) }}</span>
      <SimplePagination
        :page="page + 1"
        :page-count="pageCount"
        @update:page="onPage"
      />
    </div>
    </template>

    <div v-else class="news-content">
      <div v-if="newsStore.loading && !newsStore.news.length" class="news-state">{{ t("common.loading") }}</div>
      <div v-else-if="!newsStore.news.length" class="news-state">
        <div class="news-state-icon"><IconNewspaper /></div>
        <p>{{ t("news.empty") }}</p>
      </div>
      <div v-else class="news-list">
        <div
          v-for="(n, i) in newsStore.news"
          :key="n.url"
          class="news-card"
          :style="{ animationDelay: newsDelay(i) }"
          @click="n.url && openUrl(n.url).catch(() => {})"
        >
          <div v-if="n.image" class="news-image">
            <img :src="n.image" :alt="n.image_alt" loading="lazy" />
          </div>
          <div class="news-body">
            <h3 class="news-title">{{ n.title }}</h3>
            <p v-if="n.description" class="news-desc">{{ n.description }}</p>
            <div class="news-meta">
              <span v-if="n.author" class="meta-author">{{ n.author }}</span>
              <span class="meta-date">{{ fmtDate(n.time) }}</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <n-drawer v-model:show="showFilter" :width="330" placement="right">
      <n-drawer-content :title="t('browse.filter.title')" closable>
        <div class="filter-group">
          <label>{{ t("browse.filter.gameVersion") }}</label>
          <n-select v-model:value="gameVersion" :options="displayVersionOptions" size="small" />
        </div>
        <div v-if="showLoaderFilter" class="filter-group">
          <label>{{ t("browse.filter.loader") }}</label>
          <div class="filter-chips">
            <button
              v-for="opt in loaderOptions"
              :key="opt.value"
              class="filter-chip"
              :class="{ active: loader === opt.value }"
              @click="loader = opt.value"
            >{{ opt.label }}</button>
          </div>
        </div>
        <div class="filter-group">
          <label>{{ t("browse.filter.category") }}</label>
          <div class="filter-chips">
            <button
              v-for="opt in catOptions"
              :key="opt.value"
              class="filter-chip"
              :class="{ active: category === opt.value }"
              @click="category = opt.value"
            >{{ opt.label }}</button>
          </div>
          <p v-if="provider === 'all'" class="filter-hint">{{ t("browse.filter.hint") }}</p>
        </div>
        <div class="filter-actions">
          <n-button size="small" @click="resetFilters">{{ t("browse.filter.reset") }}</n-button>
          <n-button size="small" type="primary" @click="showFilter = false">{{ t("browse.filter.done") }}</n-button>
        </div>
      </n-drawer-content>
    </n-drawer>

    <InstallDialog v-model:show="showInstall" :project="installTarget" :default-instance="selectedInstanceId" @install-dep="openInstallDep" />
  </div>
</template>

<style scoped>
.browse {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.toolbar {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
}
.toolbar-row {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}
.toolbar-row .tb-select.provider {
  margin-left: auto;
}
.tb-select.provider {
  width: 120px;
}
.search-box {
  flex: 1;
  min-width: 220px;
  display: flex;
  align-items: center;
  gap: 8px;
  background: var(--w-06);
  border: 1px solid var(--border);
  border-radius: 9px;
  padding: 0 12px;
  color: var(--text-3);
}
.search-box input {
  flex: 1;
  background: none;
  border: none;
  outline: none;
  color: var(--text-1);
  padding: 8px 0;
  font-size: 13px;
  font-family: inherit;
}
.type-card {
  position: relative;
  display: inline-flex;
  gap: 4px;
  padding: 5px;
  align-self: flex-start;
}
.type-card .indicator {
  position: absolute;
  top: 5px;
  bottom: 5px;
  border-radius: 9px;
  background: var(--accent-soft);
  pointer-events: none;
}
.type-card button {
  position: relative;
  display: inline-flex;
  align-items: center;
  border: none;
  background: transparent;
  color: var(--text-2);
  padding: 8px 18px;
  border-radius: 9px;
  font-size: 14px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
}
.type-card button.active {
  color: var(--accent);
}
.filter-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border);
  background: var(--w-04);
  color: var(--text-2);
  border-radius: 9px;
  padding: 7px 13px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.12s;
}
.filter-btn:hover {
  background: var(--w-08);
}
.filter-btn.on {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-soft);
}
.filter-group {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 18px;
}
.filter-group label {
  font-size: 12px;
  color: var(--text-2);
  font-weight: 600;
}
.filter-hint {
  font-size: 11px;
  color: var(--text-3);
  margin: 4px 0 0;
}
.filter-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.filter-chip {
  border: 1px solid var(--border);
  background: var(--w-04);
  color: var(--text-2);
  border-radius: 7px;
  padding: 5px 11px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.12s;
}
.filter-chip:hover {
  background: var(--w-08);
  color: var(--text-1);
}
.filter-chip.active {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: var(--accent);
}
.filter-actions {
  display: flex;
  gap: 10px;
  margin-top: 8px;
}
.cf-hint {
  padding: 12px 16px;
  font-size: 13px;
  color: var(--text-2);
}
.cf-hint a {
  color: var(--accent);
  text-decoration: none;
}
.center {
  padding: 70px;
  text-align: center;
  color: var(--text-3);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
}
.center-icon {
  font-size: 36px;
  opacity: 0.5;
}
.center p {
  margin: 0;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(330px, 1fr));
  gap: 16px;
}
/* 骨架屏：搜索时用 shimmer 占位卡，替代纯文字"搜索中" */
.skel-card {
  --skel-c: rgba(255, 255, 255, 0.07);
  --skel-hi: rgba(255, 255, 255, 0.14);
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
:root.light .skel-card {
  --skel-c: rgba(0, 0, 0, 0.06);
  --skel-hi: rgba(0, 0, 0, 0.10);
}
.skel-icon,
.skel-line,
.skel-tag,
.skel-btn {
  background: linear-gradient(90deg, var(--skel-c), var(--skel-hi), var(--skel-c));
  background-size: 200% 100%;
  animation: skel-shimmer 1.3s ease-in-out infinite;
}
.skel-main { display: flex; gap: 12px; }
.skel-icon { width: 56px; height: 56px; border-radius: 12px; flex-shrink: 0; }
.skel-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
.skel-line { height: 11px; border-radius: 6px; }
.skel-line.w-32 { width: 32%; }
.skel-line.w-40 { width: 40%; }
.skel-line.w-60 { width: 60%; }
.skel-line.w-70 { width: 70%; }
.skel-line.w-92 { width: 92%; }
.skel-cats { display: flex; gap: 6px; margin-top: 2px; }
.skel-tag { width: 52px; height: 18px; border-radius: 9px; }
.skel-foot { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.skel-btn { width: 84px; height: 28px; border-radius: 8px; }
@keyframes skel-shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}
@media (prefers-reduced-motion: reduce) {
  .skel-icon, .skel-line, .skel-tag, .skel-btn { animation: none; }
}
/* 卡片逐个进场（同首页 pin-card：translateY 28px 上浮 + 淡入，backwards 让
   未出场的卡片先保持隐藏）。延迟由模板内联 animationDelay 提供。 */
.grid :deep(.p-card) {
  animation: browse-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
@keyframes browse-card-enter {
  from {
    transform: translateY(28px);
    opacity: 0;
  }
  to {
    transform: none;
    opacity: 1;
  }
}
.grid.view-list,
.grid.view-compact {
  grid-template-columns: 1fr;
}
.grid.view-compact {
  gap: 8px;
}
.tb-select {
  width: 110px;
}
.tb-select.page-size {
  width: 130px;
}
.tb-select.instance-select {
  flex-shrink: 0;
}
.tb-select.instance-select :deep(.n-base-selection) {
  --n-height: 35px !important;
}
.view-switch {
  position: relative;
  display: flex;
  background: var(--w-05);
  border-radius: 9px;
  padding: 3px;
  gap: 2px;
}
.view-switch .indicator {
  position: absolute;
  top: 3px;
  bottom: 3px;
  border-radius: 7px;
  background: var(--accent-soft);
  pointer-events: none;
}
.view-switch button {
  border: none;
  background: transparent;
  color: var(--text-3);
  width: 30px;
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 7px;
  cursor: pointer;
  font-size: 14px;
}
.view-switch button:hover {
  color: var(--text-1);
}
.view-switch button.active {
  color: var(--accent);
}
.filter-tags {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  padding: 8px 14px;
}
.ftag {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  background: var(--accent-soft);
  color: var(--accent);
  border: 1px solid var(--accent-03);
  border-radius: 8px;
  padding: 3px 10px;
}
.ftag-x {
  border: none;
  background: transparent;
  color: inherit;
  cursor: pointer;
  display: inline-flex;
  padding: 0;
  font-size: 12px;
  opacity: 0.7;
}
.ftag-x:hover {
  opacity: 1;
}
.ftag-clear {
  background: transparent;
  color: var(--text-3);
  border-color: var(--border);
  cursor: pointer;
  font-family: inherit;
}
.ftag-clear:hover {
  color: var(--text-1);
}
.pager {
  display: flex;
  justify-content: center;
  align-items: center;
  gap: 16px;
  color: var(--text-3);
  font-size: 13px;
}
.pager-total {
  color: var(--text-3);
}

/* —— 新闻 tab（数据包右侧，分割线隔开） —— */
.type-divider {
  width: 1px;
  align-self: stretch;
  margin: 7px 6px;
  background: var(--border);
  flex-shrink: 0;
}
.news-tab {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.news-tab-icon {
  font-size: 15px;
}
/* —— 新闻内容区（替换下方浏览结果） —— */
.news-content {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.news-state {
  padding: 48px 24px;
  text-align: center;
  color: var(--text-3);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
}
.news-state-icon {
  font-size: 36px;
  opacity: 0.5;
}
.news-state p {
  margin: 0;
}
.news-list {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.news-card {
  display: flex;
  border: 1px solid var(--border);
  border-radius: 12px;
  overflow: hidden;
  cursor: pointer;
  background: var(--panel);
  backdrop-filter: blur(var(--glass-blur, 8px));
  -webkit-backdrop-filter: blur(var(--glass-blur, 8px));
  transition: transform 0.15s ease, box-shadow 0.15s ease;
  animation: news-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
@keyframes news-card-enter {
  from { transform: translateY(20px); opacity: 0; }
  to { transform: none; opacity: 1; }
}
.news-card:hover {
  box-shadow: 0 4px 20px var(--k-15);
}
.news-image {
  flex-shrink: 0;
  width: 130px;
  height: 100px;
  overflow: hidden;
  background: var(--w-03);
}
.news-image img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  transition: transform 0.32s ease;
}
.news-card:hover .news-image img {
  transform: scale(1.06);
}
.news-body {
  flex: 1;
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 5px;
  min-width: 0;
}
.news-title {
  font-size: 14px;
  font-weight: 700;
  margin: 0;
  color: var(--text-1);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.news-desc {
  font-size: 12px;
  color: var(--text-3);
  margin: 0;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  line-height: 1.5;
}
.news-meta {
  display: flex;
  gap: 12px;
  font-size: 11px;
  color: var(--text-3);
  margin-top: auto;
}
.meta-author {
  color: var(--accent);
  font-weight: 600;
}
</style>
