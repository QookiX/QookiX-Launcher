<script setup lang="ts">
import { vSegmented } from "../../composables/vSegmented";
import { computed, nextTick, onMounted, ref, watch } from "vue";
import { NTooltip, NCollapseTransition, useMessage } from "naive-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useSettingsStore } from "../../stores/settings";
import { useSlidingIndicator } from "../../composables/useSlidingIndicator";
import { useOnboarding } from "../../composables/useOnboarding";
import {
  IconExternal,
  IconFile,
  IconGithub,
  IconGlobe,
  IconHeart,
  IconList,
  IconUsers,
  IconShield,
  IconBookOpen,
} from "../../components/icons";
import {
  peekUpdate,
  isUpdateDownloaded,
  forceUpdateInfo,
  loadChangelogReleases,
  releasesInRange,
  entryTypeClass,
  type ChangelogRelease,
  type UpdateOffer,
} from "../../updater";
import devWeimoshengUrl from "../../assets/dev-weimosheng.jpg";
import devZhayiUrl from "../../assets/dev-zhayi.jpg";
import AboutShowcase from "../../components/AboutShowcase.vue";
import DiagnosticsDialog from "../../components/DiagnosticsDialog.vue";
import { useI18n } from "vue-i18n";
import { version as appVersion } from "../../../package.json";

import { api } from "../../api";
import type { QookixServiceStatus } from "../../types";
import SettingsUpdateDialog from "./SettingsUpdateDialog.vue";

const props = defineProps<{ changelogRevision: number }>();
const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();
const onboarding = useOnboarding();

const checking = ref(false);
const showDiag = ref(false);

const updateOffer = ref<UpdateOffer | null>(null);
const updateVersion = computed(() => updateOffer.value?.version ?? null);
const updateCurrentVersion = computed(() => updateOffer.value?.currentVersion ?? null);
const updateModalVisible = ref(false);

const changelogReleases = ref<ChangelogRelease[]>([]);
const changelogLoading = ref(true);
const expandedVersions = ref<Set<string>>(new Set());

/**
 * 「检查更新」弹窗里展示的更新内容：当前版本 → 最新版本之间跨过的所有版本
 * （受 changelog.json 自身覆盖范围限制，更旧的版本拿不到就省略）。
 * 渲染走共享组件 UpdateNotes，与启动时自动检查的弹窗保持同一套样式。
 */
const updateNotes = computed(() =>
  releasesInRange(changelogReleases.value, updateCurrentVersion.value, updateVersion.value)
);

function toggleVersion(version: string) {
  const s = new Set(expandedVersions.value);
  if (s.has(version)) s.delete(version);
  else s.add(version);
  expandedVersions.value = s;
}

async function loadChangelog() {
  changelogLoading.value = true;
  changelogReleases.value = await loadChangelogReleases();
  if (changelogReleases.value.length > 0) {
    expandedVersions.value = new Set([changelogReleases.value[0].version]);
  }
  changelogLoading.value = false;
}

async function checkUpdate() {
  if (checking.value) return;
  checking.value = true;
  updateOffer.value = null;
  try {
    const update = await peekUpdate(true);
    if (!update) {
      message.success(t("settings.update.upToDate"));
      return;
    }
    // 强制更新：当前版本低于 minVersion 时激活全屏锁定遮罩
    if (update.forceUpdate) {
      forceUpdateInfo.value = update;
      return;
    }
    updateOffer.value = update;
    // 更早版本可能还没加载完（首屏就点检查更新）：先补上再算跨度
    if (changelogReleases.value.length === 0) await loadChangelog();
    // 该版本已经下载好、只等重启：不必再弹一次下载确认框
    if (isUpdateDownloaded(update)) {
      message.info(t("settings.update.downloadedRestart", { version: update.version }));
      return;
    }
    updateModalVisible.value = true;
  } catch {
    // 国内镜像不可用时给出明确提示，而不是误报「已是最新版本」
    const isBucket = settings.settings?.update_source !== "github";
    message.error(
      isBucket
        ? t("settings.update.mirrorError")
        : t("settings.update.checkFailed")
    );
  } finally {
    checking.value = false;
  }
}

/** 恢复被「忽略此版本」关掉的启动更新提醒。 */
async function restoreDismissed() {
  try {
    await settings.patch({ dismissed_update_version: null });
    message.success(t("settings.update.restored"));
  } catch {
    message.error(t("settings.update.restoreFailed"));
  }
}

