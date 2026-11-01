<script setup lang="ts">
import { ref, computed } from "vue";
import { useI18n } from "vue-i18n";
import { NModal, NButton, useMessage } from "naive-ui";
import { api } from "../api";
import type { ProjectHit, ProjectVersion, Instance } from "../types";
import { IconDownload } from "./icons";
import VersionPicker from "./VersionPicker.vue";

const props = defineProps<{
  show: boolean;
  project: ProjectHit | null;
  instance: Instance | null;
}>();
const emit = defineEmits<{ "update:show": [v: boolean] }>();

const { t } = useI18n();
const message = useMessage();

interface DepItem {
  projectId: string;
  title: string;
  slug: string;
  dependencyType: string;
  versions: ProjectVersion[];
  versionId: string | null;
  declaredVersionId: string;
  installed: boolean;
  selected: boolean;
  versionsLoaded: boolean;
  loading: boolean;
}

const loading = ref(false);
const installing = ref(false);
const mainVersions = ref<ProjectVersion[]>([]);
const mainVersionId = ref<string | null>(null);
const mainInstalled = ref(false);
const deps = ref<DepItem[]>([]);
const installedIds = ref<Set<string>>(new Set());

function normId(s: string) {
  return s.toLowerCase().replace(/[^a-z0-9]/g, "");
}

/** 默认选最新稳定版；没有则降级到测试版 → 先行版 → 任意 */
function pickDefault(vers: ProjectVersion[]): string | null {
  if (!vers.length) return null;
  for (const ty of ["release", "beta", "alpha"]) {
    const v = vers.find((x) => (x.version_type ?? "release") === ty);
    if (v) return v.id;
  }
  return vers[0].id;
}

async function loadAll() {
  if (!props.project || !props.instance) return;
  const proj = props.project;
  const inst = props.instance;
  loading.value = true;
  mainVersions.value = [];
  mainVersionId.value = null;
  mainInstalled.value = false;
  deps.value = [];
  try {
    const mc = inst.mc_version;
    const ld = inst.loader !== "vanilla" ? inst.loader : "";
    const [res, ids] = await Promise.all([
      api.projectVersions(proj.provider, proj.id, mc, ld, 0, false),
      api.listInstalledModIds(inst.id),
    ]);
    mainVersions.value = res.versions;
    mainVersionId.value = pickDefault(res.versions);
    installedIds.value = new Set(ids);
    mainInstalled.value =
      installedIds.value.has(normId(proj.slug)) || installedIds.value.has(normId(proj.title));
  } catch (e) {
    message.error(String(e));
  } finally {
    loading.value = false;
  }
  // 前置依赖在后台加载，不阻塞清单显示
  loadDeps();
}

async function loadDeps() {
  if (!props.project || !mainVersionId.value) {
    deps.value = [];
    return;
  }
  const proj = props.project;
  const inst = props.instance;
  if (!inst) return;
  try {
    const raw = await api.projectDependencies(proj.provider, proj.id, mainVersionId.value);
    // 只展示必需 / 可选前置（内嵌已打进本体，不兼容是冲突）
    const list = raw.filter(
      (d) => d.dependencyType === "required" || d.dependencyType === "optional",
    );
    deps.value = list.map((d) => {
      const installed =
        installedIds.value.has(normId(d.slug)) || installedIds.value.has(normId(d.title));
      const vid = d.versionId || null;
      return {
        projectId: d.projectId,
        title: d.title,
        slug: d.slug,
        dependencyType: d.dependencyType,
        versions: [],
        // 有声明版本就直接用，没有的等后台拉版本列表再选默认值
        versionId: vid,
        declaredVersionId: d.versionId ?? "",
        installed,
        // 已安装的前置默认不勾选（没必要重复下载）
        selected: !installed,
        versionsLoaded: false,
        loading: false,
      };
    });
    // 没有声明版本的前置：后台非阻塞拉版本列表选默认值，不卡清单显示
    for (const item of deps.value) {
      if (!item.versionId) loadDepVersions(item);
    }
  } catch (e) {
    message.error(String(e));
  }
}

