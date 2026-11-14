<script setup lang="ts">
/**
 * 材质包预览：打开 .zip / 文件夹，浏览包内全部贴图。
 * 贴图清单由后端一次解析到位，图片本体按可视区域懒加载（并发 6），
 * 避免一次性拉几千张图把 IPC 和内存压垮。
 */
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useI18n } from "vue-i18n";
import { NPopover, NModal, NSelect, useMessage } from "naive-ui";
import {
  IconArchive,
  IconClock,
  IconFolder,
  IconImage,
  IconSearch,
} from "../components/icons";
import { fmtSize } from "../utils/format";
import { useInstancesStore } from "../stores/instances";

const { t, te } = useI18n();
const message = useMessage();
const instancesStore = useInstancesStore();

interface PackTexture {
  path: string;
  label: string;
  fileName: string;
  category: string;
  namespace: string;
  width: number;
  height: number;
  frames: number;
  bytes: number;
}

interface PackManifest {
  sessionId: string;
  path: string;
  kind: "zip" | "folder";
  name: string;
  packFormat: number | null;
  supportedFormats: string | null;
  description: string;
  icon: string | null;
  fileSize: number;
  textures: PackTexture[];
  namespaces: string[];
  resolutions: number[];
  animated: number;
  models: number;
  sounds: number;
  langs: number;
}

interface InstancePack {
  path: string;
  name: string;
  size: number;
}

const manifest = ref<PackManifest | null>(null);
const sessionId = ref<string | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

// —— 筛选 ——
const search = ref("");
const category = ref("all");
const onlyAnimated = ref(false);

// —— 列表分批渲染 ——
const PAGE_SIZE = 200;
const visibleCount = ref(PAGE_SIZE);
const gridRef = ref<HTMLElement | null>(null);

const categories = computed(() => {
  const map = new Map<string, number>();
  for (const tex of manifest.value?.textures ?? []) {
    map.set(tex.category, (map.get(tex.category) ?? 0) + 1);
  }
  return [...map.entries()]
    .sort((a, b) => b[1] - a[1])
    .map(([key, count]) => ({ key, count }));
});

function categoryLabel(key: string): string {
  const k = `resourcePack.cat.${key}`;
  return te(k) ? t(k) : key;
}

const filtered = computed(() => {
  const list = manifest.value?.textures ?? [];
  const q = search.value.trim().toLowerCase();
  return list.filter((tex) => {
    if (category.value !== "all" && tex.category !== category.value) return false;
    if (onlyAnimated.value && tex.frames <= 1) return false;
    if (q && !tex.label.toLowerCase().includes(q)) return false;
    return true;
  });
});

const visibleTextures = computed(() => filtered.value.slice(0, visibleCount.value));

watch([search, category, onlyAnimated], () => {
  visibleCount.value = PAGE_SIZE;
  if (gridRef.value) gridRef.value.scrollTop = 0;
});

function onGridScroll() {
  const el = gridRef.value;
  if (!el) return;
  if (el.scrollTop + el.clientHeight >= el.scrollHeight - 320) {
    if (visibleCount.value < filtered.value.length) {
      visibleCount.value += PAGE_SIZE;
    }
  }
}

// —— 贴图按需加载 ——
const urls = reactive<Record<string, string>>({});
const queued = new Set<string>();
const queue: string[] = [];
let activeLoads = 0;
const LOAD_CONCURRENCY = 6;
let loadSeq = 0;

function requestTexture(path: string) {
  if (!sessionId.value || !path || urls[path] || queued.has(path)) return;
  queued.add(path);
  queue.push(path);
  pumpLoads();
}

function pumpLoads() {
  const sid = sessionId.value;
  if (!sid) return;
  while (activeLoads < LOAD_CONCURRENCY && queue.length) {
    const path = queue.shift()!;
    const seq = loadSeq;
    activeLoads++;
    invoke<string>("resourcepack_texture", { sessionId: sid, path })
      .then((url) => {
        // 换包之后回来的旧结果直接丢弃
        if (seq === loadSeq) urls[path] = url;
      })
      .catch(() => {
        /* 单张贴图损坏不影响整体浏览 */
      })
      .finally(() => {
        activeLoads = Math.max(0, activeLoads - 1);
        queued.delete(path);
        if (seq === loadSeq) pumpLoads();
      });
  }
}

