<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, ref, watch, nextTick } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { NSelect, useMessage } from "naive-ui";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api } from "../api";
import type { ProjectGalleryImage, ProjectHit, ProjectVersion } from "../types";
import { fmtCount as fmt, fmtDateLocale as fmtDate } from "../utils/format";
import { translateCategory } from "../utils/categories";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import { useSettingsStore } from "../stores/settings";
import { useInstancesStore } from "../stores/instances";
import SimplePagination from "../components/SimplePagination.vue";
import InstallDialog from "../components/InstallDialog.vue";
import {
  IconDownload,
  IconChevronLeft,
  IconChevronRight,
  IconClose,
  IconExternal,
  IconGlobe,
} from "../components/icons";

const route = useRoute();
const router = useRouter();
const { t, locale } = useI18n();
const message = useMessage();
const settingsStore = useSettingsStore();
const instancesStore = useInstancesStore();

const provider = computed(() => route.params.provider as string);
const projectId = computed(() => route.params.id as string);

const project = ref<ProjectHit | null>(null);
const loading = ref(true);
const failed = ref(false);
const tab = ref<"desc" | "versions" | "gallery">("desc");

/** 只保留有图的项目图片；没有图库时「图库」页签整体不出现 */
const gallery = computed<ProjectGalleryImage[]>(() =>
  (project.value?.gallery ?? []).filter((g) => !!g?.url),
);
const hasGallery = computed(() => gallery.value.length > 0);

/** 页签随图库有无变化，指示条按实际下标滑动 */
const tabKeys = computed<(typeof tab.value)[]>(() =>
  hasGallery.value ? ["desc", "versions", "gallery"] : ["desc", "versions"],
);
const tabIndex = computed(() => Math.max(0, tabKeys.value.indexOf(tab.value)));

const tabBox = ref<HTMLElement | null>(null);
const { indicatorStyle: tabIndicatorStyle, refresh: refreshTabIndicator } = useSlidingIndicator(
  tabBox,
  () => Array.from(tabBox.value?.querySelectorAll<HTMLElement>(".type-card button") ?? []),
  () => tabIndex.value,
);
watch(tab, () => nextTick(() => refreshTabIndicator()));
watch(project, () => nextTick(() => refreshTabIndicator()));
watch(tabKeys, () => {
  // 项目切到没图的那个时，别停在一个已经不存在的页签上
  if (!tabKeys.value.includes(tab.value)) tab.value = "desc";
});

// ---- 图库大图查看器 ----
const viewerIndex = ref(-1);
const viewerOpen = computed(() => viewerIndex.value >= 0 && viewerIndex.value < gallery.value.length);
const viewerImage = computed(() => (viewerOpen.value ? gallery.value[viewerIndex.value] : null));

function openViewer(i: number) {
  viewerIndex.value = i;
}
function closeViewer() {
  viewerIndex.value = -1;
}
/** 循环切换：到头/到尾自动绕回，避免按钮变灰 */
function stepViewer(delta: number) {
  const n = gallery.value.length;
  if (!n || !viewerOpen.value) return;
  viewerIndex.value = (viewerIndex.value + delta + n) % n;
}
function onViewerKey(e: KeyboardEvent) {
  if (!viewerOpen.value) return;
  if (e.key === "Escape") {
    e.preventDefault();
    closeViewer();
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    stepViewer(-1);
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    stepViewer(1);
  }
}
// 只在打开时挂监听，关掉就摘，避免常驻键盘事件
watch(viewerOpen, (open) => {
  if (open) window.addEventListener("keydown", onViewerKey);
  else window.removeEventListener("keydown", onViewerKey);
});
onBeforeUnmount(() => window.removeEventListener("keydown", onViewerKey));

const bodyHtml = ref("");
const originalHtml = ref("");
const bodyLoading = ref(false);
const bodyTranslating = ref(false);
const showZh = ref(false);
const bodyNote = ref("");
const versions = ref<ProjectVersion[]>([]);
const versionsLoading = ref(false);