/** 懒加载某个前置的版本列表（用户打开下拉时才调） */
async function loadDepVersions(item: DepItem) {
  if (item.versionsLoaded || item.loading) return;
  const proj = props.project;
  const inst = props.instance;
  if (!proj || !inst) return;
  item.loading = true;
  try {
    const mc = inst.mc_version;
    const ld = inst.loader !== "vanilla" ? inst.loader : "";
    const res = await api.projectVersions(proj.provider, item.projectId, mc, ld, 0, false);
    item.versions = res.versions;
    // 若声明的版本不在列表里，退回最新稳定版
    const declared = item.declaredVersionId
      ? res.versions.find((v) => v.id === item.declaredVersionId)
      : undefined;
    if (!item.versionId || !res.versions.some((v) => v.id === item.versionId)) {
      item.versionId = declared ? declared.id : pickDefault(res.versions);
    }
    item.versionsLoaded = true;
  } catch {
    /* 单个前置版本拉取失败不阻塞整体 */
  }
  item.loading = false;
}

/** 主版本被用户切换时重新拉前置 */
function onMainVersionChange() {
  loadDeps();
}

const toInstallDeps = computed(() =>
  deps.value.filter((d) => d.selected && d.versionId && !d.installed),
);

async function confirm() {
  if (!props.project || !props.instance || !mainVersionId.value) return;
  const inst = props.instance;
  const proj = props.project;
  installing.value = true;
  const toInstall = toInstallDeps.value;
  const total = toInstall.length + 1;
  message.success(
    toInstall.length
      ? t("checklist.installingWithDeps", { count: toInstall.length, total })
      : t("checklist.addedToQueue"),
  );
  // 前置先入队（加载时需先就位），本体在后
  const queue = toInstall.map((d) =>
    api
      .installContent(inst.id, proj.provider, d.projectId, d.versionId as string, "mod")
      .catch((e) =>
        message.error(t("checklist.depInstallFailed", { title: d.title, error: String(e) })),
      ),
  );
  queue.push(
    api
      .installContent(inst.id, proj.provider, proj.id, mainVersionId.value, proj.project_type)
      .then(() => message.success(t("checklist.installDone")))
      .catch((e) => message.error(String(e))),
  );
  await Promise.all(queue);
  installing.value = false;
  emit("update:show", false);
}
</script>

<template>
  <n-modal
    :show="props.show"
    preset="card"
    :title="t('checklist.title')"
    style="width: 580px; max-width: 94vw"
    :mask-closable="true"
    :close-on-esc="true"
    @update:show="(v: boolean) => emit('update:show', v)"
    @after-enter="loadAll"
  >
    <div v-if="props.project && props.instance" class="ic-modal">
      <div class="ic-target">{{ t("checklist.installTo", { name: props.instance.name }) }}</div>
      <div v-if="loading" class="ic-loading">{{ t("checklist.loading") }}</div>
      <template v-else>
        <!-- 本体 -->
        <div class="ic-row ic-main">
          <img
            v-if="props.project.icon_url"
            :src="props.project.icon_url"
            class="ic-icon"
            alt=""
          />
          <div v-else class="ic-icon ic-icon-ph" />
          <div class="ic-row-info">
            <div class="ic-row-title">
              <span class="ic-tag ic-main-tag">{{ t("checklist.main") }}</span>
              <span class="ic-name">{{ props.project.title }}</span>
              <span v-if="mainInstalled" class="ic-tag ic-installed-tag">{{ t("checklist.installed") }}</span>
            </div>
            <div class="ic-row-sub">{{ props.project.author }}</div>
          </div>
          <VersionPicker
            v-model="mainVersionId"
            :versions="mainVersions"
            :placeholder="t('checklist.selectVersion')"
            style="flex-shrink: 0"
            @update:model-value="onMainVersionChange"
          />
        </div>

        <!-- 前置依赖 -->
        <div v-if="deps.length" class="ic-deps">
          <div class="ic-deps-head">{{ t("checklist.depsLabel") }}</div>
          <div
            v-for="d in deps"
            :key="d.projectId"
            class="ic-row ic-dep"
            :class="{ off: !d.selected }"
          >
            <label class="ic-check" :class="{ checked: d.selected }" :title="d.title">
              <input v-model="d.selected" type="checkbox" class="ic-check-input" />
              <svg viewBox="0 0 12 12" width="13" height="13"><path d="M2 6.5 L4.8 9 L10 3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>
            </label>
            <div class="ic-row-info">
              <div class="ic-row-title">
                <span class="ic-tag ic-dep-tag">{{ t("checklist.depTag") }}</span>
                <span v-if="d.installed" class="ic-tag ic-installed-tag">{{ t("checklist.installed") }}</span>
                <span class="ic-name">{{ d.title }}</span>
              </div>
              <div v-if="d.loading" class="ic-row-sub">{{ t("checklist.loading") }}</div>
              <div v-else-if="d.versionsLoaded && !d.versions.length" class="ic-row-sub ic-warn">{{ t("checklist.noVersion") }}</div>
            </div>
            <VersionPicker
              v-model="d.versionId"
              :versions="d.versions"
              :loading="d.loading"
              :placeholder="t('checklist.selectVersion')"
              :disabled="!d.selected"
              style="flex-shrink: 0"
              @open="loadDepVersions(d)"
            />
          </div>
        </div>

        <div v-if="!deps.length" class="ic-no-deps">{{ t("checklist.noDeps") }}</div>
      </template>

      <div class="ic-footer">
        <n-button @click="emit('update:show', false)">{{ t("checklist.cancel") }}</n-button>
        <n-button
          type="primary"
          :loading="installing"
          :disabled="loading || !mainVersionId || mainInstalled"
          @click="confirm"
        >
          <template #icon><IconDownload /></template>
          {{ mainInstalled ? t("checklist.installed") : t("checklist.confirm") }}
        </n-button>
      </div>
    </div>
  </n-modal>
