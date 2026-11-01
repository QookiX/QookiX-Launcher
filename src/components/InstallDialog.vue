<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useSettingsStore } from "../stores/settings";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useRouter } from "vue-router";
import { NModal, NSelect, NButton, useMessage } from "naive-ui";
import { api } from "../api";
import { fmtDateStr as fmtDate, instanceLabel } from "../utils/format";
import { useInstancesStore } from "../stores/instances";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import { IconCopy, IconExternal, IconGlobe } from "./icons";
import { useI18n } from "vue-i18n";
import type { ProjectDependency, ProjectHit, ProjectVersion } from "../types";

const props = defineProps<{
  show: boolean;
  project: ProjectHit | null;
  defaultInstance?: string | null;
  /** 内容详情页版本列表里点「下载」时带过来的版本 id，打开后优先选中它 */
  defaultVersion?: string | null;
}>();
const emit = defineEmits<{
  "update:show": [v: boolean];
  "install-dep": [dep: ProjectDependency];
}>();

// 点击弹窗卡片外部即关闭（用 document 委托，不依赖 naive-ui 的 mask 机制）。
// 用 mousedown 而非 click：打开弹窗的那次按下发生在 show 变 true 之前，
// 会被 `!props.show` 拦截，避免弹窗刚打开就被自身触发的点击冒泡关掉。
const cardRef = ref<HTMLElement | null>(null);
function onDocMouseDown(e: MouseEvent) {
  if (!props.show) return;
  const t = e.target as Element | null;
  if (!t) return;
  // 点击弹窗卡片内部、或 naive-ui 的下拉/弹层（teleport 到 body）都不应关闭
  if (cardRef.value?.contains(t)) return;
  if (t.closest(".v-binder-follower-container, .n-base-select-menu, .n-popover, .n-dropdown")) return;
  emit("update:show", false);
}
onMounted(() => document.addEventListener("mousedown", onDocMouseDown));
onBeforeUnmount(() => document.removeEventListener("mousedown", onDocMouseDown));

const instances = useInstancesStore();
const router = useRouter();
const message = useMessage();
const { t, locale } = useI18n();

async function copyName() {
  if (!props.project?.title) return;
  try {
    await navigator.clipboard.writeText(props.project.title);
    message.success(t('installDialog.copiedName'));
  } catch {
    message.error(t('installDialog.copyFailed'));
  }
}

const versions = ref<ProjectVersion[]>([]);
const loadingVersions = ref(false);
const selectedVersion = ref<string | null>(null);
const selectedInstance = ref<string | null>(null);
const installing = ref(false);
const installMsg = ref("");
const typeFilter = ref<"all" | "release" | "beta" | "alpha">("all");

// 版本类型 tabs 的滑动高亮指示器
const typeTabBox = ref<HTMLElement | null>(null);
const { indicatorStyle: typeTabIndicatorStyle, refresh: refreshTypeTabIndicator } = useSlidingIndicator(
  typeTabBox,
  () => Array.from(typeTabBox.value?.querySelectorAll<HTMLElement>(".id-type-tabs button") ?? []),
  () => ["all", "release", "beta", "alpha"].indexOf(typeFilter.value),
  { axis: "horizontal" }
);
watch(typeFilter, () => nextTick(() => refreshTypeTabIndicator()));
const deps = ref<ProjectDependency[]>([]);
const loadingDeps = ref(false);

const isModpack = computed(() => props.project?.project_type === "modpack");

const mcWikiUrl = ref("");
const sourceUrl = computed(() => {
  const p = props.project;
  if (!p) return "";
  if (p.provider === "modrinth") {
    return `https://modrinth.com/${p.project_type}/${p.slug}`;
  }
  if (p.provider === "curseforge") {
    const kind = p.project_type === "modpack" ? "modpacks"
      : p.project_type === "resourcepack" ? "texture-packs"
      : p.project_type === "shader" ? "shaders"
      : "mc-mods";
    return `https://www.curseforge.com/minecraft/${kind}/${p.slug}`;
  }
  return "";
});

async function loadMcWikiUrl() {
  if (!props.project) return;
  try {
    mcWikiUrl.value = await api.mcWikiUrl(props.project.title, props.project.slug, props.project.provider);
  } catch {
    mcWikiUrl.value = "";
  }
}

const instanceOptions = () =>
  instances.instances.filter((i) => i.loader !== "vanilla").map((i) => ({
    label: instanceLabel(i),
    value: i.id,
  }));