function resetTextureCache() {
  loadSeq++;
  for (const key of Object.keys(urls)) delete urls[key];
  queue.length = 0;
  queued.clear();
}

// 缩略图进入视口才开始拉取
let observer: IntersectionObserver | null = null;
function observeThumb(el: unknown, path: string) {
  if (!(el instanceof HTMLElement)) return;
  el.dataset.path = path;
  observer?.observe(el);
}

// —— 详情面板 ——
const selected = ref<PackTexture | null>(null);
const zoom = ref(2);
const playing = ref(true);

const selectedUrl = computed(() => (selected.value ? urls[selected.value.path] : undefined));
const selectedFrameHeight = computed(() => {
  const tex = selected.value;
  if (!tex) return 1;
  return Math.max(1, Math.round(tex.height / tex.frames));
});
const previewFrameStyle = computed(() => {
  const tex = selected.value;
  if (!tex) return {};
  const width = Math.min(Math.max(tex.width * zoom.value, 64), 560);
  return {
    width: `${width}px`,
    aspectRatio: `${tex.width} / ${selectedFrameHeight.value}`,
  };
});
/** 动画播放：关键帧名写在 CSS 类里（scoped 会把名字一起改写），
 *  这里只下发时长 / 阶梯数与位移量，避免内联的 animation 覆盖掉类里的名字。 */
const previewImageStyle = computed(() => {
  const tex = selected.value;
  if (!tex) return {} as Record<string, string>;
  const style: Record<string, string> = { imageRendering: "pixelated" };
  if (tex.frames > 1 && playing.value) {
    style.animationDuration = `${(tex.frames * 0.05).toFixed(2)}s`;
    style.animationTimingFunction = `steps(${tex.frames})`;
    style["--rp-shift"] = `${((-100 * (tex.frames - 1)) / tex.frames).toFixed(4)}%`;
  }
  return style;
});

watch(selected, (tex) => {
  if (tex) requestTexture(tex.path);
}, { immediate: true });

// —— 最近打开 ——
interface RecentPack {
  path: string;
  name: string;
  textures: number;
  at: number;
}
const RECENT_KEY = "qookix.resourcepack.recent";
const recent = ref<RecentPack[]>([]);

function loadRecent() {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    const arr = raw ? JSON.parse(raw) : [];
    recent.value = Array.isArray(arr) ? arr.slice(0, 20) : [];
  } catch {
    recent.value = [];
  }
}

function pushRecent(m: PackManifest) {
  const item: RecentPack = {
    path: m.path,
    name: m.name,
    textures: m.textures.length,
    at: Date.now(),
  };
  recent.value = [item, ...recent.value.filter((r) => r.path !== m.path)].slice(0, 20);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(recent.value));
  } catch { /* ignore */ }
}

function clearRecent() {
  recent.value = [];
  try { localStorage.removeItem(RECENT_KEY); } catch { /* ignore */ }
}

// —— 打开材质包 ——
async function pickFile() {
  const picked = await openDialog({
    multiple: false,
    filters: [{ name: "Resource Packs", extensions: ["zip"] }],
  });
  if (typeof picked !== "string" || !picked) return;
  await openPack(picked);
}

async function pickFolder() {
  const picked = await openDialog({ directory: true, multiple: false });
  if (typeof picked !== "string" || !picked) return;
  await openPack(picked);
}

