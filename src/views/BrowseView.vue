<script setup lang="ts">
import { staggeredCardDelay } from "../composables/staggeredCardDelay";
import UiStatus from "../components/UiStatus.vue";
import { vSegmented } from "../composables/vSegmented";
import {
  computed,
  defineAsyncComponent,
  nextTick,
  onActivated,
  onBeforeUnmount,
  onDeactivated,
  onMounted,
  reactive,
  ref,
  watch,
} from "vue";
import { useRoute } from "vue-router";
import { useI18n } from "vue-i18n";
import { NInput, NButton, NDrawer, NDrawerContent, NSelect, useMessage, type SelectOption } from "naive-ui";
import { api } from "../api";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useSettingsStore } from "../stores/settings";
import InstallDialog from "../components/InstallDialog.vue";
import InstallChecklist from "../components/InstallChecklist.vue";
import ProjectCard from "../components/ProjectCard.vue";
import SimplePagination from "../components/SimplePagination.vue";
import {
  IconAlignJustify,
  IconArchive,
  IconBox,
  IconClose,
  IconDatabase,
  IconGrid,
  IconImage,
  IconLayers,
  IconList,
  IconNewspaper,
  IconSearch,
  IconSliders,
  IconStar,
  IconSun,
} from "../components/icons";
import { cnCfName, translateCategory } from "../utils/categories";
import {
  cacheGet,
  cacheSet,
  cacheGetPersistent,
  cacheSetPersistent,
  cacheDelete,
  cacheDeletePersistent,
} from "../utils/cache";
import { fmtDateShort as fmtDate, instanceLabel } from "../utils/format";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import { useNewsStore } from "../stores/news";
import { useFavoritesStore, favoriteToHit } from "../stores/favorites";
import type { Instance, ProjectDependency, ProjectHit } from "../types";

defineOptions({ name: "BrowseView" });

const message = useMessage();
const { t, locale } = useI18n();
const settingsStore = useSettingsStore();
const route = useRoute();
const newsStore = useNewsStore();
const favoritesStore = useFavoritesStore();

// —— 新闻（已并入「发现」页：作为「数据包」右侧的第 6 个 tab，选中后替换下方内容） ——
const showNewsEntry = computed(() => settingsStore.settings?.show_news ?? true);
const provider = ref<"all" | "modrinth" | "curseforge">("all");
const query = ref(typeof route.query.q === "string" ? route.query.q : "");
const type = ref("mod");
watch(type, (value) => newsStore.setActive(value === "news"), { immediate: true });
onBeforeUnmount(() => newsStore.setActive(false));
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
  // 图标按「一眼能区分」挑的：单个方块 / 收纳箱 / 图片 / 太阳 / 数据 / 层叠结构
  { key: "mod", label: t("browse.type.mod"), icon: IconBox },
  { key: "modpack", label: t("browse.type.modpack"), icon: IconArchive },
  { key: "resourcepack", label: t("browse.type.resourcepack"), icon: IconImage },
  { key: "shader", label: t("browse.type.shader"), icon: IconSun },
  { key: "datapack", label: t("browse.type.datapack"), icon: IconDatabase },
  { key: "shimu", label: t("browse.type.schematic"), icon: IconLayers },
]);

// 「投影」tab 直接复用拾木空间投影库整页（自带搜索 / 板块 / 账号 / 详情），
// 异步加载：不在发现页首屏就把投影库那套 DOM 解析逻辑一起打包进来。
const ShimuLibraryView = defineAsyncComponent(() => import("./ShimuLibraryView.vue"));

// —— 投影分类的工具栏状态（与内容搜索互不干扰） ——
const shimuKeyword = ref("");
/** 防抖后的关键词：投影库只在停止输入后才重新请求 */
const shimuKeywordApplied = ref("");
let shimuDebounce: ReturnType<typeof setTimeout> | null = null;
watch(shimuKeyword, (v) => {
  if (shimuDebounce) clearTimeout(shimuDebounce);
  shimuDebounce = setTimeout(() => { shimuKeywordApplied.value = v; }, 450);
});
const shimuSort = ref<"new" | "hot" | "featured">("hot");
const shimuSortOptions = computed(() => [
  { label: t("shimu.sortHot"), value: "hot" },
  { label: t("shimu.sortNew"), value: "new" },
  { label: t("shimu.sortFeatured"), value: "featured" },
]);