// 用户从发现页带进来的实例若是原版，它不会出现在上面的下拉里（原版没有模组加载器，
// 装不了模组/光影）。这种情况给一条明确提示，而不是让用户对着空列表发懵。
const defaultInstanceIsVanilla = computed(() => {
  if (!props.defaultInstance) return false;
  return instances.get(props.defaultInstance)?.loader === "vanilla";
});

function versionType(v: ProjectVersion): string {
  // modrinth: version_type; curseforge: release_type (1=release,2=beta,3=alpha)
  if (v.version_type) return v.version_type;
  if (v.release_type === 2) return "beta";
  if (v.release_type === 3) return "alpha";
  return "release";
}

function typeLabel(type: string) {
  return ({ release: t('installDialog.versionType.release'), beta: t('installDialog.versionType.beta'), alpha: t('installDialog.versionType.alpha') } as Record<string, string>)[type] ?? t('installDialog.versionType.release');
}

const filteredVersions = computed(() => {
  if (typeFilter.value === "all") return versions.value;
  return versions.value.filter((v) => versionType(v) === typeFilter.value);
});

// —— 缺失前置检测（与本体一起由底部「一键安装」统一安装，用户可逐项取消勾选）——
const missingDeps = ref<{ projectId: string; title: string; provider: string; versionId: string; selected: boolean }[]>([]);

// 本体是否已安装在所选实例（用于把安装按钮文案改为「已安装」）
const alreadyInstalled = ref(false);
async function checkAlreadyInstalled() {
  alreadyInstalled.value = false;
  const inst = selectedInstance.value;
  const p = props.project;
  if (!inst || !p || isModpack.value) return;
  try {
    alreadyInstalled.value = await api.isModInstalled(inst, p.slug || p.title, p.title);
  } catch { /* 忽略 */ }
}

// 按 projectId 合并声明的依赖与缺失前置，避免同一模组显示两行
const mergedDeps = computed(() => {
  type Miss = (typeof missingDeps.value)[number];
  const map = new Map<string, { projectId: string; title: string; dependencyType: string; dep: ProjectDependency | null; miss: Miss | null }>();
  for (const d of deps.value) {
    map.set(d.projectId, { projectId: d.projectId, title: d.title, dependencyType: d.dependencyType, dep: d, miss: null });
  }
  for (const m of missingDeps.value) {
    const ex = map.get(m.projectId);
    if (ex) { ex.miss = m; }
    else { map.set(m.projectId, { projectId: m.projectId, title: m.title, dependencyType: "required", dep: null, miss: m }); }
  }
  return Array.from(map.values());
});

async function loadMissing() {
  missingDeps.value = [];
  const inst = selectedInstance.value;
  const ver = selectedVersion.value;
  if (!inst || !ver || isModpack.value || !deps.value.length) return;
  if (!props.project) return;
  try {
    // 递归解析整棵前置树（含传递前置），返回未安装清单
    const tree = await api.resolveDependencyTree(
      inst,
      props.project.provider,
      props.project.id,
      ver
    );
    missingDeps.value = tree.missing
      .filter((m) => m.versionId) // 无兼容版本的装不了，不列入一键安装
      .map((m) => ({
        projectId: m.projectId,
        title: m.title,
        provider: m.provider,
        versionId: m.versionId,
        selected: true, // 默认勾选，保留原一键体验；用户可取消
      }));
  } catch {
    /* 检测失败不影响正常安装流程 */
  }
}

async function loadDeps() {
  deps.value = [];
  missingDeps.value = [];
  if (!props.project || !selectedVersion.value) return;
  loadingDeps.value = true;
  try {
    deps.value = await api.projectDependencies(props.project.provider, props.project.id, selectedVersion.value);
    await loadMissing();
  } catch {
    deps.value = [];
  } finally {
    loadingDeps.value = false;
  }
}

watch(selectedVersion, () => { if (!isModpack.value) loadDeps(); });

// ---- 描述翻译 ----
const settingsStore = useSettingsStore();
/** 当前翻译服务：default（内置）| custom（用户 AI）| baidu_web（跳转浏览器） */
const translateService = computed(() => settingsStore.settings?.translate_provider ?? "default");
/** 质量反馈接口仅内置翻译服务支持 */
const translateIsDefault = computed(() => translateService.value === "default");
const descZh = ref<string | null>(null);
const descLoading = ref(false);