// 更新源 seg 滑动高亮（国内镜像 / GitHub 官方）
const updateSourceSegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: updateSourceSegStyle, refresh: refreshUpdateSourceSeg } =
  useSlidingIndicator(
    updateSourceSegRef,
    () => Array.from(updateSourceSegRef.value?.querySelectorAll<HTMLElement>(".seg button") ?? []),
    () => (settings.settings?.update_source === "github" ? 1 : 0),
    { axis: "horizontal" }
  );
watch(() => settings.settings?.update_source, () => nextTick(() => refreshUpdateSourceSeg()));

const aboutDeps: Record<
  "frontend" | "rust" | "thirdparty",
  { name: string; version: string; license: string; url: string; licenseUrl: string }[]
> = {
  frontend: [
    { name: "Vue", version: "3.5", license: "MIT", url: "https://vuejs.org", licenseUrl: "https://github.com/vuejs/core/blob/main/LICENSE" },
    { name: "Vue Router", version: "4.4", license: "MIT", url: "https://router.vuejs.org", licenseUrl: "https://github.com/vuejs/router/blob/main/LICENSE" },
    { name: "Naive UI", version: "2.45", license: "MIT", url: "https://www.naiveui.com", licenseUrl: "https://github.com/tusen-design/naive-ui/blob/main/LICENSE" },
    { name: "Pinia", version: "2.2", license: "MIT", url: "https://pinia.vuejs.org", licenseUrl: "https://github.com/vuejs/pinia/blob/v2/LICENSE" },
    { name: "Tauri API", version: "2", license: "MIT/Apache-2.0", url: "https://tauri.app", licenseUrl: "https://github.com/tauri-apps/tauri/blob/dev/LICENSE" },
    { name: "driver.js", version: "1.8", license: "MIT", url: "https://driverjs.com", licenseUrl: "https://github.com/driverjs/driver.js/blob/main/LICENSE" },
    { name: "marked", version: "18.0", license: "MIT", url: "https://marked.js.org", licenseUrl: "https://github.com/markedjs/marked/blob/master/LICENSE.md" },
    { name: "DOMPurify", version: "3.4", license: "Apache-2.0/MPL-2.0", url: "https://github.com/cure53/DOMPurify", licenseUrl: "https://github.com/cure53/DOMPurify/blob/main/LICENSE" },
    { name: "skinview3d", version: "3.4", license: "MIT", url: "https://github.com/bs-community/skinview3d", licenseUrl: "https://github.com/bs-community/skinview3d/blob/master/LICENSE" },
  ],
  rust: [
    { name: "Tauri", version: "2", license: "MIT/Apache-2.0", url: "https://tauri.app", licenseUrl: "https://github.com/tauri-apps/tauri/blob/dev/LICENSE" },
    { name: "Tokio", version: "1", license: "MIT", url: "https://tokio.rs", licenseUrl: "https://github.com/tokio-rs/tokio/blob/master/LICENSE" },
    { name: "reqwest", version: "0.12", license: "MIT/Apache-2.0", url: "https://github.com/seanmonstar/reqwest", licenseUrl: "https://github.com/seanmonstar/reqwest/blob/master/LICENSE" },
    { name: "serde", version: "1", license: "MIT/Apache-2.0", url: "https://serde.rs", licenseUrl: "https://github.com/serde-rs/serde/blob/master/LICENSE" },
    { name: "fastnbt", version: "2", license: "MIT", url: "https://github.com/owengage/fastnbt", licenseUrl: "https://github.com/owengage/fastnbt/blob/main/LICENSE" },
    { name: "cc", version: "1.0", license: "MIT/Apache-2.0", url: "https://github.com/rust-lang/cc-rs", licenseUrl: "https://github.com/rust-lang/cc-rs/blob/main/LICENSE" },
  ],
  thirdparty: [
    { name: "Terracotta", version: "", license: "AGPL-3.0-or-later", url: "https://github.com/burningtnt/Terracotta", licenseUrl: "https://github.com/burningtnt/Terracotta/blob/master/LICENSE" },
    { name: "Feather Icons", version: "", license: "MIT", url: "https://feathericons.com", licenseUrl: "https://github.com/feathericons/feather/blob/main/LICENSE" },
    { name: "cubiomes", version: "", license: "MIT", url: "https://github.com/Cubitect/cubiomes", licenseUrl: "https://github.com/Cubitect/cubiomes/blob/master/LICENSE" },
  ],
};