const verTypeFilter = ref<"all" | "release" | "beta" | "alpha">("all");
const verGameFilter = ref<string>("");
const verPage = ref(0);
const VER_PAGE_SIZE = 20;
const verHasMore = ref(false);
const allLoading = ref(false);

// 完整 MC 版本清单（用于「版本」tab 的筛选框列出所有版本 + 无实例时兜底最新版）
const manifestVersions = ref<{ id: string; type: string; releaseTime: string }[]>([]);
const latestMc = ref("");
let manifestLoaded = false;
async function ensureManifest() {
  if (manifestLoaded) return;
  try {
    const m = await api.getVersionManifest();
    manifestVersions.value = m.versions ?? [];
    latestMc.value = m.latest?.release ?? "";
    manifestLoaded = true;
  } catch {
    /* 清单拉取失败时回退到项目自带版本 */
  }
}

const gameVersionOptions = computed(() => {
  const seen = new Set<string>();
  const out: { label: string; value: string }[] = [];
  // 1) 清单里的全部版本（保持清单顺序：最新在前）
  for (const v of manifestVersions.value) {
    if (!v.id || seen.has(v.id)) continue;
    seen.add(v.id);
    out.push({ label: v.id, value: v.id });
  }
  // 2) 项目 / 已加载版本里出现、但清单没有的版本，追加在后
  const extra = new Set<string>(project.value?.game_versions ?? []);
  for (const v of versions.value) for (const g of v.game_versions) extra.add(g);
  for (const g of extra) {
    if (!g || seen.has(g)) continue;
    seen.add(g);
    out.push({ label: g, value: g });
  }
  return out;
});
const filteredVersions = computed(() => {
  let list = versions.value;
  if (verTypeFilter.value !== "all") list = list.filter((v) => (v.version_type ?? "") === verTypeFilter.value);
  if (verGameFilter.value) list = list.filter((v) => v.game_versions.includes(verGameFilter.value));
  return list;
});
const verPageCount = computed(() => Math.max(1, Math.ceil(filteredVersions.value.length / VER_PAGE_SIZE)));
const pagedVersions = computed(() =>
  filteredVersions.value.slice(verPage.value * VER_PAGE_SIZE, (verPage.value + 1) * VER_PAGE_SIZE),
);
watch(verTypeFilter, () => { verPage.value = 0; });
watch(verGameFilter, () => {
  verPage.value = 0;
  loadVersions();
});

const showInstall = ref(false);
// 版本列表里点「下载」时带进弹窗的版本 / 兼容实例
const installVersionId = ref<string | null>(null);
const installInstanceId = ref<string | null>(null);

/** 顶部「安装」按钮：不预选版本/实例，交给弹窗自己决定 */
function openInstall() {
  installVersionId.value = null;
  installInstanceId.value = null;
  showInstall.value = true;
}

/** 版本行「下载」按钮：预选该版本，并尽量挑一个游戏版本兼容的实例 */
function openInstallForVersion(v: ProjectVersion) {
  installVersionId.value = v.id;
  installInstanceId.value =
    instancesStore.instances.find(
      (i) => i.loader !== "vanilla" && v.game_versions.includes(i.mc_version),
    )?.id ?? null;
  showInstall.value = true;
}

/** 正文翻译的项目标识：CurseForge 的 description 接口只认数字 modId，
 *  Modrinth 的 project 接口同时接受 id 与 slug，用 slug（与服务端预热缓存同 key） */
function bodyModId(p: ProjectHit): string {
  return p.provider === "curseforge" ? p.id : p.slug || p.id;
}

/** 内置服务与自定义 AI 均已支持两平台正文翻译；百度网页模式不支持 */
const canTranslateBody = computed(
  () =>
    settingsStore.settings?.translate_provider === "default" ||
    settingsStore.settings?.translate_provider === "custom",
);

/** 消毒 + 图片链接修复：CurseForge 正文里的图片是站内相对路径，需要补全基地址；
 *  http 升级 https（混合内容会被 webview 拦截）；链接强制新窗口打开，避免 webview 就地导航 */