async function openPack(path: string) {
  if (loading.value) return;
  loading.value = true;
  error.value = null;
  try {
    const m = await invoke<PackManifest>("resourcepack_open", { path });
    const prev = sessionId.value;
    sessionId.value = m.sessionId;
    manifest.value = m;
    resetTextureCache();
    search.value = "";
    category.value = "all";
    onlyAnimated.value = false;
    visibleCount.value = PAGE_SIZE;
    zoom.value = 2;
    playing.value = true;
    selected.value = m.textures[0] ?? null;
    if (gridRef.value) gridRef.value.scrollTop = 0;
    pushRecent(m);
    if (prev && prev !== m.sessionId) {
      invoke("resourcepack_close", { sessionId: prev }).catch(() => {});
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}

// —— 从实例打开 ——
const instanceModal = ref(false);
const instanceId = ref<string | null>(null);
const instancePacks = ref<InstancePack[]>([]);
const packsLoading = ref(false);
const instanceOptions = computed(() =>
  instancesStore.instances.map((i) => ({ label: i.name, value: i.id }))
);

async function openInstancePicker() {
  void instancesStore.load();
  instanceModal.value = true;
  if (!instanceId.value) instanceId.value = instanceOptions.value[0]?.value ?? null;
  if (instanceId.value) await loadInstancePacks();
}

async function loadInstancePacks() {
  const id = instanceId.value;
  if (!id) {
    instancePacks.value = [];
    return;
  }
  packsLoading.value = true;
  try {
    instancePacks.value = await invoke<InstancePack[]>("resourcepack_list_instance_packs", {
      instanceId: id,
    });
  } catch (e) {
    instancePacks.value = [];
    message.error(String(e));
  } finally {
    packsLoading.value = false;
  }
}

watch(instanceId, () => void loadInstancePacks());

async function chooseInstancePack(pack: InstancePack) {
  instanceModal.value = false;
  await openPack(pack.path);
}

onMounted(() => {
  loadRecent();
  observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const el = entry.target as HTMLElement;
        observer?.unobserve(el);
        if (el.dataset.path) requestTexture(el.dataset.path);
      }
    },
    { rootMargin: "480px 0px" }
  );
});

onBeforeUnmount(() => {
  observer?.disconnect();
  observer = null;
  if (sessionId.value) {
    invoke("resourcepack_close", { sessionId: sessionId.value }).catch(() => {});
  }
});

function resolutionLabel(widths: number[]): string {
  if (!widths.length) return "—";
  const head = widths.slice(0, 3).join("x / ");
  return widths.length > 3 ? `${head}x …` : `${head}x`;
}
</script>

