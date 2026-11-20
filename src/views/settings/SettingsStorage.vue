<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { fmtSize, fmtTime } from "../../utils/format";
import { useMessage, useDialog } from "naive-ui";
import { api } from "../../api";
import { IconGithub, IconRefresh, IconTrash } from "../../components/icons";
import type { StorageStats } from "../../types";
import { IconCloud } from "../../components/icons";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const message = useMessage();
const dialog = useDialog();
const emit = defineEmits<{ openCloud: []; openAccounts: [] }>();

// ---------------------------------------------------------------------------
// 存储统计
// ---------------------------------------------------------------------------
const stats = ref<StorageStats | null>(null);
const loadingStats = ref(false);
const clearing = ref(false);

const DONUT_COLORS: Record<string, string> = {
  instances: "var(--accent)",
  servers: "#f59e0b",
  libraries: "#5aa2f0",
  assets: "#4ec9a0",
  versions: "#8b5cf6",
  runtime: "#ec4899",
  logs: "#94a3b8",
  other: "#64748b",
  launcher: "#22d3ee",
};
const DONUT_R = 74;
const DONUT_C = 2 * Math.PI * DONUT_R;

function pct(size: number): number {
  if (!stats.value || !stats.value.total) return 0;
  return Math.round((size / stats.value.total) * 1000) / 10;
}

const visibleCats = computed(() => (stats.value?.categories ?? []).filter((c) => c.size > 0));

/** 环形饼图各分段：start 偏移累积，返回 dash/offset 供 SVG stroke-dasharray 使用 */
const donutSegs = computed(() => {
  const total = stats.value?.total ?? 0;
  if (!total) return [];
  let acc = 0;
  return visibleCats.value.map((c) => {
    const frac = c.size / total;
    const dash = Math.max(0, frac * DONUT_C - 1.5);
    const seg = { key: c.key, dash, offset: -acc, color: DONUT_COLORS[c.key] ?? "#64748b" };
    acc += frac * DONUT_C;
    return seg;
  });
});

async function loadStats() {
  loadingStats.value = true;
  try {
    stats.value = await api.getStorageStats();
  } catch (e) {
    message.error(t("settings.storage.loadFailed", { error: String(e) }));
  } finally {
    loadingStats.value = false;
  }
}

async function refreshStats(showConfirmation = true) {
  loadingStats.value = true;
  try {
    stats.value = await api.refreshStorageStats();
    if (showConfirmation) message.success(t("settings.storage.refreshed"));
  } catch (e) {
    message.error(t("settings.storage.refreshFailed", { error: String(e) }));
  } finally {
    loadingStats.value = false;
  }
}

function confirmClear() {
  dialog.warning({
    title: t("settings.storage.clearTitle"),
    content: t("settings.storage.clearContent"),
    positiveText: t("settings.storage.clearCache"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      clearing.value = true;
      try {
        const res = await api.clearCache();
        const summary = { size: fmtSize(res.freed) };
        if (res.java_download_in_use || res.errors.length) {
          const details = [
            ...(res.java_download_in_use ? [t("settings.storage.javaDownloadInUse")] : []),
            ...res.errors,
          ];
          message.warning(t("settings.storage.clearPartial", { ...summary, details: details.join("\n") }));
        } else {
          message.success(t("settings.storage.clearSuccess", summary));
        }
        await refreshStats(false);
      } catch (e) {
        message.error(t("settings.storage.clearFailed", { error: String(e) }));
      } finally {
        clearing.value = false;
      }
    },
  });
}

onMounted(loadStats);
</script>