/** 百度网页模式：用浏览器打开翻译页，由用户自行查看结果（不做自动抓取） */
function openBaiduTranslate() {
  const p = props.project;
  if (!p) return;
  const q = encodeURIComponent(p.description || p.title);
  openUrl(`https://fanyi.baidu.com/mtpe-individual/transText?query=${q}&lang=en2zh`).catch(
    (e: unknown) => message.error(t('installDialog.openBrowserFailed', { error: String(e) }))
  );
}

// ---- 翻译反馈面板 ----
const feedbackPanel = ref(false);
const feedbackMode = ref<"choose" | "quality">("choose");
const issueType = ref("wrong_translation");
const userSuggestion = ref("");
const userComment = ref("");
const submittingFeedback = ref(false);
const ISSUE_TYPES = computed(() => [
  { v: "wrong_translation", label: t('installDialog.issueType.wrongTranslation') },
  { v: "unnatural", label: t('installDialog.issueType.unnatural') },
  { v: "missing", label: t('installDialog.issueType.missing') },
  { v: "other", label: t('installDialog.issueType.other') },
]);

watch(
  () => [props.show, props.project?.id, props.project?.provider] as const,
  async ([show, id, provider]) => {
    descZh.value = null;
    feedbackPanel.value = false;
    const custom = translateService.value === "custom";
    // 百度网页模式不请求翻译；内置服务只支持 Modrinth；自定义 AI 两个平台都能翻
    if (!show || !id || !provider || translateService.value === "baidu_web" || !locale.value.startsWith("zh")) return;
    if (provider !== "modrinth" && !custom) return;
    descLoading.value = true;
    try {
      const r = await api.translateDescriptions(provider, [id]);
      descZh.value = r.translations[id] ?? null;
    } catch {
      descZh.value = null;
    } finally {
      descLoading.value = false;
    }
  }
);

function toggleFeedbackPanel() {
  feedbackPanel.value = !feedbackPanel.value;
  feedbackMode.value = "choose";
}

async function reportStale() {
  const p = props.project;
  if (!p) return;
  try {
    const status = await api.reportStaleTranslation(p.provider, p.slug);
    message.success(t('installDialog.thanksForFeedback'));
    feedbackPanel.value = false;
    if (status === "updated") {
      const r = await api.translateDescriptions(p.provider, [p.slug]);
      descZh.value = r.translations[p.slug] ?? null;
    }
  } catch (e) {
    message.error(t('installDialog.feedbackFailed', { error: String(e) }));
  }
}

async function submitQuality() {
  const p = props.project;
  if (!p || submittingFeedback.value) return;
  submittingFeedback.value = true;
  try {
    await api.reportQualityFeedback(
      p.provider,
      p.slug,
      issueType.value,
      userSuggestion.value,
      userComment.value
    );
    message.success(t('installDialog.thanksForFeedback'));
    feedbackPanel.value = false;
    feedbackMode.value = "choose";
    issueType.value = "wrong_translation";
    userSuggestion.value = "";
    userComment.value = "";
  } catch (e) {
    message.error(t('installDialog.feedbackFailed', { error: String(e) }));
  } finally {
    submittingFeedback.value = false;
  }
}

async function loadVersions() {
  if (!props.project) return;
  loadingVersions.value = true;
  selectedVersion.value = null;
  deps.value = [];
  try {
    // 已选实例时用实例的 MC 版本 + 加载器请求上游，让 Modrinth/CurseForge
    // 只返回兼容的版本，从而 API 本身返回更少的数据（而非全量拉取后本地过滤）
    const inst = selectedInstance.value ? instances.get(selectedInstance.value) : null;
    const mc = inst?.mc_version ?? "";
    const ld = inst && inst.loader !== "vanilla" ? inst.loader : "";
    const res = await api.projectVersions(props.project.provider, props.project.id, mc, ld, 0, false);
    versions.value = res.versions;
    if (inst && !res.versions.length) {
      // 上游按实例筛选无结果时，回退拉取全部版本供选择
      const all = await api.projectVersions(props.project.provider, props.project.id, "", "", 0, false);
      versions.value = all.versions;
    }
    // 详情页版本行「下载」带进来的版本优先选中；不在当前（按实例过滤后的）列表里
    // 就退回默认的「第一个正式版」
    const preferred = props.defaultVersion
      ? versions.value.find((v) => v.id === props.defaultVersion)
      : undefined;
    const picked = preferred ?? versions.value.find((v) => versionType(v) === "release") ?? versions.value[0];
    if (picked) selectedVersion.value = picked.id;
  } catch (e) {
    message.error(String(e));
  } finally {
    loadingVersions.value = false;
  }
}