<template>
  <div class="rp-view">
    <div class="rp-main">
      <!-- 参数栏 -->
      <div class="glass param-bar">
        <button class="mini-btn primary" :disabled="loading" @click="pickFile">
          <IconFolder />
          {{ loading ? t("resourcePack.loading") : t("resourcePack.open") }}
        </button>
        <button class="mini-btn" :disabled="loading" @click="pickFolder">
          <IconArchive />{{ t("resourcePack.openFolder") }}
        </button>
        <NPopover trigger="click" placement="bottom-start" :width="320">
          <template #trigger>
            <button class="mini-btn" :disabled="!recent.length">
              <IconClock />{{ t("resourcePack.recent") }}({{ recent.length }})
            </button>
          </template>
          <div class="recent-panel">
            <div class="recent-head">
              <span>{{ t("resourcePack.recent") }}</span>
              <button class="link-btn" @click="clearRecent">{{ t("resourcePack.clearRecent") }}</button>
            </div>
            <div v-if="!recent.length" class="muted">{{ t("resourcePack.noRecent") }}</div>
            <button
              v-for="(r, i) in recent"
              :key="i"
              class="recent-item"
              @click="openPack(r.path)"
            >
              <span class="rf-name">{{ r.name }}</span>
              <span class="rf-meta">{{ t("resourcePack.textures") }} {{ r.textures }}</span>
            </button>
          </div>
        </NPopover>
        <span class="param-sep" />
        <button class="mini-btn" :disabled="loading" @click="openInstancePicker">
          <IconImage />{{ t("resourcePack.fromInstance") }}
        </button>
      </div>

      <div v-if="error" class="error glass">{{ error }}</div>

      <template v-if="manifest">
        <!-- 材质包信息 -->
        <div class="glass pack-head">
          <div class="pack-icon">
            <img v-if="manifest.icon" :src="manifest.icon" alt="" />
            <IconImage v-else class="pack-icon-ph" />
          </div>
          <div class="pack-meta">
            <div class="pack-title">
              <h2>{{ manifest.name }}</h2>
              <span class="tag">{{ manifest.kind === "zip" ? "ZIP" : t("resourcePack.folder") }}</span>
            </div>
            <p v-if="manifest.description" class="pack-desc">{{ manifest.description }}</p>
            <p v-else class="pack-desc muted">{{ manifest.path }}</p>
            <div class="pack-chips">
              <span v-if="manifest.packFormat != null" class="chip-static">
                {{ t("resourcePack.packFormat") }} {{ manifest.packFormat }}
                <template v-if="manifest.supportedFormats"> ({{ manifest.supportedFormats }})</template>
              </span>
              <span class="chip-static">{{ t("resourcePack.resolution") }} {{ resolutionLabel(manifest.resolutions) }}</span>
              <span class="chip-static">{{ t("resourcePack.textures") }} {{ manifest.textures.length }}</span>
              <span v-if="manifest.animated" class="chip-static">{{ t("resourcePack.animated") }} {{ manifest.animated }}</span>
              <span v-if="manifest.models" class="chip-static">{{ t("resourcePack.models") }} {{ manifest.models }}</span>
              <span v-if="manifest.langs" class="chip-static">{{ t("resourcePack.langs") }} {{ manifest.langs }}</span>
              <span class="chip-static">{{ fmtSize(manifest.fileSize) }}</span>
            </div>
          </div>
        </div>

        <!-- 搜索 + 分类 -->
        <div class="glass filter-bar">
          <div class="search-box">
            <IconSearch />
            <input
              v-model="search"
              type="text"
              :placeholder="t('resourcePack.searchPlaceholder')"
            />
          </div>
          <div class="chips">
            <button class="chip" :class="{ active: category === 'all' }" @click="category = 'all'">
              {{ t("resourcePack.all") }} {{ manifest.textures.length }}
            </button>
            <button
              v-for="c in categories"
              :key="c.key"
              class="chip"
              :class="{ active: category === c.key }"
              @click="category = c.key"
            >
              {{ categoryLabel(c.key) }} {{ c.count }}
            </button>
          </div>
          <button
            class="chip anim-toggle"
            :class="{ active: onlyAnimated }"
            @click="onlyAnimated = !onlyAnimated"
          >
            {{ t("resourcePack.onlyAnimated") }}
          </button>
        </div>

        <!-- 贴图墙 -->
        <div ref="gridRef" class="glass grid-wrap" @scroll="onGridScroll">
          <div v-if="filtered.length" class="tex-grid">
            <button
              v-for="tex in visibleTextures"
              :key="tex.path"
              :ref="(el) => observeThumb(el, tex.path)"
              class="tex-cell"
              :class="{ selected: selected?.path === tex.path }"
              :title="tex.label"
              @click="selected = tex"
            >
              <div class="thumb">
                <img
                  v-if="urls[tex.path]"
                  class="thumb-img"
                  :class="{ anim: tex.frames > 1 }"
                  :src="urls[tex.path]"
                  alt=""
                  decoding="async"
                />
              </div>
              <span class="tex-name">{{ tex.fileName }}</span>
              <span v-if="tex.frames > 1" class="tex-badge">
                {{ tex.frames }}
              </span>
            </button>
          </div>
          <div v-else class="empty-hint">
            <p class="muted">{{ t("resourcePack.noMatch") }}</p>
          </div>
        </div>
      </template>

      <!-- 空状态 -->
      <div v-else-if="!loading" class="glass empty-state">
        <div class="empty-copy">
          <h3>{{ t("resourcePack.title") }}</h3>
          <p class="muted">{{ t("resourcePack.empty") }}</p>
        </div>
        <div v-if="recent.length" class="recent-grid">
          <h4>{{ t("resourcePack.recent") }}</h4>
          <div class="recent-cards">
            <button
              v-for="(r, i) in recent.slice(0, 8)"
              :key="i"
              class="recent-card glass clickable"
              @click="openPack(r.path)"
            >
              <span class="rc-name">{{ r.name }}</span>
              <span class="rc-meta">{{ t("resourcePack.textures") }} {{ r.textures }}</span>
            </button>
          </div>
        </div>
      </div>

      <div v-if="loading && !manifest" class="glass loading-state">
        <div class="spinner" />
        <span>{{ t("resourcePack.loading") }}</span>
      </div>
    </div>

    <!-- 右侧：选中贴图详情 -->
    <aside v-if="manifest" class="glass rp-side">
      <template v-if="selected">
        <h3 class="panel-title">{{ selected.fileName }}</h3>
        <div class="preview-stage">
          <div v-if="selectedUrl" class="preview-frame" :style="previewFrameStyle">
            <img
              :src="selectedUrl"
              alt=""
              :class="{ 'rp-anim': selected.frames > 1 && playing }"
              :style="previewImageStyle"
            />
          </div>
          <span v-else class="muted">{{ t("resourcePack.loading") }}</span>
        </div>
        <div class="zoom-row">
          <button
            v-for="z in [1, 2, 4, 8]"
            :key="z"
            class="mini-btn zoom-btn"
            :class="{ primary: zoom === z }"
            @click="zoom = z"
          >
            {{ z }}x
          </button>
          <button
            v-if="selected.frames > 1"
            class="mini-btn"
            :class="{ primary: playing }"
            @click="playing = !playing"
          >
            {{ playing ? t("resourcePack.pauseAnim") : t("resourcePack.playAnim") }}
          </button>
        </div>
        <div class="side-sep" />
        <div class="info-item">
          <span class="label">{{ t("resourcePack.path") }}</span>
          <span class="value break">{{ selected.path }}</span>
        </div>
        <div class="info-item">
          <span class="label">{{ t("resourcePack.size") }}</span>
          <span class="value">
            {{ selected.width }} × {{ selected.height }}
            <template v-if="selected.frames > 1">
              ({{ t("resourcePack.frames", { n: selected.frames }) }})
            </template>
          </span>
        </div>
        <div class="info-item">
          <span class="label">{{ t("resourcePack.category") }}</span>
          <span class="value">
            {{ categoryLabel(selected.category) }} · {{ selected.namespace }}
          </span>
        </div>
        <div class="info-item">
          <span class="label">{{ t("resourcePack.fileSize") }}</span>
          <span class="value">{{ fmtSize(selected.bytes) }}</span>
        </div>
      </template>
      <p v-else class="muted">{{ t("resourcePack.selectHint") }}</p>
    </aside>

    <!-- 从实例选择材质包 -->
    <NModal
      v-model:show="instanceModal"
      preset="card"
      :title="t('resourcePack.instanceTitle')"
      style="max-width: 460px"
      :bordered="false"
      size="small"
    >
      <div v-if="!instanceOptions.length" class="muted">{{ t("resourcePack.noInstance") }}</div>
      <template v-else>
        <NSelect
          v-model:value="instanceId"
          :options="instanceOptions"
          :placeholder="t('resourcePack.selectInstance')"
          filterable
        />
        <div class="pack-list">
          <div v-if="packsLoading" class="muted">{{ t("resourcePack.loading") }}</div>
          <div v-else-if="!instancePacks.length" class="muted">{{ t("resourcePack.noPack") }}</div>
          <button
            v-for="p in instancePacks"
            :key="p.path"
            class="pack-item"
            @click="chooseInstancePack(p)"
          >
            <IconArchive />
            <span class="pi-name">{{ p.name }}</span>
            <span class="pi-size">{{ fmtSize(p.size) }}</span>
          </button>
        </div>
      </template>
    </NModal>
  </div>