<template>
  <div class="settings-body">
    <div class="settings-pane">
      <div class="grid storage-grid">
        <div class="card glass storage-card" id="storage-cloud">
          <div class="storage-header">
            <h3>{{ t("settings.storage.cloudSave") }}</h3>
            <div class="storage-actions">
              <span class="hint-inline">{{ t("settings.storage.cloudHint") }}</span>
              <button class="mini-btn" @click="emit('openCloud')">
                <IconCloud class="btn-icon" />
                {{ t("settings.storage.browseCloud") }}
              </button>
            </div>
          </div>

          <!-- GitHub 账户已统一到「账号」页管理，这里只留入口 -->
          <p class="gh-moved">
            <span>{{ t("settings.storage.githubMoved") }}</span>
            <button class="mini-btn" @click="emit('openAccounts')">
              <IconGithub class="btn-icon" />{{ t("settings.tab.accounts") }}
            </button>
          </p>
        </div>

        <div class="card glass storage-card" id="storage-stats" :aria-busy="loadingStats">
          <div class="storage-header">
            <h3>{{ t("settings.storage.stats") }}</h3>
            <div class="storage-actions">
              <span class="hint-inline">
                <template v-if="stats">{{ t("settings.storage.updatedAt", { label: stats.cached ? t("settings.storage.lastUpdate") : t("settings.storage.updated"), time: fmtTime(stats.updated_at) }) }}</template>
                <template v-else>{{ t("settings.storage.notScanned") }}</template>
              </span>
              <button class="mini-btn" :disabled="loadingStats" @click="refreshStats()">
                <IconRefresh class="btn-icon" />
                {{ loadingStats ? t("settings.storage.scanning") : t("settings.storage.refresh") }}
              </button>
            </div>
          </div>

          <div v-if="stats && stats.total > 0" class="storage-body">
            <div class="donut-wrap">
              <svg viewBox="0 0 200 200" class="donut">
                <circle
                  v-for="seg in donutSegs"
                  :key="seg.key"
                  cx="100"
                  cy="100"
                  r="74"
                  fill="none"
                  :stroke="seg.color"
                  stroke-width="30"
                  :stroke-dasharray="`${seg.dash} ${DONUT_C - seg.dash}`"
                  :stroke-dashoffset="seg.offset"
                  transform="rotate(-90 100 100)"
                />
              </svg>
              <div class="donut-center">
                <span class="donut-total">{{ fmtSize(stats.total) }}</span>
                <span class="donut-label">{{ t("settings.storage.totalUsed") }}</span>
              </div>
            </div>

            <ul class="storage-legend">
              <li v-for="cat in visibleCats" :key="cat.key">
                <span class="legend-dot" :style="{ background: DONUT_COLORS[cat.key] ?? '#64748b' }"></span>
                <span class="legend-name">{{ cat.label }}</span>
                <span class="legend-size">{{ fmtSize(cat.size) }}</span>
                <span class="legend-pct">{{ pct(cat.size) }}%</span>
              </li>
            </ul>
          </div>

          <div v-if="stats && stats.instances.length" class="instance-storage">
            <h4 class="instance-storage-title">
              {{ t("settings.storage.perInstance") }}
              <span class="hint-inline">{{ t("settings.storage.count", { count: stats.instances.length }) }}</span>
            </h4>
            <ul class="instance-storage-list">
              <li v-for="inst in stats.instances" :key="inst.id">
                <span class="instance-name" :title="inst.name">{{ inst.name }}</span>
                <span class="legend-size">{{ fmtSize(inst.size) }}</span>
                <span class="legend-pct">{{ pct(inst.size) }}%</span>
              </li>
            </ul>
          </div>

          <div v-if="stats && stats.servers.length" class="instance-storage">
            <h4 class="instance-storage-title">
              {{ t("settings.storage.perServer") }}
              <span class="hint-inline">{{ t("settings.storage.count", { count: stats.servers.length }) }}</span>
            </h4>
            <ul class="instance-storage-list">
              <li v-for="srv in stats.servers" :key="srv.id">
                <span class="instance-name" :title="srv.name">{{ srv.name }}</span>
                <span class="legend-size">{{ fmtSize(srv.size) }}</span>
                <span class="legend-pct">{{ pct(srv.size) }}%</span>
              </li>
            </ul>
          </div>
          <p v-else-if="!stats?.instances.length" class="hint">{{ stats ? t("settings.storage.noData") : t("settings.storage.loading") }}</p>

          <div class="storage-footer">
            <button class="mini-btn danger" :disabled="clearing" @click="confirmClear">
              <IconTrash class="btn-icon" />
              {{ clearing ? t("settings.storage.clearing") : t("settings.storage.clearCache") }}
            </button>
            <span class="hint">{{ t("settings.storage.clearHint") }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