// 切换实例时按新实例的 MC 版本/加载器重新拉取
watch(selectedInstance, () => {
  if (props.show && props.project) loadVersions();
  checkAlreadyInstalled();
});

function resetForProject() {
  if (!props.project) return;
  const nonVanilla = instances.instances.filter((i) => i.loader !== "vanilla");
  const pref = props.defaultInstance && nonVanilla.some((i) => i.id === props.defaultInstance)
    ? props.defaultInstance
    : nonVanilla[0]?.id ?? null;
  selectedInstance.value = isModpack.value ? null : pref;
  installMsg.value = "";
  typeFilter.value = "all";
  loadVersions();
  loadMcWikiUrl();
  checkAlreadyInstalled();
}

async function onOpen() {
  if (!props.show) return;
  api.logDebug(
    `[fe] 对话框打开 project=${props.project?.provider}/${props.project?.id} type=${props.project?.project_type}`
  );
  resetForProject();
}

// 弹窗内切换项目（点击前置依赖）时重新加载版本与依赖
watch(
  () => props.project,
  (proj, oldProj) => {
    if (proj && oldProj && proj !== oldProj) resetForProject();
  }
);

function depLabel(type: string) {
  return ({ required: t('installDialog.deps.required'), optional: t('installDialog.deps.optional'), incompatible: t('installDialog.deps.incompatible'), embedded: t('installDialog.deps.embedded') } as Record<string, string>)[type] ?? type;
}

async function install() {
  if (!props.project) return;
  api.logDebug(
    `[fe] install() 进入 provider=${props.project.provider} id=${props.project.id} type=${props.project.project_type} isModpack=${isModpack.value} versions=${versions.value.length}`
  );
  if (!isModpack.value && !selectedInstance.value) {
    api.logDebug("[fe] 拦截：未选择实例");
    message.warning(t('installDialog.selectInstanceWarn'));
    return;
  }
  if (!selectedVersion.value) {
    api.logDebug(`[fe] 拦截：未选择版本 selectedVersion=${String(selectedVersion.value)}`);
    message.warning(versions.value.length ? t('installDialog.selectVersionWarn') : t('installDialog.noAvailableVersion'));
    return;
  }
  api.logDebug(
    `[fe] 发起 invoke instanceId=${String(selectedInstance.value)} version=${String(selectedVersion.value)} kind=${props.project.project_type}`
  );
  installing.value = true;
  // 缺失前置与本体一起入队（前置在前，游戏加载时前置需先就位）；仅装用户勾选的前置
  const missing = missingDeps.value.filter((m) => m.selected);
  const total = missing.length + 1;
  message.success(
    isModpack.value
      ? t('installDialog.installStarted')
      : missing.length
        ? t('installDialog.installingWithDeps', { count: missing.length, total })
        : t('installDialog.addedToQueue')
  );
  // 整合包安装是长任务（下载整包 + 逐个拉取 mod 元数据 + 装游戏本体），
  // 不阻塞对话框——立即关闭，进度与成败都通过 install://progress 事件进下载中心。
  const queue = missing.map((m) =>
    api
      .installContent(selectedInstance.value ?? "", m.provider, m.projectId, m.versionId, "mod")
      .then((r) => api.logDebug(`[fe] 前置 ${m.title} 返回成功 ${JSON.stringify(r)}`))
      .catch((e) => {
        api.logDebug(`[fe] 前置 ${m.title} 返回失败 ${String(e)}`);
        message.error(t('installDialog.depInstallFailed', { title: m.title, error: String(e) }));
      })
  );
  queue.push(
    api
      .installContent(
        selectedInstance.value ?? "",
        props.project.provider,
        props.project.id,
        selectedVersion.value,
        props.project.project_type
      )
      .then((r) => {
        api.logDebug(`[fe] 返回成功 ${JSON.stringify(r)}`);
        if (!isModpack.value) message.success(t('installDialog.installDone'));
      })
      .catch((e) => {
        api.logDebug(`[fe] 返回失败 ${String(e)}`);
        message.error(String(e));
      })
  );
  Promise.all(queue)
    .then(() => {
      // 全部入队成功后清掉缺失提示；单项失败的话提示已单独弹出
      missingDeps.value = [];
    })
    .finally(() => {
      installing.value = false;
    });
  emit("update:show", false);
}

</script>