</template>

<style scoped>
.rp-view {
  display: flex;
  gap: 16px;
  height: 100%;
  min-height: 0;
}
.rp-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
}

/* —— 通用按钮 / 玻璃条 —— */
.param-bar { padding: 10px 14px; display: flex; gap: 8px; align-items: center; flex: none; }
.param-sep { width: 1px; align-self: stretch; background: var(--border); }
.link-btn { padding: 0; border: none; background: none; color: var(--text-3); font-size: var(--fs-xs); cursor: pointer; }
.link-btn:hover { color: var(--accent); }

/* —— 最近打开 —— */
.recent-panel { display: flex; flex-direction: column; gap: 2px; max-height: 360px; overflow-y: auto; }
.recent-head { display: flex; align-items: center; justify-content: space-between; font-size: var(--fs-xs); color: var(--text-3); padding-bottom: 6px; }
.recent-item { display: flex; flex-direction: column; gap: 2px; text-align: left; padding: 6px 8px; border: none; border-radius: var(--r-xs); background: transparent; cursor: pointer; color: var(--text-1); font: inherit; }
.recent-item:hover { background: var(--panel-hover); }
.rf-name { font-size: var(--fs-sm); }
.rf-meta { font-size: var(--fs-2xs); color: var(--text-3); }

/* —— 材质包信息 —— */
.pack-head { display: flex; gap: 14px; padding: 14px 16px; flex: none; }
.pack-icon {
  width: 64px;
  height: 64px;
  flex: none;
  border-radius: var(--r-md);
  overflow: hidden;
  background: var(--w-06, rgba(255, 255, 255, 0.06));
  display: flex;
  align-items: center;
  justify-content: center;
}
.pack-icon img { width: 100%; height: 100%; object-fit: cover; image-rendering: pixelated; }
.pack-icon-ph { font-size: var(--fs-xl); color: var(--text-3); }
.pack-meta { min-width: 0; display: flex; flex-direction: column; gap: 6px; }
.pack-title { display: flex; align-items: center; gap: 8px; min-width: 0; }
.pack-title h2 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tag {
  flex: none;
  padding: 1px 6px;
  border-radius: var(--r-xs);
  font-size: var(--fs-2xs);
  font-weight: 700;
  letter-spacing: 0.06em;
  color: var(--accent);
  background: var(--accent-16);
  border: 1px solid var(--accent-35);
}
.pack-desc {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-2);
  line-height: 1.55;
  overflow: hidden;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  word-break: break-all;
}
.pack-chips { display: flex; flex-wrap: wrap; gap: 6px; }
.chip-static {
  padding: 2px 8px;
  border-radius: var(--r-xs);
  font-size: var(--fs-2xs);
  color: var(--text-3);
  background: var(--w-04);
  border: 1px solid var(--border);
}