/**
 * 投影来源：与内容分类的「来源」同位置、同语义。
 *
 * `all` = 所有已接入来源合并；具体来源一律用**已接入的 key**，没接入的站点不摆出来
 * 当死选项。目前只接了拾木空间，所以清单里就它一项 —— 以后接入新投影站时，
 * 往 `schematicSourceOptions` 里加一项、在投影库里按 source 分流取数即可。
 */
type SchematicSourceKey = "all" | "shimu";
const schematicSource = ref<SchematicSourceKey>("all");
const schematicSourceOptions = computed(() => [
  { label: t("browse.provider.all"), value: "all" satisfies SchematicSourceKey },
  { label: t("shimu.title"), value: "shimu" satisfies SchematicSourceKey },
]);
/** 投影库进详情时收起工具栏，避免上下两套控件打架 */
const shimuDetailOpen = ref(false);


/** 搜索框：投影走站点关键词搜索，其余走内容搜索 */
const searchText = computed({
  get: () => (type.value === "shimu" ? shimuKeyword.value : query.value),
  set: (v: string) => {
    if (type.value === "shimu") shimuKeyword.value = v;
    else query.value = v;
  },
});
const searchPlaceholder = computed(() => {
  if (type.value === "shimu") return t("shimu.searchPlaceholder");
  if (type.value === "fav") return t("browse.fav.searchPlaceholder");
  return t("browse.search.placeholder");
});

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
const showChecklist = ref(false);
/** 当前选中的实例对象（供清单弹窗使用） */
const selectedInstance = computed(() =>
  instances.value.find((i) => i.id === selectedInstanceId.value) ?? null,
);

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
const searchError = ref("");

/** 发现页缓存载荷（cf_error 一并缓存的历史包袱见 search()） */
interface BrowseCachePayload {
  hits: ProjectHit[];
  total: number;
  cf_error?: string | null;
  cf_count?: number;
}

async function search() {
  // 新闻 / 投影 / 收藏夹 tab 不走内容搜索
  if (type.value === "news" || type.value === "shimu" || type.value === "fav") return;
  const seq = ++searchSeq;
  searchError.value = "";
  // 缓存：内存（5 分钟，会话内）+ localStorage 持久化（6 小时，跨会话）。
  // 应用重启后内存清空，但 localStorage 仍在，打开发现页命中持久化缓存不调 API。
  const cacheKey = `browse:${provider.value}|${query.value}|${type.value}|${category.value}|${page.value}|${gameVersion.value}|${loader.value}|${sort.value}|${pageSize.value}`;
  const cached = cacheGet<BrowseCachePayload>(cacheKey) ?? cacheGetPersistent<BrowseCachePayload>(cacheKey);
  // 带 cf_error 的缓存条目一律当未命中并删掉：
  // CurseForge 那次失败只是一瞬间的网络抖动，但旧实现把错误一起缓存了
  // （持久层 6 小时），命中它时根本不会重新发请求，于是表现为
  // 「只有这个筛选/这一页一直报加载失败，换个来源或翻页都正常」。
  // 这里是用户升级后立刻生效的那一步——历史脏缓存不用等 6 小时过期。
  if (cached?.cf_error) {
    cacheDelete(cacheKey);
    cacheDeletePersistent(cacheKey);
  } else if (cached) {
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
    // 失败（含"部分来源失败"）的结果不写缓存：错误是瞬时状态，
    // 一旦落盘就会把一次抖动冻结整个 TTL，且命中缓存期间不会再发请求。
    if (!res.cf_error) {
      cacheSet(cacheKey, res, 5 * 60 * 1000);
      cacheSetPersistent(cacheKey, res, 6 * 60 * 60 * 1000);
    }
  } catch (e) {
    if (seq !== searchSeq) return;
    searchError.value = String(e);
    cfError.value = "";
  } finally {
    if (seq === searchSeq) loading.value = false;
  }
}

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / pageSize.value)));

// —— 收藏夹 ——
// 纯本地数据：不过接口、不分页、不参与来源/筛选。搜索框和排序照旧可用，
// 只是作用在本地的 items 上。
const isFavTab = computed(() => type.value === "fav");

/** 发现页的几个内容分类，也是收藏夹里「资源类型」筛选的固定顺序 */
const RES_TYPE_KEYS = ["mod", "modpack", "resourcepack", "shader", "datapack"] as const;
type ResTypeKey = (typeof RES_TYPE_KEYS)[number];
const isResType = (v: string): v is ResTypeKey => (RES_TYPE_KEYS as readonly string[]).includes(v);