<template>
  <n-modal
    :show="props.show"
    preset="card"
    :title="props.project?.title ?? t('installDialog.installContent')"
    style="width: 680px; max-width: 94vw"
    :mask-closable="true"
    :close-on-esc="true"
    @update:show="(v: boolean) => emit('update:show', v)"
    @mask-click="() => emit('update:show', false)"
    @after-enter="onOpen"
  >
    <div v-if="props.project" ref="cardRef" class="id-modal">
      <div class="id-head">
        <img v-if="props.project.icon_url" :src="props.project.icon_url" class="id-icon" alt="" />
        <div class="id-info">
          <div class="id-title">{{ props.project.title }}</div>
          <div class="id-meta">
            <span class="id-author">{{ props.project.author }}</span>
            <span class="id-dl">{{ t('installDialog.downloads', { wan: (props.project.downloads / 10000).toFixed(1), count: props.project.downloads }) }}</span>
            <span class="id-type">{{ props.project.project_type }}</span>
          </div>
          <div class="id-desc" :class="{ expanded: !!descZh }">
            <template v-if="descZh">
              <div class="id-desc-zh">{{ descZh }}</div>
              <div class="id-desc-en">{{ props.project.description }}</div>
              <button
                v-if="descZh && translateIsDefault"
                class="id-desc-feedback"
                @click="toggleFeedbackPanel"
              >
                {{ feedbackPanel ? t('installDialog.feedback.collapse') : t('installDialog.feedback.reportIssue') }}
              </button>
              <button
                v-if="translateService === 'baidu_web'"
                class="id-desc-feedback"
                @click="openBaiduTranslate"
              >{{ t('installDialog.feedback.openBaidu') }}</button>
              <div v-if="feedbackPanel" class="id-fb-panel">
                <template v-if="feedbackMode === 'choose'">
                  <button class="id-fb-opt" @click="reportStale">{{ t('installDialog.feedback.stale') }}</button>
                  <button class="id-fb-opt" @click="feedbackMode = 'quality'">{{ t('installDialog.feedback.quality') }}</button>
                </template>
                <template v-else>
                  <div class="id-fb-types">
                    <button
                      v-for="issue in ISSUE_TYPES"
                      :key="issue.v"
                      class="id-fb-type"
                      :class="{ on: issueType === issue.v }"
                      @click="issueType = issue.v"
                    >{{ issue.label }}</button>
                  </div>
                  <textarea
                    v-model="userSuggestion"
                    class="id-fb-input"
                    rows="2"
                    maxlength="160"
                    :placeholder="t('installDialog.feedback.suggestionPlaceholder')"
                  ></textarea>
                  <textarea
                    v-model="userComment"
                    class="id-fb-input"
                    rows="2"
                    maxlength="160"
                    :placeholder="t('installDialog.feedback.commentPlaceholder')"
                  ></textarea>
                  <div class="id-fb-actions">
                    <button class="id-fb-back" @click="feedbackMode = 'choose'">{{ t('installDialog.feedback.back') }}</button>
                    <button class="id-fb-submit" :disabled="submittingFeedback" @click="submitQuality">
                      {{ submittingFeedback ? t('installDialog.feedback.submitting') : t('installDialog.feedback.submit') }}
                    </button>
                  </div>
                </template>
              </div>
            </template>
            <template v-else>{{ props.project.description }}</template>
          </div>
          <div class="id-links">
            <a v-if="mcWikiUrl" :href="mcWikiUrl" target="_blank" class="id-link"><IconGlobe /> {{ t('installDialog.links.mcWiki') }}</a>
            <a v-if="sourceUrl" :href="sourceUrl" target="_blank" class="id-link"><IconExternal /> {{ t('installDialog.links.openInBrowser') }}</a>
            <button class="id-link" @click="copyName"><IconCopy /> {{ t('installDialog.links.copyName') }}</button>
          </div>
        </div>
      </div>

      <div class="id-form">
        <label v-if="!isModpack" class="id-field">
          <span>{{ t('installDialog.form.installToInstance') }}</span>
          <n-select v-model:value="selectedInstance" :options="instanceOptions()" :placeholder="t('installDialog.form.selectInstance')" />
        </label>
        <div v-if="!isModpack && defaultInstanceIsVanilla" class="id-vanilla-hint">
          {{ t('installDialog.form.vanillaHint') }}
        </div>

        <div v-if="isModpack" class="id-field">
          <span class="id-modpack-hint">{{ t('installDialog.form.modpackHint') }}</span>
        </div>

        <div class="id-field">
          <div class="id-ver-head">
            <span>{{ t('installDialog.form.selectVersion') }}</span>
            <div ref="typeTabBox" class="id-type-tabs">
              <div class="indicator" :style="typeTabIndicatorStyle"></div>
              <button :class="{ active: typeFilter === 'all' }" @click="typeFilter = 'all'">{{ t('installDialog.form.typeAll') }}</button>
              <button :class="{ active: typeFilter === 'release' }" @click="typeFilter = 'release'">{{ t('installDialog.versionType.release') }}</button>
              <button :class="{ active: typeFilter === 'beta' }" @click="typeFilter = 'beta'">{{ t('installDialog.versionType.beta') }}</button>
              <button :class="{ active: typeFilter === 'alpha' }" @click="typeFilter = 'alpha'">{{ t('installDialog.versionType.alpha') }}</button>
            </div>
          </div>
          <div v-if="loadingVersions" class="id-loading">{{ t('installDialog.loading') }}</div>
          <div v-else class="id-ver-list">
            <button
              v-for="v in filteredVersions.slice(0, 80)"
              :key="v.id"
              class="id-ver-row"
              :class="{ active: selectedVersion === v.id }"
              @click="selectedVersion = v.id"
            >
              <span class="id-ver-num mono">{{ v.version_number }}</span>
              <span class="id-ver-type" :class="versionType(v)">{{ typeLabel(versionType(v)) }}</span>
              <span class="id-ver-mc">{{ (v.game_versions ?? []).slice(-2).join(", ") }}</span>
              <span class="id-ver-date">{{ fmtDate(v.date_published) }}</span>
            </button>
            <div v-if="!filteredVersions.length" class="id-empty">{{ t('installDialog.noVersionInCategory') }}</div>
          </div>
        </div>

        <!-- dependencies -->
        <div v-if="(deps.length || loadingDeps) && !isModpack" class="id-deps">
          <div class="id-deps-head">
            <span class="id-deps-label">{{ t('installDialog.deps.label') }}</span>
            <span v-if="missingDeps.length && !loadingDeps" class="id-deps-hint">{{ t('installDialog.deps.missingHint') }}</span>
          </div>
          <div v-if="!loadingDeps" class="id-deps-list">
            <button
              v-for="d in mergedDeps"
              :key="d.projectId"
              class="id-dep-row"
              :class="[d.dependencyType, { miss: d.miss, off: d.miss && !d.miss.selected }]"
              :title="t('installDialog.deps.viewDetail', { title: d.title })"
              @click="emit('install-dep', d.dep ?? { projectId: d.projectId, title: d.title, slug: d.title, dependencyType: d.dependencyType })"
            >
              <span v-if="d.miss" class="id-check" :class="{ checked: d.miss.selected }" :title="d.miss.selected ? t('installDialog.deps.willInstall') : t('installDialog.deps.skipInstall')" @click.stop="d.miss.selected = !d.miss.selected"><svg viewBox="0 0 12 12" width="12" height="12"><path d="M2 6.5 L4.8 9 L10 3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg></span>
              <span class="id-dep-row-title">{{ d.title }}</span>
              <span v-if="!d.miss" class="id-dep-row-tag installed">{{ t('installDialog.deps.installed') }}</span>
              <span class="id-dep-row-tag">{{ depLabel(d.dependencyType) }}</span>
            </button>
          </div>
          <div v-if="loadingDeps" class="id-deps-loading">{{ t('installDialog.deps.loading') }}</div>
        </div>
      </div>

      <div v-if="installMsg" class="id-msg">{{ installMsg }}</div>
      <div v-if="!isModpack && !instances.instances.length" class="id-noinst">
        {{ t('installDialog.noInstance') }}<a @click="emit('update:show', false); router.push('/instances')">{{ t('installDialog.createInstance') }}</a>
      </div>

      <!-- 操作按钮放在内容区内（而非 #footer）：
           版本列表较长时会把窗口撑高，footer 会跑到视口外点不到。 -->
      <div class="id-footer">
        <n-button @click="emit('update:show', false)">{{ t('installDialog.close') }}</n-button>
        <n-button
          type="primary"
          :loading="installing"
          :disabled="alreadyInstalled"
          @click="
            api.logDebug('[fe] 一键安装按钮被点击');
            install();
          "
        >
          {{ alreadyInstalled ? t('installDialog.alreadyInstalled') : t('installDialog.install') }}
        </n-button>
      </div>
    </div>
  </n-modal>