// ---- 服务状态（公开接口，不需要登录）----
const svcStatus = ref<QookixServiceStatus | null>(null);
const svcLoading = ref(false);
const svcState = computed<"ok" | "warn" | "bad" | "unknown">(() => {
  const s = svcStatus.value;
  if (!s) return "unknown";
  if (s.online === false) return "bad";
  return s.state === "degraded" ? "warn" : "ok";
});
const svcText = computed(() => {
  const s = svcStatus.value;
  if (!s) return t("qookix.serviceStatus.unknown");
  if (s.online === false) return t("qookix.serviceStatus.unknown");
  return s.state === "degraded"
    ? t("qookix.serviceStatus.degraded")
    : t("qookix.serviceStatus.operational");
});
const svcDetail = computed(() => {
  const s = svcStatus.value;
  if (!s) return "";
  const parts: string[] = [s.worker?.running ? t("qookix.serviceStatus.workerOn") : t("qookix.serviceStatus.workerOff")];
  const chars = s.today?.chars ?? 0;
  const mods = s.today?.mods ?? 0;
  if (chars > 0 || mods > 0) {
    parts.push(t("qookix.serviceStatus.today", { chars: chars.toLocaleString(), mods }));
  }
  return parts.join(" · ");
});

async function loadServiceStatus() {
  if (svcLoading.value) return;
  svcLoading.value = true;
  try {
    svcStatus.value = await api.qookixServiceStatus();
  } catch {
    svcStatus.value = null;
  } finally {
    svcLoading.value = false;
  }
}
onMounted(loadServiceStatus);

onMounted(loadChangelog);
watch(() => props.changelogRevision, loadChangelog);
</script>