/** 资源类型的中文/英文名（未知类型原样显示，别把 Spigot 之类弄丢了） */
function resTypeLabel(raw: string): string {
  const k = (raw || "").toLowerCase();
  return isResType(k) ? t(`browse.type.${k}`) : raw || "";
}

const favTypeFilter = ref("");
/** 筛选下拉只列出收藏里真实存在的类型：点进去不会是空列表 */
const favTypeOptions = computed(() => {
  const present = new Set(
    favoritesStore.items.map((i) => (i.project_type || "").toLowerCase()).filter(Boolean),
  );
  return [
    { label: t("browse.fav.allTypes"), value: "" },
    ...RES_TYPE_KEYS.filter((k) => present.has(k)).map((k) => ({ label: t(`browse.type.${k}`), value: k })),
    ...[...present].filter((k) => !isResType(k)).map((k) => ({ label: k, value: k })),
  ];
});
// 该类型的收藏被删光后，筛选停在它上面会得到一个永远空着的列表，顺手退回「全部」
watch(favTypeOptions, (opts) => {
  if (favTypeFilter.value && !opts.some((o) => o.value === favTypeFilter.value)) favTypeFilter.value = "";
});

const timeOf = (s: string) => {
  const v = Date.parse(s || "");
  return Number.isFinite(v) ? v : 0;
};
/** 收藏夹列表：先按资源类型 + 搜索词本地过滤，再按当前排序本地排 */
const favHits = computed<ProjectHit[]>(() => {
  const q = query.value.trim().toLowerCase();
  const list = favoritesStore.items.filter((it) => {
    if (favTypeFilter.value && (it.project_type || "").toLowerCase() !== favTypeFilter.value) return false;
    return (
      !q ||
      it.title.toLowerCase().includes(q) ||
      it.description.toLowerCase().includes(q) ||
      it.author.toLowerCase().includes(q)
    );
  });
  const byAdded = (a: { addedAt: number }, b: { addedAt: number }) => b.addedAt - a.addedAt;
  // 「最新发布」没有发布时间字段可依，落在「最近收藏」上；「相关度」同理
  const sorters: Record<string, (a: (typeof list)[number], b: (typeof list)[number]) => number> = {
    downloads: (a, b) => b.downloads - a.downloads,
    follows: (a, b) => b.follows - a.follows,
    updated: (a, b) => timeOf(b.updated) - timeOf(a.updated) || byAdded(a, b),
    newest: byAdded,
    relevance: byAdded,
  };
  return [...list].sort(sorters[sort.value] ?? byAdded).map(favoriteToHit);
});

/** 列表/空态统一走这个入口，收藏夹和普通搜索结果共用同一套渲染 */
const visibleResults = computed(() => (isFavTab.value ? favHits.value : results.value));
/** 空态文案：收藏夹要区分「一个都没有」和「被筛选/搜索滤空了」 */
const emptyText = computed(() => {
  if (!isFavTab.value) return t("browse.search.noResult");
  return favoritesStore.count ? t("browse.fav.noMatch") : t("browse.fav.empty");
});

function toggleFavorite(p: ProjectHit) {
  try {
    const on = favoritesStore.toggle(p);
    message.success(on ? t("projectCard.addedToFavorites") : t("projectCard.removedFromFavorites"));
  } catch (error) {
    message.error(String(error));
  }
}
function onPage(p: number) {
  page.value = p - 1;
  search();
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
  // 新闻 / 投影 tab 无内容筛选状态，不保存（投影库自己维护分页与筛选）
  if (type.value === "news" || type.value === "shimu") return;
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
  shimuDetailOpen.value = false;
  // 切到新闻 tab 时拉取新闻（已有缓存则秒回）
  if (key === "news") void newsStore.load().catch((e) => message.error(String(e)));
  // 收藏夹里可能混着各分类的项目，进 tab 时按当前实例重算一次"已安装"标记
  if (key === "fav") void refreshInstalled();
}

watch([provider, type], async (n, o) => {
  if (n[0] !== o?.[0]) {
    for (const k in typeTabs) delete typeTabs[k];
  }
  // 新闻 / 投影 / 收藏夹 tab 不触发内容搜索/分类重建
  if (type.value === "news" || type.value === "shimu" || type.value === "fav") return;
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
  refreshInstalled();
});