</template>

<style>
/* modal content is teleported to <body>; keep styles global */
.id-modal {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.id-head {
  display: flex;
  gap: 14px;
  align-items: flex-start;
}
.id-icon {
  width: 56px;
  height: 56px;
  border-radius: 12px;
  object-fit: cover;
  flex-shrink: 0;
  background: var(--panel);
}
.id-title {
  font-size: 16px;
  font-weight: 700;
  margin-bottom: 4px;
}
.id-meta {
  display: flex;
  gap: 10px;
  font-size: 12px;
  color: var(--text-3);
  margin-bottom: 6px;
}
.id-type {
  background: var(--accent-soft);
  color: var(--accent);
  border-radius: 6px;
  padding: 0 7px;
  font-weight: 600;
}
.id-desc {
  font-size: 12px;
  color: var(--text-3);
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.id-links {
  display: flex;
  gap: 8px;
  margin-top: 6px;
}
.id-link {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--text-3);
  background: var(--panel);
  border: 1px solid var(--border);
  border-radius: 7px;
  padding: 3px 9px;
  text-decoration: none;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.12s;
}
.id-link:hover {
  color: var(--accent, #e89a4b);
  border-color: var(--accent-45);
}
.id-site {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border: 1px solid var(--border);
  background: var(--panel);
  color: var(--text-2);
  border-radius: 8px;
  padding: 6px 11px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  flex-shrink: 0;
}
.id-site:hover {
  color: var(--accent);
  border-color: var(--accent-45);
}
.id-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.id-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
  color: var(--text-2);
}
.id-modpack-hint {
  color: var(--accent, #50c878);
  font-weight: 600;
  font-size: 13px;
}
.id-vanilla-hint {
  margin-top: -4px;
  padding: 8px 10px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--warning, #e0a400) 14%, transparent);
  border: 1px solid color-mix(in srgb, var(--warning, #e0a400) 35%, transparent);
  color: var(--text-1, #ddd);
  font-size: 12px;
  line-height: 1.5;
}
.id-ver-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.id-type-tabs {
  position: relative;
  display: flex;
  gap: 3px;
  background: var(--panel);
  border-radius: 8px;
  padding: 2px;
}
.id-type-tabs .indicator {
  position: absolute;
  top: 2px;
  bottom: 2px;
  border-radius: 6px;
  background: var(--accent-soft);
  pointer-events: none;
}
.id-type-tabs button {
  border: none;
  background: transparent;
  color: var(--text-3);
  padding: 4px 10px;
  border-radius: 6px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
}
.id-type-tabs button.active {
  color: var(--accent);
}
.id-loading {
  padding: 20px;
  text-align: center;
  color: var(--text-3);
  font-size: 13px;
}
.id-ver-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  max-height: 200px;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 6px;
  background: var(--panel);
}
.id-ver-row {
  display: flex;
  align-items: center;
  gap: 8px;
  border: 1px solid transparent;
  background: transparent;
  color: var(--text-2);
  padding: 6px 10px;
  border-radius: 8px;
  cursor: pointer;
  font-family: inherit;
  text-align: left;
}
.id-ver-row:hover {
  background: var(--panel);
}
.id-ver-row.active {
  border-color: var(--accent-05);
  background: var(--accent-soft);
}
.id-ver-num {
  font-size: 12px;
  font-weight: 600;
  min-width: 0;
  flex-shrink: 1;
}
.id-ver-type {
  font-size: 10px;
  padding: 1px 7px;
  border-radius: 6px;
  font-weight: 600;
  flex-shrink: 0;
}
.id-ver-type.release {
  background: rgba(78, 201, 160, 0.14);
  color: var(--success);
}
.id-ver-type.beta {
  background: var(--accent-soft);
  color: var(--accent);
}
.id-ver-type.alpha {
  background: var(--danger-14);
  color: var(--danger);
}
.id-ver-mc {
  font-size: 11px;
  color: var(--text-3);
  flex: 1;
  text-align: right;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.id-ver-date {
  font-size: 11px;
  color: var(--text-3);
  flex-shrink: 0;
}
.id-empty {
  text-align: center;
  color: var(--text-3);
  font-size: 13px;
  padding: 16px 0;
}
.id-deps {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.id-deps-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex-wrap: wrap;
}
.id-deps-label {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-3);
}
.id-deps-hint {
  font-size: 11px;
  color: var(--text-3);
  opacity: 0.85;
}
.id-deps-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.id-dep-row {
  display: flex;
  align-items: center;
  gap: 10px;
  border: 1px solid var(--border);
  background: var(--panel);
  border-radius: 10px;
  padding: 9px 12px;
  font-size: 13px;
  cursor: pointer;
  font-family: inherit;
  color: var(--text-1);
  transition: border-color 0.15s, background 0.15s;
  text-align: left;
  width: 100%;
}
.id-dep-row:hover {
  border-color: var(--accent-45);
}
.id-dep-row-title {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.id-dep-row-tag {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 700;
  padding: 2px 8px;
  border-radius: 6px;
  background: var(--border);
  color: var(--text-3);
}
.id-dep-row.required .id-dep-row-tag {
  background: var(--danger-16);
  color: var(--danger);
}
.id-dep-row.optional .id-dep-row-tag {
  background: var(--info-15);
  color: var(--text-2);
}
.id-dep-row.incompatible .id-dep-row-tag {
  background: var(--border);
  color: var(--text-3);
}
.id-dep-row .id-dep-row-tag.installed {
  background: rgba(76, 175, 80, 0.18);
  color: #4caf50;
}
.id-check {
  flex-shrink: 0;
  width: 18px;
  height: 18px;
  border-radius: 5px;
  border: 1.5px solid var(--border);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  transition: background 0.15s, border-color 0.15s;
}
.id-check svg {
  opacity: 0;
  transition: opacity 0.12s;
}
.id-check.checked {
  background: var(--accent);
  border-color: var(--accent);
}
.id-check.checked svg {
  opacity: 1;
}
.id-dep-row.miss.off {
  opacity: 0.55;
}
.id-dep-row.miss.off .id-dep-row-title {
  color: var(--text-3);
}
.id-deps-loading {
  font-size: 12px;
  color: var(--text-3);
}
.id-msg {
  font-size: 13px;
  color: var(--accent);
}
.id-noinst {
  font-size: 12px;
  color: var(--text-3);
}
.id-noinst a {
  color: var(--accent);
  cursor: pointer;
}
.id-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}
.id-desc-zh {
  color: var(--text-1);
}
/* 中英对照模式放开 2 行截断，长描述完整展示 */
.id-desc.expanded {
  display: block;
  -webkit-line-clamp: unset;
  overflow: visible;
}
.id-desc-en {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-3);
}
.id-desc-feedback {
  margin-top: 6px;
  background: none;
  border: none;
  color: var(--text-3);
  font-size: 12px;
  cursor: pointer;
  padding: 0;
  text-decoration: underline dotted;
  text-underline-offset: 3px;
}
.id-desc-feedback:hover {
  color: var(--text-1);
}
.id-desc-feedback:disabled {
  cursor: default;
  opacity: 0.6;
}

/* ---- 翻译反馈面板 ---- */
.id-fb-panel {
  margin-top: 8px;
  padding: 10px 12px;
  border: 1px solid var(--w-10);
  border-radius: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.id-fb-opt {
  text-align: left;
  background: var(--w-04);
  border: none;
  border-radius: 8px;
  color: var(--text-2);
  font-size: 12px;
  padding: 7px 10px;
  cursor: pointer;
}
.id-fb-opt:hover {
  background: var(--w-08);
  color: var(--text-1);
}
.id-fb-types {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.id-fb-type {
  background: var(--w-04);
  border: none;
  border-radius: 14px;
  color: var(--text-2);
  font-size: 12px;
  padding: 4px 10px;
  cursor: pointer;
}
.id-fb-type.on {
  background: var(--accent-14);
  color: var(--accent);
}
.id-fb-input {
  background: var(--w-04);
  border: 1px solid var(--w-08);
  border-radius: 8px;
  color: var(--text-1);
  font-size: 12px;
  line-height: 1.5;
  padding: 6px 8px;
  resize: none;
  font-family: inherit;
}
.id-fb-input:focus {
  outline: none;
  border-color: var(--accent-35);
}
.id-fb-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.id-fb-back {
  background: none;
  border: none;
  color: var(--text-3);
  font-size: 12px;
  cursor: pointer;
  padding: 4px 6px;
}
.id-fb-back:hover {
  color: var(--text-1);
}
.id-fb-submit {
  background: var(--accent-14);
  color: var(--accent);
  border: none;
  border-radius: 8px;
  font-size: 12px;
  padding: 5px 12px;
  cursor: pointer;
}
.id-fb-submit:hover {
  background: var(--accent-22);
}
.id-fb-submit:disabled {
  opacity: 0.6;
  cursor: default;
}
</style>