<template>
  <div v-if="settings.settings" class="settings-body">
    <div class="settings-pane">
      <div class="card glass about-showcase" id="about-showcase">
        <AboutShowcase />
        <div class="about-hero-title">
          <span class="about-name about-hero-name">QookiX Launcher</span>
          <span class="about-ver">v{{ appVersion }}</span>
        </div>
        <p class="about-hero-slogan">{{ t("settings.about.slogan") }}</p>
      </div>
      <div class="grid about-grid">
        <div class="card glass about-card" id="about-devs">
          <div class="about-devs-title">{{ t("settings.about.developers") }}</div>
          <div class="dev-list">
            <div class="dev-line">
              <img class="dev-avatar" :src="devWeimoshengUrl" alt="维墨笙" />
              <div class="dev-meta">
                <div class="dev-head">
                  <span class="dev-name">维墨笙</span>
                  <n-tooltip trigger="hover" placement="top">
                    <template #trigger>
                      <button class="dev-github-btn" @click="openUrl('https://github.com/weimosheng')">
                        <IconGithub />
                      </button>
                    </template>
                    {{ t("settings.about.githubHome") }}
                  </n-tooltip>
                </div>
                <span class="dev-role">{{ t("settings.about.devRole") }}</span>
              </div>
            </div>
            <div class="dev-line">
              <img class="dev-avatar" :src="devZhayiUrl" alt="ZhaYi" />
              <div class="dev-meta">
                <div class="dev-head">
                  <span class="dev-name">ZhaYi</span>
                  <n-tooltip trigger="hover" placement="top">
                    <template #trigger>
                      <button class="dev-github-btn" @click="openUrl('https://github.com/ZhaYi-Miao')">
                        <IconGithub />
                      </button>
                    </template>
                    {{ t("settings.about.githubHome") }}
                  </n-tooltip>
                </div>
                <span class="dev-role">{{ t("settings.about.collaborator") }}</span>
              </div>
            </div>
          </div>
        </div>
        <div class="card glass about-update-card" id="about-update">
          <div class="about-update">
            <button class="mini-btn" @click="showDiag = true">{{ t("settings.about.diagReport") }}</button>
            <button class="mini-btn primary" :disabled="checking" @click="checkUpdate">
              {{ checking ? t("settings.about.checking") : t("settings.about.checkUpdate") }}
            </button>
            <span v-if="updateVersion" class="hint-inline">
              {{ t("settings.about.newVersionFound", { version: updateVersion }) }}
            </span>
            <button
              v-if="settings.settings?.dismissed_update_version"
              class="mini-btn"
              @click="restoreDismissed"
            >
              {{ t("settings.about.restoreUpdate", { version: settings.settings.dismissed_update_version }) }}
            </button>
          </div>
          <div class="update-source">
            <div class="choice-info">
              <span class="choice-label">{{ t("settings.about.updateSource") }}</span>
              <p class="choice-hint">{{ t("settings.about.updateSourceHint") }}</p>
            </div>
            <div ref="updateSourceSegRef" v-segmented="{ label: t('settings.about.updateSource') }" class="seg qx-seg qx-seg--sm">
              <div class="qx-seg__indicator" :style="updateSourceSegStyle"></div>
              <button
                :class="{ active: settings.settings.update_source !== 'github' }"
                @click="settings.patch({ update_source: 'bucket' })"
              >
                {{ t("settings.about.mirrorCN") }}
              </button>
              <button
                :class="{ active: settings.settings.update_source === 'github' }"
                @click="settings.patch({ update_source: 'github' })"
              >
                {{ t("settings.about.githubOfficial") }}
              </button>
            </div>
          </div>
        </div>
        <div class="card glass about-status-card" id="about-service-status">
          <h3>{{ t("qookix.serviceStatus.title") }}</h3>
          <div class="svc-row">
            <span class="svc-dot" :class="svcState"></span>
            <span class="svc-text">{{ svcText }}</span>
            <button class="mini-btn" :disabled="svcLoading" @click="loadServiceStatus">
              {{ t("qookix.serviceStatus.refresh") }}
            </button>
          </div>
          <p v-if="svcDetail" class="license-text">{{ svcDetail }}</p>
        </div>
        <div class="card glass about-coop-card" id="about-coop">
          <h3>{{ t("settings.about.coopTitle") }}</h3>
          <p class="license-text">
            {{ t("settings.about.coopLead") }}
            <span class="license-accent">{{ t("settings.about.coopAccent") }}</span>
            {{ t("settings.about.coopTail") }}
          </p>
          <button class="about-link" @click="openUrl('https://www.mcimg.com/')">
            <span class="link-left"><IconGlobe /> {{ t("settings.about.coopVisit") }}</span>
            <span class="link-arrow">→</span>
          </button>
        </div>
        <div class="about-links-row">
          <button class="about-link" @click="openUrl('https://qookix.cn/')">
            <span class="link-left"><IconGlobe /> {{ t("settings.about.website") }}</span>
            <span class="link-arrow">→</span>
          </button>
          <button class="about-link" @click="openUrl('https://github.com/weimosheng/QookiX-Launcher')">
            <span class="link-left"><IconGithub /> {{ t("settings.about.githubRepo") }}</span>
            <span class="link-arrow">→</span>
          </button>
          <button class="about-link" @click="openUrl('https://github.com/weimosheng/QookiX-Launcher/issues')">
            <span class="link-left"><IconExternal /> {{ t("settings.about.feedback") }}</span>
            <span class="link-arrow">→</span>
          </button>
          <button class="about-link" @click="openUrl('https://github.com/weimosheng/QookiX-Launcher/releases')">
            <span class="link-left"><IconList /> {{ t("settings.about.changelog") }}</span>
            <span class="link-arrow">→</span>
          </button>
          <button class="about-link" @click="openUrl('https://qm.qq.com/q/91keQnJ8dy')">
            <span class="link-left"><IconUsers /> {{ t("settings.about.qqGroup") }}</span>
            <span class="link-arrow">→</span>
          </button>
          <button class="about-link" @click="openUrl('https://afdian.com/a/weimosheng')">
            <span class="link-left"><IconHeart /> {{ t("settings.about.sponsor") }}</span>
            <span class="link-arrow">→</span>
          </button>
        </div>
        <div class="card glass about-changelog-card" id="about-changelog">
          <h3>{{ t("settings.about.changelogTitle") }}</h3>
          <div v-if="changelogLoading" class="changelog-loading">{{ t("settings.about.changelogLoading") }}</div>
          <div v-else class="cl-releases">
            <div
              v-for="r in changelogReleases"
              :key="r.version"
              class="cl-release"
              :class="{ expanded: expandedVersions.has(r.version) }"
            >
              <button class="cl-release-header" :aria-expanded="expandedVersions.has(r.version)" @click="toggleVersion(r.version)">
                <span class="cl-version">v{{ r.version }}</span>
                <span class="cl-date">{{ r.date }}</span>
                <span class="cl-count">{{ r.entries.length }}</span>
                <span class="cl-arrow">▾</span>
              </button>
              <NCollapseTransition :show="expandedVersions.has(r.version)">
                <div class="cl-release-body">
                  <div v-for="(e, i) in r.entries" :key="i" class="cl-entry">
                    <span class="cl-tag" :class="entryTypeClass(e.text)">{{ e.category }}</span>
                    <span class="cl-text">{{ e.text }}</span>
                  </div>
                </div>
              </NCollapseTransition>
            </div>
          </div>
        </div>
        <div class="card glass about-license-card" id="about-license">
          <h3>{{ t("settings.about.license") }}</h3>
          <p class="license-text">
            {{ t("settings.about.licenseLead") }}
            <span class="license-accent">{{ t("settings.about.licenseAccent") }}</span>
            {{ t("settings.about.licenseTail") }}
          </p>
          <p class="legal-privacy-note">{{ t("settings.about.legalPrivacyNote") }}</p>
          <button class="about-link" @click="openUrl('https://github.com/weimosheng/QookiX-Launcher/blob/main/LICENSE')">
            <span class="link-left"><IconFile /> {{ t("settings.about.viewLicense") }}</span>
            <span class="link-arrow">→</span>
          </button>
          <button class="about-link" @click="openUrl('https://docs.qookix.cn/agreement.html')">
            <span class="link-left"><IconShield /> {{ t("settings.about.agreement") }}</span>
            <span class="link-arrow">→</span>
          </button>
        </div>
        <div class="card glass about-replay-card" id="about-onboarding">
          <div class="about-replay-info">
            <div class="about-replay-title">{{ t("settings.about.onboarding") }}</div>
            <p class="about-replay-desc">{{ t("settings.about.onboardingDesc") }}</p>
          </div>
          <button class="mini-btn primary" @click="onboarding.open">
            <IconBookOpen /> {{ t("settings.about.replayOnboarding") }}
          </button>
        </div>
        <div class="card glass about-deps-card" id="about-deps">
          <h3>{{ t("settings.about.depsTitle") }}</h3>
          <p class="license-text">{{ t("settings.about.depsIntro") }}</p>
          <div class="deps-groups">
            <div class="deps-group">
              <div class="deps-group-title">{{ t("settings.about.depsFrontend") }}</div>
              <div v-for="d in aboutDeps.frontend" :key="d.name" class="about-dep-row">
                <div class="dep-info">
                  <span class="dep-name">{{ d.name }}<span class="dep-ver" v-if="d.version">v{{ d.version }}</span></span>
                  <span class="dep-license">{{ d.license }}</span>
                </div>
                <div class="dep-links">
                  <button class="dep-link" @click="openUrl(d.url)">{{ t("settings.about.source") }}</button>
                  <button class="dep-link" @click="openUrl(d.licenseUrl)">{{ t("settings.about.licenseLabel") }}</button>
                </div>
              </div>
            </div>
            <div class="deps-group">
              <div class="deps-group-title">Rust</div>
              <div v-for="d in aboutDeps.rust" :key="d.name" class="about-dep-row">
                <div class="dep-info">
                  <span class="dep-name">{{ d.name }}<span class="dep-ver" v-if="d.version">v{{ d.version }}</span></span>
                  <span class="dep-license">{{ d.license }}</span>
                </div>
                <div class="dep-links">
                  <button class="dep-link" @click="openUrl(d.url)">{{ t("settings.about.source") }}</button>
                  <button class="dep-link" @click="openUrl(d.licenseUrl)">{{ t("settings.about.licenseLabel") }}</button>
                </div>
              </div>
            </div>
            <div class="deps-group">
              <div class="deps-group-title">{{ t("settings.about.depsThirdparty") }}</div>
              <div v-for="d in aboutDeps.thirdparty" :key="d.name" class="about-dep-row">
                <div class="dep-info">
                  <span class="dep-name">{{ d.name }}<span class="dep-ver" v-if="d.version">v{{ d.version }}</span></span>
                  <span class="dep-license">{{ d.license }}</span>
                </div>
                <div class="dep-links">
                  <button class="dep-link" @click="openUrl(d.url)">{{ t("settings.about.source") }}</button>
                  <button class="dep-link" @click="openUrl(d.licenseUrl)">{{ t("settings.about.licenseLabel") }}</button>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
    <DiagnosticsDialog v-model:show="showDiag" />
    <SettingsUpdateDialog v-model:show="updateModalVisible" :offer="updateOffer" :notes="updateNotes" />
  </div>
</template>