/* —— 搜索 + 分类 —— */
.filter-bar { display: flex; align-items: center; gap: 10px; padding: 8px 12px; flex: none; }
.search-box {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: none;
  width: 220px;
  padding: 6px 10px;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: var(--w-04);
  color: var(--text-3);
}
.search-box svg { font-size: var(--fs-base); flex: none; }
.search-box input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text-1);
  font: inherit;
  font-size: var(--fs-xs);
}
.chips { display: flex; gap: 6px; overflow-x: auto; flex: 1; min-width: 0; padding-bottom: 2px; }
.chip {
  flex: none;
  padding: 5px 10px;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: var(--w-04);
  color: var(--text-3);
  font-size: var(--fs-2xs);
  cursor: pointer;
  white-space: nowrap;
  transition: background 0.15s, color 0.15s, border-color 0.15s;
}
.chip:hover { color: var(--text-1); background: var(--w-08); }
.chip.active { color: var(--accent); border-color: var(--accent-45); background: var(--accent-16); }
.anim-toggle { flex: none; }

/* —— 贴图墙 —— */
.grid-wrap { flex: 1; min-height: 0; overflow-y: auto; padding: 12px; }
.tex-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(96px, 1fr));
  gap: 10px;
}
.tex-cell {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 8px 6px;
  border-radius: var(--r-sm);
  border: 1px solid transparent;
  background: var(--w-03);
  color: var(--text-2);
  cursor: pointer;
  font: inherit;
  transition: border-color 0.15s, background 0.15s;
}
.tex-cell:hover { background: var(--w-06); border-color: var(--border); }
.tex-cell.selected { border-color: var(--accent-45); background: var(--accent-16); }
.thumb {
  width: 100%;
  aspect-ratio: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  border-radius: var(--r-xs);
  background-image:
    linear-gradient(45deg, var(--w-06) 25%, transparent 25%),
    linear-gradient(-45deg, var(--w-06) 25%, transparent 25%),
    linear-gradient(45deg, transparent 75%, var(--w-06) 75%),
    linear-gradient(-45deg, transparent 75%, var(--w-06) 75%);
  background-size: 12px 12px;
  background-position: 0 0, 0 6px, 6px -6px, -6px 0;
}
.thumb-img {
  max-width: 100%;
  max-height: 100%;
  image-rendering: pixelated;
}
/* 动画贴图是一整条竖图：裁到顶部一帧 */
.thumb-img.anim {
  width: 100%;
  height: 100%;
  object-fit: cover;
  object-position: top center;
}
.tex-name {
  width: 100%;
  font-size: var(--fs-2xs);
  line-height: 1.3;
  color: var(--text-3);
  text-align: center;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tex-badge {
  position: absolute;
  top: 4px;
  right: 4px;
  padding: 0 4px;
  border-radius: var(--r-xs);
  font-size: var(--fs-2xs);
  color: var(--accent);
  background: var(--accent-16);
  border: 1px solid var(--accent-35);
}
.empty-hint { display: flex; align-items: center; justify-content: center; height: 100%; }

/* —— 右侧详情 —— */
.rp-side {
  flex: none;
  width: 280px;
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  overflow-y: auto;
}
.panel-title {
  margin: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.preview-stage {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 10px;
  border-radius: var(--r-md);
  border: 1px solid var(--border);
  min-height: 120px;
  overflow: hidden;
  background-image:
    linear-gradient(45deg, var(--w-06) 25%, transparent 25%),
    linear-gradient(-45deg, var(--w-06) 25%, transparent 25%),
    linear-gradient(45deg, transparent 75%, var(--w-06) 75%),
    linear-gradient(-45deg, transparent 75%, var(--w-06) 75%);
  background-size: 16px 16px;
  background-position: 0 0, 0 8px, 8px -8px, -8px 0;
}
.preview-frame { overflow: hidden; max-width: 100%; }
.preview-frame img { display: block; width: 100%; height: auto; }
/* 动画贴图是一整条竖图：按帧高逐帧位移（时长 / 阶梯数由内联 style 下发） */
.preview-frame img.rp-anim {
  animation-name: rp-frames;
  animation-iteration-count: infinite;
}
@keyframes rp-frames {
  from { transform: translateY(0); }
  to { transform: translateY(var(--rp-shift, 0%)); }
}
.zoom-row { display: flex; gap: 6px; flex-wrap: wrap; }
.zoom-btn { padding: 5px 10px; }
.side-sep { height: 1px; background: var(--border); }
.info-item { display: flex; flex-direction: column; gap: 2px; }
.label { font-size: var(--fs-2xs); color: var(--text-3); }
.value { font-size: var(--fs-xs); color: var(--text-1); }
.break { word-break: break-all; }

/* —— 空 / 加载 —— */
.empty-state { flex: 1; border-radius: var(--r-md); padding: 24px; display: flex; flex-direction: column; gap: 18px; overflow-y: auto; }
.empty-copy h3 { margin: 0 0 8px; font-size: var(--fs-md); }
.empty-copy p { margin: 0; font-size: var(--fs-xs); line-height: 1.6; }
.recent-grid h4 { margin: 0 0 10px; font-size: var(--fs-sm); font-weight: 600; }
.recent-cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 10px; }
.recent-card { display: flex; flex-direction: column; gap: var(--sp-1); padding: var(--sp-5); border-radius: var(--r-md); border: 1px solid var(--border); cursor: pointer; text-align: left; font: inherit; color: var(--text-1); transition: border-color var(--dur-sm) var(--ease-out), background var(--dur-sm) var(--ease-out); }
.recent-card:hover { border-color: var(--accent-45); background: var(--accent-soft); }
.rc-name { font-size: var(--fs-sm); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rc-meta { font-size: var(--fs-2xs); color: var(--text-3); }
.loading-state { flex: 1; display: flex; align-items: center; justify-content: center; gap: 10px; border-radius: var(--r-md); font-size: var(--fs-sm); color: var(--text-2); }
.spinner {
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: 2px solid var(--w-12, rgba(255, 255, 255, 0.14));
  border-top-color: var(--accent);
  animation: rp-spin 0.8s linear infinite;
}
@keyframes rp-spin { to { transform: rotate(360deg); } }
.error { padding: 12px 16px; border-radius: var(--r-md); color: var(--danger); font-size: var(--fs-sm); }
.muted { color: var(--text-3); font-size: var(--fs-xs); }

/* —— 实例材质包列表 —— */
.pack-list { margin-top: 10px; display: flex; flex-direction: column; gap: 4px; max-height: 320px; overflow-y: auto; }
.pack-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: var(--r-sm);
  border: 1px solid transparent;
  background: transparent;
  color: var(--text-1);
  font: inherit;
  font-size: var(--fs-xs);
  text-align: left;
  cursor: pointer;
}
.pack-item:hover { background: var(--panel-hover); border-color: var(--border); }
.pack-item svg { font-size: var(--fs-base); color: var(--text-3); flex: none; }
.pi-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.pi-size { font-size: var(--fs-2xs); color: var(--text-3); flex: none; }
</style>