</template>

<style>
/* modal 内容 teleport 到 body，样式保持全局 */
.ic-modal {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.ic-target {
  font-size: 12px;
  color: var(--text-2);
  padding: 0 2px;
}
.ic-loading {
  padding: 40px 0;
  text-align: center;
  color: var(--text-3);
  font-size: 13px;
}
.ic-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  background: var(--w-04, rgba(255, 255, 255, 0.03));
}
.ic-main {
  background: var(--w-08, rgba(255, 255, 255, 0.06));
}
.ic-icon {
  width: 36px;
  height: 36px;
  border-radius: 8px;
  object-fit: cover;
  flex-shrink: 0;
}
.ic-icon-ph {
  background: var(--w-12, rgba(255, 255, 255, 0.08));
}
.ic-row-info {
  flex: 1;
  min-width: 0;
}
.ic-row-title {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
}
.ic-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ic-row-sub {
  margin-top: 2px;
  font-size: 11px;
  color: var(--text-3);
  font-weight: 400;
}
.ic-warn {
  color: #e8a04b;
}
.ic-tag {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 700;
  padding: 2px 7px;
  border-radius: 5px;
  line-height: 1.4;
}
.ic-main-tag {
  background: rgba(76, 175, 80, 0.18);
  color: #4caf50;
}
.ic-dep-tag {
  background: rgba(232, 154, 75, 0.16);
  color: #e8a04b;
}
.ic-installed-tag {
  background: rgba(33, 150, 243, 0.16);
  color: #2196f3;
}
.ic-deps {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.ic-deps-head {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-2);
  padding-left: 2px;
}
.ic-dep {
  padding: 8px 12px;
}
.ic-dep.off {
  opacity: 0.5;
}
.ic-check {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: 1.5px solid var(--border);
  border-radius: 5px;
  cursor: pointer;
  color: #fff;
  transition: background 0.12s, border-color 0.12s;
}
.ic-check.checked {
  background: #4caf50;
  border-color: #4caf50;
}
.ic-check-input {
  display: none;
}
.ic-check svg {
  opacity: 0;
}
.ic-check.checked svg {
  opacity: 1;
}
.ic-no-deps {
  font-size: 12px;
  color: var(--text-3);
  text-align: center;
  padding: 6px 0;
}
.ic-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 4px;
}
</style>