// 已安装 mod 的归一化 id 集合（按所选实例一次性拉取，供卡片批量判断）
const installedIds = ref<Set<string>>(new Set());
const normId = (s: string) => s.toLowerCase().replace(/[-_ ]/g, "");
async function refreshInstalled() {
  installedIds.value = new Set();
  const id = selectedInstanceId.value;
  if (!id || type.value === "modpack") return;
  try {
    installedIds.value = new Set(await api.listInstalledModIds(id));
  } catch { /* 忽略 */ }
}
function isProjectInstalled(p: ProjectHit) {
  if (!installedIds.value.size) return false;
  return installedIds.value.has(normId(p.slug)) || installedIds.value.has(normId(p.title));
}
watch(results, () => refreshInstalled());

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

/** 支持从工具页等入口直接定位分类：/browse?type=shimu */
function syncTypeFromRoute() {
  const tp = route.query.type;
  if (typeof tp !== "string" || !tp) return;
  // 只认标签栏里真实存在的 key，避免乱写的 query 把页面切到空白
  if (tp !== "news" && tp !== "fav" && !types.value.some((x) => x.key === tp)) return;
  if (type.value !== tp) switchType(tp);
}

// 支持从实例模组列表的"在内容中心搜索"跳转：带上 q 与 provider，直接定位平台
watch(route, () => {
  if (route.name !== "browse") return;
  syncTypeFromRoute();
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
  // 已选非 vanilla 实例 → 直接弹安装清单；否则走原安装弹窗
  const inst = selectedInstance.value;
  if (inst && inst.loader !== "vanilla") {
    showChecklist.value = true;
  } else {
    showInstall.value = true;
  }
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
  // 按 data-tab 找回下标：新闻、收藏夹这两个不在 types 里，写死下标很容易错位
  () =>
    Array.from(typeBox.value?.querySelectorAll<HTMLElement>(".type-card button") ?? []).findIndex(
      (b) => b.dataset.tab === type.value,
    ),
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

// —— 页首控制条吸顶 ——
// 分类卡 + 工具栏平时是普通流式排布（首屏看起来和原来完全一样），
// 一旦滚过首屏、顶部贴住滚动容器上沿，就切到 pinned 状态：背后浮出一层模糊底衬 +
// 投影，内部间距顺手收紧一点。视觉变化全交给 CSS transition，这里只负责切状态。
//
// 滚动容器是 App.vue 里的 #app-content；BrowseView 被 KeepAlive 缓存，
// 所以监听在 activated / deactivated 里挂载和摘除——否则切到别的页面后，
// 那边的滚动依旧会改这里的 pinned 状态。
const headBox = ref<HTMLElement | null>(null);
const headPinned = ref(false);
let scrollHost: HTMLElement | null = null;
let pinRaf = 0;
let settleTimer: ReturnType<typeof setTimeout> | null = null;
// 与 App.vue 里 <Transition :duration="{ enter: 260 }"> 对齐，再留一帧余量
const ENTER_SETTLE_MS = 300;

function measurePinned() {
  pinRaf = 0;
  const el = headBox.value;
  if (!el || !scrollHost) return;
  // 贴在容器最顶上时滚动量一定 > 0；scrollTop 为 0 说明还在首屏，
  // 直接判定未吸附——省一次布局读取，也挡住激活瞬间的脏测量。
  if (scrollHost.scrollTop <= 0) {
    if (headPinned.value) headPinned.value = false;
    return;
  }
  // 判定用「头部顶边是否已经贴住滚动容器上沿」，而不是写死某个 scrollTop 阈值：
  // 阈值天然跟着容器上内边距走，改布局也不会失真。
  const pinned = el.getBoundingClientRect().top <= scrollHost.getBoundingClientRect().top + 0.5;
  if (pinned !== headPinned.value) headPinned.value = pinned;
}

function onHostScroll() {
  // 滚动事件按帧合并：一帧只量一次，避免每像素都同步触发布局计算
  if (pinRaf) return;
  pinRaf = requestAnimationFrame(measurePinned);
}

// 转场结束后补量一次：此时页面根的 top 位移已经收回，量到的是真实位置
function scheduleSettleMeasure() {
  if (settleTimer) clearTimeout(settleTimer);
  settleTimer = setTimeout(() => {
    settleTimer = null;
    onHostScroll();
  }, ENTER_SETTLE_MS);
}

function bindScrollHost() {
  if (scrollHost) return;
  scrollHost = document.getElementById("app-content");
  if (!scrollHost) return;
  scrollHost.addEventListener("scroll", onHostScroll, { passive: true });
  // 这里不能同步 measurePinned()：KeepAlive 重新激活的瞬间，页面元素刚从缓存
  // 里插回 DOM，且还挂在转场的 enter-from 位移上，量到的顶边未必是最终位置
  // （纵向转场那会儿就是被 enter-from 的 top:-32px 误判成「已吸附」，吸附底衬
  // 直接浮出，而转场结束不会有 scroll 事件来纠正这个错误状态）。
  // 所以等转场时长过去、布局稳定后再量一次。
  scheduleSettleMeasure();
}

function unbindScrollHost() {
  if (scrollHost) scrollHost.removeEventListener("scroll", onHostScroll);
  scrollHost = null;
  if (pinRaf) {
    cancelAnimationFrame(pinRaf);
    pinRaf = 0;
  }
  if (settleTimer) {
    clearTimeout(settleTimer);
    settleTimer = null;
  }
  headPinned.value = false;
}

onActivated(bindScrollHost);
onDeactivated(unbindScrollHost);
onBeforeUnmount(unbindScrollHost);

// —— 内容 API 镜像切换后自动重拉 ——
// 镜像在设置页切换（默认官方源），那边会顺手作废发现页的缓存；但本页被 KeepAlive
// 缓存着，回来时 results 仍是旧来源的数据。这里比较一次「镜像签名」，变了就重查。
function contentMirrorSignature(): string {
  const s = settingsStore.settings;
  return `${s?.content_mirror ?? "official"}|${s?.content_mirror_custom ?? ""}`;
}
const contentMirrorUsed = ref(contentMirrorSignature());
onActivated(() => {
  const now = contentMirrorSignature();
  if (now === contentMirrorUsed.value) return;
  contentMirrorUsed.value = now;
  search();
});

onMounted(async () => {
  bindScrollHost();
  try {
    instances.value = (await api.listInstances()).items;
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
  // 先用 query 定分类，再建分类选项 / 拉列表：/browse?type=shimu 要直接落在投影上
  syncTypeFromRoute();
  rebuildOptions();
  loadVersions();
  syncProviderFromRoute();
  await search();
});
</script>

<template>
  <div id="browse-root" class="browse" data-tauri-drag-region>
    <!-- 页首控制条：分类卡 + 工具栏。首屏与原来的流式排布完全一致，
         滚过首屏后整条吸附到页顶（样式见 .browse-head）。 -->
    <div ref="headBox" class="browse-head" :class="{ pinned: headPinned }">
    <!-- 吸附底衬：单独一个元素而不是 ::before，这样它能正常吃到 backdrop-filter
         （负 z-index 的伪元素在 WebView2 上磨砂不一定生效），颜色和卡片走同一档半透。 -->
    <div class="head-glass" aria-hidden="true"></div>
    <div ref="typeBox" v-segmented="{ label: t('nav.browse') }" class="type-card qx-seg">
      <div class="qx-seg__indicator" :style="typeIndicatorStyle"></div>
      <button
        v-for="tp in types"
        :key="tp.key"
        :data-tab="tp.key"
        :class="{ active: type === tp.key }"
        @click="switchType(tp.key)"
      >
        <component :is="tp.icon" class="type-icon" />
        <span>{{ tp.label }}</span>
      </button>
      <span class="type-divider"></span>
      <button
        class="news-tab"
        data-tab="fav"
        :class="{ active: type === 'fav' }"
        @click="switchType('fav')"
      >
        <IconStar class="news-tab-icon" />
        <span>{{ t("browse.fav.tab") }}</span>
      </button>
      <template v-if="showNewsEntry">
        <span class="type-divider"></span>
        <button
          class="news-tab"
          data-tab="news"
          :class="{ active: type === 'news' }"
          @click="switchType('news')"
        >
          <IconNewspaper class="news-tab-icon" />
          <span>{{ t("nav.news") }}</span>
        </button>
      </template>
    </div>

    <div v-if="type !== 'news' && !shimuDetailOpen" class="toolbar glass">
      <div class="toolbar-row">
        <NInput id="browse-search" v-model:value="searchText" class="search-box" :placeholder="searchPlaceholder" clearable>
          <template #prefix><IconSearch /></template>
        </NInput>
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
          v-if="type === 'shimu'"
          v-model:value="shimuSort"
          :options="shimuSortOptions"
          size="small"
          class="tb-select"
        />
        <n-select
          v-else
          v-model:value="sort"
          :options="sortOptions"
          size="small"
          class="tb-select"
        />
        <n-select
          v-if="!isFavTab"
          v-model:value="pageSize"
          :options="pageSizeOptions"
          size="small"
          class="tb-select page-size"
        />
        <div ref="viewBox" v-segmented="{ label: t('common.viewMode') }" class="view-switch qx-seg qx-seg--sm">
          <div class="qx-seg__indicator" :style="viewIndicatorStyle"></div>
          <button :class="{ active: view === 'grid' }" :title="t('browse.view.grid')" @click="view = 'grid'"><IconGrid /></button>
          <button :class="{ active: view === 'list' }" :title="t('browse.view.list')" @click="view = 'list'"><IconList /></button>
          <button :class="{ active: view === 'compact' }" :title="t('browse.view.compact')" @click="view = 'compact'"><IconAlignJustify /></button>
        </div>
        <button v-if="type !== 'shimu' && !isFavTab" class="filter-btn" :class="{ on: hasFilter }" @click="showFilter = true">
          <IconSliders /> {{ t("browse.filter.button") }}
        </button>
        <!-- 来源选择器：内容分类走 Modrinth / CurseForge，投影分类走已接入的投影站；
             收藏夹是本地数据，没有来源概念，那个位置改成显示条数 -->
        <template v-if="!isFavTab">
          <n-select
            v-if="type !== 'shimu'"
            v-model:value="provider"
            :options="providerOptions"
            size="small"
            class="tb-select provider"
          />
          <n-select
            v-else
            v-model:value="schematicSource"
            :options="schematicSourceOptions"
            size="small"
            class="tb-select provider"
          />
        </template>
        <!-- 收藏夹没有来源概念，同一个位置换成「资源类型」筛选 -->
        <n-select
          v-else
          v-model:value="favTypeFilter"
          :options="favTypeOptions"
          size="small"
          class="tb-select provider"
        />
        <button
          v-if="type !== 'shimu' && !isFavTab && locale.startsWith('zh')"
          class="filter-btn translate-toggle"
          :class="{ on: translateMode }"
          @click="toggleTranslateMode"
        >
          <IconLayers /> {{ translateMode ? t("browse.translate.done") : t("browse.translate.toggle") }}
        </button>
      </div>
    </div>
    </div>

    <template v-if="type !== 'news'">
    <!-- 投影：整块嵌入拾木空间投影库（列表 / 详情 / 下载都在这套组件里） -->
    <ShimuLibraryView
      v-if="type === 'shimu'"
      embedded
      :view="view"
      :source="schematicSource"
      :keyword="shimuKeywordApplied"
      :sort="shimuSort"
      :page-size="pageSize"
      :target-instance-id="selectedInstanceId"
      @detail-open="shimuDetailOpen = $event"
    />

    <template v-else>

    <div v-if="hasFilter && !isFavTab" class="filter-tags glass">
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

    <div v-if="provider === 'curseforge' && !cfCategories.length && !loading && !isFavTab" class="cf-hint glass">
      {{ t("browse.cf.needApiKeyPre") }}
      <a href="https://console.curseforge.com" target="_blank">console.curseforge.com</a>
      {{ t("browse.cf.needApiKeyMid") }} <router-link to="/settings">{{ t("browse.cf.settings") }}</router-link> {{ t("browse.cf.needApiKeyPost") }}
    </div>

    <div v-if="provider === 'all' && cfError && !loading && !isFavTab" class="cf-hint glass">
      {{ t("browse.cf.loadFailedPre", { error: cfError }) }} <router-link to="/settings">{{ t("browse.cf.settings") }}</router-link> {{ t("browse.cf.loadFailedPost") }}
    </div>

    <UiStatus v-if="!isFavTab && searchError" kind="error" :text="searchError" compact>
      <button class="qx-btn qx-btn--sm" @click="search">{{ t("common.retry") }}</button>
    </UiStatus>
    <UiStatus v-if="loading && visibleResults.length" kind="loading" :text="t('common.loading')" compact />
    <div v-show="loading && !visibleResults.length" class="grid skel-grid" role="status" :aria-label="t('common.loading')">
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
    <div v-show="!loading && !visibleResults.length && (isFavTab || !searchError)" class="center">
      <div class="center-icon"><IconStar v-if="isFavTab" /><IconSearch v-else /></div>
      <p>{{ emptyText }}</p>
      <p v-if="isFavTab && !favoritesStore.count" class="center-hint">{{ t("browse.fav.emptyHint") }}</p>
    </div>
    <div v-show="visibleResults.length" class="grid" :class="`view-${view}`">
      <ProjectCard
        v-for="(p, i) in visibleResults"
        :key="p.provider + p.id"
        class="qx-card-enter"
        :style="{ animationDelay: staggeredCardDelay(i, visibleResults.length) }"
        :project="p"
        :view="view"
        :installed="isProjectInstalled(p)"
        :mc-version="gameVersion"
        :favorite="favoritesStore.isFavorite(p)"
        :type-label="isFavTab ? resTypeLabel(p.project_type) : ''"
        :translate-mode="translateMode && !isFavTab"
        :translating="translatingSlugs.includes(p.id)"
        :translated-desc="translatedDescs[p.id] ?? null"
        @install="openInstall"
        @translate="translateCard"
        @favorite="toggleFavorite"
      />
    </div>

    <div v-if="!isFavTab && total > 20" class="pager">
      <span class="pager-total">{{ t("browse.pager.total", { count: total }) }}</span>
      <SimplePagination
        :page="page + 1"
        :page-count="pageCount"
        @update:page="onPage"
      />
    </div>
    </template>
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
          class="news-card glass qx-card-enter"
          :style="{ animationDelay: staggeredCardDelay(i, newsStore.news.length), '--card-enter-offset': '20px' }"
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
    <InstallChecklist v-model:show="showChecklist" :project="installTarget" :instance="selectedInstance" />
  </div>
</template>

<style scoped>
.browse {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

/* —— 页首控制条（分类卡 + 工具栏） ——
   首屏时它就是普通流式节点，位置和间距与改造前完全一致；
   靠 position: sticky 在滚过首屏后贴住 #app-content 的上沿。
   .pinned 只负责"视觉上的变换"（底衬浮出 + 间距收紧），全部走 transition，
   所以进出吸附都是渐变的，不会突然跳一下。 */
.browse-head {
  position: sticky;
  /* 关键：sticky 的 top 是从滚动容器的内容盒（已排除内边距）算起的，
     而 #app-content 有 22px 上内边距。写成 -22px 才能让它一路贴到页面最顶上，
     不留一条会漏出滚动卡片的缝。这个值跟着 App.vue 的 --content-pad-top 走。 */
  top: calc(-1 * var(--content-pad-top, 22px));
  z-index: var(--z-header);
  display: flex;
  flex-direction: column;
  gap: 16px;
  transition:
    gap 0.3s cubic-bezier(0.22, 0.61, 0.36, 1),
    padding 0.3s cubic-bezier(0.22, 0.61, 0.36, 1);
}
/* 吸附底衬：负 inset 正好吃掉内容区内边距，让它铺满整行；左右恰好停在容器边缘，
   不会撑出横向滚动条。未吸附时 opacity 0，完全不参与视觉。
   颜色刻意和卡片走同一档"半透 + 磨砂"，而不是实心黑/白——发现页里几乎所有表面
   （卡片、Dock）都能透出壁纸，只有这条实心的话会显得特别突兀。
   模糊比卡片给得更重，否则背后的标题会以"浮字"形式透出来。 */
.head-glass {
  position: absolute;
  inset: calc(-1 * var(--content-pad-top, 22px)) calc(-1 * var(--content-pad-x, 26px)) -8px;
  z-index: 0;
  opacity: 0;
  /* 不吸附时彻底不参与渲染：一直在跑的 backdrop-filter 是白白的性能开销，
     淡出结束后再挂 visibility:hidden。 */
  visibility: hidden;
  transform: translateY(-8px);
  pointer-events: none;
  background: rgba(22, 25, 34, 0.68);
  box-shadow: 0 10px 28px var(--k-30);
  backdrop-filter: blur(calc(var(--glass-blur, 8px) + 16px)) saturate(1.25);
  -webkit-backdrop-filter: blur(calc(var(--glass-blur, 8px) + 16px)) saturate(1.25);
  transition:
    opacity 0.24s ease,
    transform 0.34s cubic-bezier(0.22, 0.61, 0.36, 1),
    visibility 0s linear 0.34s;
}
:root.light .head-glass {
  background: rgba(255, 255, 255, 0.68);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.12);
}
.browse-head.pinned {
  gap: 8px;
  /* 面板化进底衬后，分类药丸离窗口最上沿只剩它自己的 5px，太挤；补一点呼吸 */
  padding-top: 7px;
}
.browse-head.pinned .head-glass {
  opacity: 1;
  transform: none;
  visibility: visible;
  transition:
    opacity 0.24s ease,
    transform 0.34s cubic-bezier(0.22, 0.61, 0.36, 1),
    visibility 0s linear 0s;
}

.toolbar {
  /* 压在 .head-glass 上面（它是 absolute + z-index 0，静态元素会被它盖住） */
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
  /* 透明上边框先占位，吸附时只改颜色就能淡入分隔线，不会因为多出 1px 而抖一下 */
  border-top: 1px solid transparent;
  transition:
    padding 0.3s cubic-bezier(0.22, 0.61, 0.36, 1),
    gap 0.3s cubic-bezier(0.22, 0.61, 0.36, 1),
    background-color 0.26s ease,
    border-radius 0.26s ease,
    border-color 0.26s ease;
}
/* —— 吸附态：两块玻璃面板"化"进底衬 ——
   底衬本身已经是一整条了，面板再叠上去就成了卡片套卡片。
   这里把面板的背景 / 圆角 / 模糊都收掉，只留里面的控件贴在底衬上，
   左内边距收到 5px 与分类卡的 5px 对齐（分类卡不能改，指示条是按它的盒算的）。 */
.browse-head.pinned .type-card,
.browse-head.pinned .toolbar {
  background: transparent;
  border-radius: 0;
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
}
.browse-head.pinned .toolbar {
  padding: 10px 5px 9px;
  gap: 8px;
  border-top-color: var(--border);
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
.search-box { flex: 1; min-width: 220px; }
/* 外观（容器/按钮/滑块/悬停）全部走全局 .qx-seg，这里只留吸顶面板的层级与过渡 */
.type-card {
  z-index: 1;
  align-self: flex-start;
  /* 吸顶时面板要"化"进底衬：背景和圆角都得能过渡 */
  transition:
    background-color 0.26s ease,
    border-radius 0.26s ease;
}
/* 分类图标：跟着文字一起变色，宽度固定不参与收缩 */
.type-icon {
  font-size: var(--fs-base);
  flex-shrink: 0;
}
.filter-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border);
  background: var(--w-04);
  color: var(--text-2);
  border-radius: var(--r-sm);
  padding: 7px 13px;
  font-size: var(--fs-sm);
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
  font-size: var(--fs-xs);
  color: var(--text-2);
  font-weight: 600;
}
.filter-hint {
  font-size: var(--fs-2xs);
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
  border-radius: var(--r-sm);
  padding: 5px 11px;
  font-size: var(--fs-xs);
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
  font-size: var(--fs-sm);
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
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 16px;
}
/* 骨架屏：只排布局，颜色变量 / shimmer / 条块尺寸走 styles.css 的公共原子（投影列表共用同一套） */
.skel-card {
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.skel-main { display: flex; gap: 12px; }
.skel-body { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
.skel-cats { display: flex; gap: 6px; margin-top: 2px; }
.skel-foot { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
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
/* 网格 / 列表 / 紧凑：外观走全局 .qx-seg--sm，这里只固定成等宽图标按钮 */
.view-switch > button {
  width: 30px;
  padding: 0;
  font-size: var(--fs-base); /* 图标尺寸（svg 为 1em），不是文字字号 */
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
  font-size: var(--fs-xs);
  background: var(--accent-soft);
  color: var(--accent);
  border: 1px solid var(--accent-03);
  border-radius: var(--r-sm);
  padding: 3px 10px;
}
.ftag-x {
  border: none;
  background: transparent;
  color: inherit;
  cursor: pointer;
  display: inline-flex;
  padding: 0;
  font-size: var(--fs-xs);
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
  font-size: var(--fs-sm);
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
  font-size: var(--fs-md);
}
.center-hint {
  margin-top: 6px;
  font-size: var(--fs-xs);
  color: var(--text-3);
  opacity: 0.8;
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
  /* 与实例卡 / 服务器卡 / 项目卡同一套：只靠 --panel 明度差定义边界，不加描边；
     圆角走 --r-lg，不再单独写 12px。磨砂与底色由 .glass 提供。 */
  border-radius: var(--r-lg);
  overflow: hidden;
  cursor: pointer;
  box-shadow: var(--card-shadow);
  transition: transform 0.15s ease, box-shadow var(--dur-md) var(--ease-out);
}
/* 原来这里是 `0 4px 20px var(--k-15)` 的黑色投影，深色主题下和卡片其它投影
   是两套语言；统一改用卡片的悬停令牌（深色 accent 细环 / 浅色浅投影 + 细环）。
   图片放大保留，它是新闻卡独有的「信息正在展开」提示。 */
.news-card:hover {
  box-shadow: var(--card-shadow-hover);
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
  font-size: var(--fs-base);
  font-weight: 700;
  margin: 0;
  color: var(--text-1);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.news-desc {
  font-size: var(--fs-xs);
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
  font-size: var(--fs-2xs);
  color: var(--text-3);
  margin-top: auto;
}
.meta-author {
  color: var(--accent);
  font-weight: 600;
}
</style>