function sanitizeBody(html: string): string {
  const clean = DOMPurify.sanitize(html, { ADD_ATTR: ["target"] });
  const p = project.value;
  const base =
    p?.provider === "curseforge"
      ? "https://www.curseforge.com"
      : p?.provider === "spigot"
        ? "https://www.spigotmc.org"
        : "https://modrinth.com";
  const doc = new DOMParser().parseFromString(clean, "text/html");
  doc.querySelectorAll("img[src]").forEach((img) => {
    const src = img.getAttribute("src") ?? "";
    if (src.startsWith("//")) {
      img.setAttribute("src", `https:${src}`);
    } else if (src.startsWith("/") && !src.startsWith("//")) {
      img.setAttribute("src", `${base}${src}`);
    } else if (src.startsWith("http://")) {
      img.setAttribute("src", src.replace(/^http:\/\//, "https://"));
    }
  });
  doc.querySelectorAll("a[href]").forEach((a) => {
    a.setAttribute("target", "_blank");
    a.setAttribute("rel", "noopener noreferrer");
  });
  return doc.body.innerHTML;
}

/** 原文渲染：Modrinth 是 Markdown 转 HTML；CurseForge / Spigot 本身是 HTML 原样保留 */
function renderOriginal(text: string, prov: string): string {
  const html = prov === "curseforge" || prov === "spigot" ? text : (marked.parse(text, { async: false }) as string);
  return sanitizeBody(html);
}

/** 译文渲染：LLM 输出可能是 Markdown 或 HTML 混合，统一走 Markdown 解析（marked 对内嵌
 *  HTML 宽容）再消毒，避免 CF 译文按 HTML 渲染时 Markdown 符号（**、#）原样显示导致格式错乱 */
function renderTranslated(text: string): string {
  return sanitizeBody(marked.parse(text, { async: false }) as string);
}

/** 当前应渲染的内容：showZh 且有译文 → 译文，否则原文 */
const bodyDisplayHtml = computed(() => (showZh.value && bodyHtml.value ? bodyHtml.value : originalHtml.value));
const bodyIsZh = computed(() => showZh.value && !!bodyHtml.value);

async function loadProject() {
  loading.value = true;
  failed.value = false;
  project.value = null;
  // 版本筛选默认值：优先用发现页带过来的实例 MC 版本（?mc=），
  // 没有实例版本时回退到最新正式版。
  await ensureManifest();
  const mcQuery = typeof route.query.mc === "string" ? route.query.mc : "";
  const targetFilter = mcQuery || latestMc.value || "";
  try {
    project.value = await api.projectInfo(provider.value, projectId.value);
    loadBody();
    if (targetFilter && verGameFilter.value !== targetFilter) {
      // 变更筛选值会触发 watch(verGameFilter) → loadVersions()
      verGameFilter.value = targetFilter;
    } else {
      loadVersions();
    }
  } catch {
    failed.value = true;
  } finally {
    loading.value = false;
  }
}

async function loadBody() {
  const p = project.value;
  if (!p) return;
  bodyLoading.value = true;
  bodyNote.value = "";
  bodyHtml.value = "";
  originalHtml.value = "";
  showZh.value = false;
  // 仅中文用户启用翻译；外国用户默认显示原文
  const canTranslate = locale.value.startsWith("zh");
  const auto = canTranslate && !!settingsStore.settings?.body_translate_auto;
  const wantTranslate = auto && canTranslateBody.value;
  bodyTranslating.value = wantTranslate;
  try {
    const r = await api.translateBody(p.provider, bodyModId(p), wantTranslate);
    originalHtml.value = r.original ? renderOriginal(r.original, p.provider) : "";
    if (r.body && r.supported) {
      bodyHtml.value = renderTranslated(r.body);
      showZh.value = true;
    }
    if (r.error) bodyNote.value = t("projectDetail.translateFailed", { error: r.error });
  } catch {
    bodyNote.value = t("projectDetail.loadFailed");
  } finally {
    bodyLoading.value = false;
    bodyTranslating.value = false;
  }
}

/** 翻译按钮：未翻译时发起翻译；已有译文时在中/英之间切换 */
function onTranslateBtn() {
  const p = project.value;
  if (!p || bodyTranslating.value || bodyLoading.value) return;
  if (bodyHtml.value) {
    showZh.value = !showZh.value;
    return;
  }
  if (!canTranslateBody.value) {
    message.info(t("projectDetail.serviceNotSupported"));
    return;
  }
  if (!originalHtml.value) {
    message.info(t("projectDetail.noBody"));
    return;
  }
  bodyTranslating.value = true;
  api
    .translateBody(p.provider, bodyModId(p), true)
    .then((r) => {
      if (r.body) {
        bodyHtml.value = renderTranslated(r.body);
        showZh.value = true;
        bodyNote.value = "";
      } else {
        bodyNote.value = r.error ?? t("projectDetail.translateFailedShort");
      }
    })
    .catch((e) => message.error(t("projectDetail.translateFailed", { error: String(e) })))
    .finally(() => {
      bodyTranslating.value = false;
    });
}

const verApiIndex = ref(0); // 已加载到第几页（后端每页 100 条）

async function loadVersions() {
  const p = project.value;
  if (!p) return;
  versionsLoading.value = true;
  versions.value = [];
  verApiIndex.value = 0;
  try {
    const r = await api.projectVersions(p.provider, p.id, verGameFilter.value, "", 0, false);
    versions.value = r.versions;
    verHasMore.value = r.hasMore;
  } catch {
    versions.value = [];
    verHasMore.value = false;
  } finally {
    versionsLoading.value = false;
  }
}

// 只有用户手动点「加载更多」时才继续往后翻页
async function loadMore() {
  const p = project.value;
  if (!p || allLoading.value || !verHasMore.value) return;
  allLoading.value = true;
  try {
    const next = verApiIndex.value + 1;
    const r = await api.projectVersions(p.provider, p.id, verGameFilter.value, "", next, false);
    // 追加新一页，按 id 去重
    const seen = new Set(versions.value.map((v) => v.id));
    for (const v of r.versions) {
      if (!seen.has(v.id)) {
        seen.add(v.id);
        versions.value.push(v);
      }
    }
    verApiIndex.value = next;
    verHasMore.value = r.hasMore;
  } catch {
    /* 保留已加载的数据，下次点击可重试 */
  } finally {
    allLoading.value = false;
  }
}

/** 页签标题：描述 / 版本 / 图库 */
function tabLabel(k: "desc" | "versions" | "gallery"): string {
  if (k === "versions") return t("projectDetail.versions");
  if (k === "gallery") return t("projectDetail.gallery");
  return t("projectDetail.desc");
}

function versionTypeLabel(v: ProjectVersion): string {
  const ty = v.version_type ?? "";
  if (ty === "release") return t("projectDetail.release");
  if (ty === "beta") return t("projectDetail.beta");
  if (ty === "alpha") return t("projectDetail.alpha");
  return t("projectDetail.unknown");
}

function openSite() {
  const p = project.value;
  if (!p) return;
  const url = p.provider === "modrinth"
    ? `https://modrinth.com/${p.project_type}/${p.slug}`
    : `https://www.curseforge.com/minecraft/${p.project_type === "mod" ? "mc-mods" : p.project_type}/${p.slug}`;
  openUrl(url).catch(() => {});
}

onMounted(loadProject);
watch([provider, projectId], loadProject);
</script>

<template>
  <div class="pd" data-tauri-drag-region>
    <div class="toolbar glass">
      <div class="toolbar-row">
        <button class="btn ghost" @click="router.back()">
          <IconChevronLeft /> {{ t("projectDetail.back") }}
        </button>
        <button v-if="project" class="btn ghost" style="margin-left: auto;" @click="openSite">
          <IconExternal /> {{ t("projectDetail.viewOnSite") }}
        </button>
      </div>
    </div>

    <div v-if="loading" class="pd-state">{{ t("projectDetail.loading") }}</div>
    <div v-else-if="failed" class="pd-state">{{ t("projectDetail.loadFailed") }}</div>
    <template v-else-if="project">
      <div class="pd-head glass">
        <img
          v-if="project.icon_url"
          class="pd-icon"
          :src="project.icon_url"
          :alt="project.title"
          @error="($event.target as HTMLImageElement).style.display = 'none'"
        />
        <div class="pd-info">
          <h1 class="pd-title">{{ project.title }}</h1>
          <div class="pd-meta">
            <span>{{ t("projectDetail.author") }}: {{ project.author }}</span>
            <span>{{ t("projectDetail.downloads") }}: {{ fmt(project.downloads) }}</span>
            <span>{{ t("projectDetail.follows") }}: {{ fmt(project.follows) }}</span>
            <span>{{ t("projectDetail.type") }}: {{ project.project_type }}</span>
            <span>{{ t("projectDetail.updated") }}: {{ fmtDate(Date.parse(project.updated) / 1000) }}</span>
          </div>
          <p class="pd-desc">{{ project.description }}</p>
          <div class="pd-cats">
            <span v-for="c in project.categories" :key="c" class="pd-cat">{{ translateCategory(c) }}</span>
          </div>
        </div>
        <div class="pd-head-actions">
          <button class="btn primary" @click="openInstall">
            <IconDownload /> {{ t("projectDetail.install") }}
          </button>
        </div>
      </div>

      <div ref="tabBox" class="type-card glass">
        <div class="indicator" :style="tabIndicatorStyle"></div>
        <button
          v-for="k in tabKeys"
          :key="k"
          :class="{ active: tab === k }"
          @click="tab = k"
        >
          {{ tabLabel(k) }}<span v-if="k === 'gallery'" class="pd-tab-count">{{ gallery.length }}</span>
        </button>
      </div>

      <div class="pd-body glass">
        <div v-show="tab === 'desc'">
          <div v-if="canTranslateBody && locale.startsWith('zh') && (originalHtml || bodyHtml) && !bodyLoading" class="pd-body-head">
            <button
              class="pd-translate-btn"
              :class="{ on: bodyIsZh }"
              :disabled="bodyTranslating"
              @click="onTranslateBtn"
            >
              <IconGlobe />
              {{ bodyTranslating
                ? t("projectDetail.translating")
                : bodyIsZh ? t("projectDetail.showOriginal")
                : bodyHtml ? t("projectDetail.showTranslated") : t("projectDetail.translate") }}
            </button>
          </div>
          <div v-if="bodyLoading" class="pd-state">{{ t("projectDetail.loading") }}</div>
          <template v-else>
            <div v-if="bodyNote" class="pd-body-note">{{ bodyNote }}</div>
            <Transition name="pdbodyfade" mode="out-in">
              <div
                v-if="bodyDisplayHtml"
                :key="bodyIsZh ? 'zh' : 'en'"
                class="pd-prose"
                :class="{ 'pd-prose-orig': !bodyIsZh }"
                v-html="bodyDisplayHtml"
              ></div>
              <div v-else key="empty" class="pd-state">{{ t("projectDetail.noBody") }}</div>
            </Transition>
          </template>
        </div>
        <div v-show="tab === 'versions'">
          <!-- 筛选栏常驻：即使当前筛选下没有版本，也必须让用户能改/清筛选，
               不能把筛选器一起藏掉（否则就卡在空列表里出不来） -->
          <div class="pd-ver-filter">
            <button
              v-for="opt in (['all','release','beta','alpha'] as const)"
              :key="opt"
              class="pd-ver-ft"
              :class="{ active: verTypeFilter === opt }"
              @click="verTypeFilter = opt"
            >{{ opt === 'all' ? t('projectDetail.all') : versionTypeLabel({ version_type: opt } as ProjectVersion) }}</button>
            <NSelect
              v-if="gameVersionOptions.length"
              v-model:value="verGameFilter"
              :options="gameVersionOptions"
              :placeholder="t('projectDetail.gameVersion')"
              size="small"
              filterable
              clearable
              style="width: 160px; margin-left: auto;"
            />
          </div>
          <div v-if="versionsLoading" class="pd-state">{{ t("projectDetail.loading") }}</div>
          <div v-else-if="!versions.length" class="pd-state">
            {{ verGameFilter || verTypeFilter !== 'all' ? t("projectDetail.noVersionForFilter") : t("projectDetail.noVersions") }}
          </div>
          <template v-else>
            <div class="pd-ver-list">
              <div v-for="v in pagedVersions" :key="v.id" class="pd-ver">
                <div class="pd-ver-info">
                  <div class="pd-ver-main">
                    <span class="pd-ver-name">{{ v.name || v.version_number }}</span>
                    <span class="pd-ver-type" :class="v.version_type">{{ versionTypeLabel(v) }}</span>
                  </div>
                  <div class="pd-ver-sub">
                    <span v-if="v.version_number" class="pd-ver-num">{{ v.version_number }}</span>
                    <span class="pd-ver-gv">{{ v.game_versions.join(", ") }}</span>
                    <span class="pd-ver-ld">{{ v.loaders.join(", ") }}</span>
                    <span class="pd-ver-date">{{ fmtDate(Date.parse(v.date_published) / 1000) }}</span>
                  </div>
                </div>
                <button class="pd-ver-dl" :title="t('projectDetail.install')" @click="openInstallForVersion(v)">
                  <IconDownload />
                </button>
              </div>
            </div>
            <div v-if="filteredVersions.length > VER_PAGE_SIZE || verHasMore" class="pd-pager">
              <span class="pd-pager-total">{{ t("projectDetail.total", { count: filteredVersions.length }) }}</span>
              <SimplePagination
                v-if="filteredVersions.length > VER_PAGE_SIZE"
                :page="verPage + 1"
                :page-count="verPageCount"
                @update:page="verPage = $event - 1"
              />
              <button v-if="verHasMore" class="pd-load-more-btn" :disabled="allLoading" @click="loadMore">
                {{ allLoading ? t("projectDetail.loading") : t("projectDetail.loadMore") }}
              </button>
            </div>
          </template>
        </div>
        <div v-show="tab === 'gallery'">
          <div v-if="!hasGallery" class="pd-state">{{ t("projectDetail.noGallery") }}</div>
          <div v-else class="pd-gallery">
            <button
              v-for="(g, i) in gallery"
              :key="g.url"
              class="pd-gcell"
              :title="g.title || g.description || ''"
              @click="openViewer(i)"
            >
              <img
                :src="g.thumb || g.url"
                :alt="g.title || (project?.title ?? '')"
                loading="lazy"
                @error="($event.target as HTMLImageElement).style.visibility = 'hidden'"
              />
              <span v-if="g.title" class="pd-gcap">{{ g.title }}</span>
            </button>
          </div>
        </div>
      </div>
    </template>

    <InstallDialog
      v-model:show="showInstall"
      :project="project"
      :default-instance="installInstanceId"
      :default-version="installVersionId"
    />

    <!-- 图库大图查看器：Esc 关闭，左右键切换 -->
    <Teleport to="body">
      <div v-if="viewerOpen && viewerImage" class="pd-viewer" @click.self="closeViewer">
        <button class="pd-viewer-btn pd-viewer-close" :title="t('projectDetail.close')" @click="closeViewer">
          <IconClose />
        </button>
        <button
          v-if="gallery.length > 1"
          class="pd-viewer-btn pd-viewer-prev"
          @click="stepViewer(-1)"
        >
          <IconChevronLeft />
        </button>
        <button
          v-if="gallery.length > 1"
          class="pd-viewer-btn pd-viewer-next"
          @click="stepViewer(1)"
        >
          <IconChevronRight />
        </button>
        <figure class="pd-viewer-fig">
          <img :src="viewerImage.url" :alt="viewerImage.title || (project?.title ?? '')" />
          <figcaption>
            <span class="pd-viewer-count">{{ viewerIndex + 1 }} / {{ gallery.length }}</span>
            <span v-if="viewerImage.title" class="pd-viewer-title">{{ viewerImage.title }}</span>
          </figcaption>
        </figure>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.btn {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  border: none;
  border-radius: 10px;
  padding: 9px 18px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.14s;
}
.btn.primary {
  background: linear-gradient(135deg, var(--accent), var(--accent-deep));
  color: var(--accent-contrast);
}
.btn.primary:hover:not(:disabled) {
  filter: brightness(1.08);
}
.btn.ghost {
  background: var(--w-06);
  color: var(--text-1);
}
.btn.ghost:hover {
  background: var(--w-10);
}
.btn:disabled {
  opacity: 0.5;
  cursor: default;
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
.pd {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.pd-state {
  color: var(--text-3);
  font-size: 14px;
  padding: 32px 0;
  text-align: center;
}
.pd-head {
  display: flex;
  gap: 18px;
  padding: 18px 20px;
  align-items: center;
}
.pd-icon {
  width: 88px;
  height: 88px;
  border-radius: 12px;
  object-fit: cover;
  flex-shrink: 0;
}
.pd-info {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.pd-title {
  margin: 0;
  font-size: 20px;
  font-weight: 800;
  color: var(--text-1);
}
.pd-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 14px;
  font-size: 12px;
  color: var(--text-3);
}
.pd-desc {
  margin: 0;
  font-size: 13px;
  color: var(--text-2);
  line-height: 1.6;
}
.pd-cats {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.pd-cat {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 6px;
  background: var(--panel);
  color: var(--text-2);
}
.pd-head-actions {
  flex-shrink: 0;
  margin-left: auto;
}
.pd-body {
  padding: 20px 24px;
  min-height: 200px;
}
.pd-body-head {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 12px;
}
.pd-translate-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border);
  background: var(--w-04);
  color: var(--text-2);
  border-radius: 9px;
  padding: 6px 13px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.12s;
}
.pd-translate-btn:hover:not(:disabled) {
  background: var(--w-08);
  color: var(--text-1);
}
.pd-translate-btn.on {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-soft);
}
.pd-translate-btn:disabled {
  opacity: 0.6;
  cursor: default;
}
.pd-body-note {
  font-size: 12px;
  color: var(--text-3);
  margin-bottom: 10px;
}
.pdbodyfade-enter-active,
.pdbodyfade-leave-active {
  transition: opacity 0.18s ease;
}
.pdbodyfade-enter-from,
.pdbodyfade-leave-to {
  opacity: 0;
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
.pd-prose {
  color: var(--text-2);
  font-size: 14px;
  line-height: 1.75;
}
.pd-prose :deep(h1),
.pd-prose :deep(h2),
.pd-prose :deep(h3) {
  color: var(--text-1);
  margin: 1.2em 0 0.5em;
}
.pd-prose :deep(a) {
  color: var(--accent);
}
.pd-prose :deep(img) {
  max-width: 100%;
  border-radius: 8px;
}
.pd-prose :deep(code) {
  background: var(--panel);
  padding: 1px 5px;
  border-radius: 4px;
  font-size: 12px;
}
.pd-prose :deep(pre) {
  background: var(--panel);
  padding: 12px;
  border-radius: 8px;
  overflow-x: auto;
}
.pd-ver-list {
  display: flex;
  flex-direction: column;
  gap: 0;
}
.pd-ver-filter {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.pd-ver-ft {
  border: none;
  background: var(--w-06);
  color: var(--text-2);
  padding: 5px 14px;
  border-radius: 8px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: background 0.15s, color 0.15s;
}
.pd-ver-ft:hover {
  background: var(--w-10);
}
.pd-ver-ft.active {
  background: var(--accent-soft);
  color: var(--accent);
}
.pd-pager {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 16px;
}
.pd-pager-total {
  font-size: 12px;
  color: var(--text-3);
}
.pd-load-more-btn {
  border: none;
  background: var(--w-08);
  color: var(--text-1);
  padding: 6px 16px;
  border-radius: 8px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  margin-left: auto;
  transition: background 0.15s, opacity 0.15s;
}
.pd-load-more-btn:hover:not(:disabled) {
  background: var(--w-12);
}
.pd-load-more-btn:disabled {
  opacity: 0.6;
  cursor: default;
}
.pd-ver {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 0;
  border-bottom: 1px solid var(--border);
}
.pd-ver:last-child {
  border-bottom: none;
}
.pd-ver-info {
  flex: 1;
  min-width: 0;
}
.pd-ver-dl {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 8px;
  background: var(--w-08);
  color: var(--text-2);
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}
.pd-ver-dl:hover {
  background: var(--accent, #50c878);
  color: #fff;
}
.pd-ver-main {
  display: flex;
  align-items: center;
  gap: 10px;
}
.pd-ver-name {
  font-size: 14px;
  font-weight: 700;
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pd-ver-type {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 700;
  padding: 2px 8px;
  border-radius: 6px;
  background: var(--border);
  color: var(--text-3);
}
.pd-ver-type.release {
  background: rgba(76, 175, 80, 0.18);
  color: #4caf50;
}
.pd-ver-type.beta {
  background: var(--accent-12, rgba(232, 154, 75, 0.16));
  color: var(--accent);
}
.pd-ver-sub {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  margin-top: 4px;
  font-size: 11px;
  color: var(--text-3);
}
/* ---- 图库页签 ---- */
.pd-tab-count {
  margin-left: 6px;
  font-size: 11px;
  font-weight: 700;
  padding: 0 6px;
  height: 16px;
  line-height: 16px;
  border-radius: 8px;
  background: var(--w-10);
  color: var(--text-3);
}
.type-card button.active .pd-tab-count {
  background: var(--accent-soft);
  color: var(--accent);
}
.pd-gallery {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 12px;
}
.pd-gcell {
  position: relative;
  padding: 0;
  border: none;
  border-radius: 10px;
  overflow: hidden;
  cursor: zoom-in;
  background: var(--panel);
  aspect-ratio: 16 / 9;
  transition: transform 0.15s, box-shadow 0.15s;
}
.pd-gcell:hover {
  transform: translateY(-2px);
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.28);
}
.pd-gcell img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.pd-gcap {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  padding: 14px 10px 7px;
  font-size: 11px;
  font-weight: 600;
  text-align: left;
  color: #fff;
  background: linear-gradient(to top, rgba(0, 0, 0, 0.72), transparent);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* ---- 大图查看器 ---- */
.pd-viewer {
  position: fixed;
  inset: 0;
  z-index: 2000;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 56px 68px;
  background: rgba(0, 0, 0, 0.82);
  backdrop-filter: blur(3px);
  animation: pdViewerIn 0.16s ease;
}
@keyframes pdViewerIn {
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
}
.pd-viewer-fig {
  margin: 0;
  max-width: 100%;
  max-height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
}
.pd-viewer-fig img {
  max-width: 100%;
  max-height: calc(100vh - 140px);
  object-fit: contain;
  border-radius: 10px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5);
}
.pd-viewer-fig figcaption {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 12px;
  color: rgba(255, 255, 255, 0.78);
}
.pd-viewer-count {
  font-variant-numeric: tabular-nums;
}
.pd-viewer-title {
  max-width: 60vw;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pd-viewer-btn {
  position: absolute;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border: none;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.14);
  color: #fff;
  cursor: pointer;
  transition: background 0.15s;
}
.pd-viewer-btn:hover {
  background: rgba(255, 255, 255, 0.26);
}
.pd-viewer-close {
  top: 16px;
  right: 16px;
}
.pd-viewer-prev {
  left: 14px;
  top: 50%;
  transform: translateY(-50%);
}
.pd-viewer-next {
  right: 14px;
  top: 50%;
  transform: translateY(-50%);
}
</style>
