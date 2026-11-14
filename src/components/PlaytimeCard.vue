<script setup lang="ts">
/**
 * 游玩时长统计卡片（游戏库页顶部）：
 * 总时长 + 近 30 天柱状图 + 游玩时间最长的实例 Top3。
 */
import { computed, onMounted, ref } from "vue";
import { api } from "../api";
import { fmtDuration } from "../utils/format";
import type { PlaytimeStats } from "../types";
import { useI18n } from "vue-i18n";

const { t } = useI18n();

const stats = ref<PlaytimeStats | null>(null);
const loading = ref(false);

onMounted(async () => {
  loading.value = true;
  try {
    stats.value = await api.playtimeStats();
  } catch {
    stats.value = null;
  } finally {
    loading.value = false;
  }
});

const top3 = computed(() => (stats.value?.byInstance ?? []).filter((i) => i.seconds > 0).slice(0, 3));

/** 近 30 天柱高（0~1），按最大值归一 */
const bars = computed(() => {
  const days = stats.value?.byDay ?? [];
  const max = Math.max(...days.map((d) => d.seconds), 1);
  return days.map((d) => ({
    day: d.day,
    seconds: d.seconds,
    h: Math.max(d.seconds > 0 ? 6 : 2, Math.round((d.seconds / max) * 42)),
    title: d.seconds > 0 ? `${fmtDuration(d.seconds)}` : t("playtimeCard.none"),
  }));
});

/** day 索引 → "M/D" 标签（只给最后一天和有记录的第一天用） */
function dayLabel(day: number): string {
  const d = new Date((day * 86400 - 8 * 3600) * 1000);
  return `${d.getMonth() + 1}/${d.getDate()}`;
}
</script>

<template>
  <div class="pt-card glass">
    <div class="pt-total">
      <div class="pt-label">{{ t("playtimeCard.totalLabel") }}</div>
      <div class="pt-value">{{ loading ? "…" : fmtDuration(stats?.totalSeconds ?? 0) }}</div>
    </div>
    <div v-if="bars.length" class="pt-chart" :title="t('playtimeCard.chartTitle')">
      <div class="pt-bars">
        <div
          v-for="b in bars"
          :key="b.day"
          class="pt-bar"
          :class="{ 'is-empty': !b.seconds }"
          :style="{ height: b.h + 'px' }"
          :title="b.title"
        ></div>
      </div>
      <div class="pt-range">
        <span>{{ dayLabel(bars[0].day) }}</span>
        <span>{{ dayLabel(bars[bars.length - 1].day) }}</span>
      </div>
    </div>
    <div v-if="top3.length" class="pt-top">
      <div class="pt-label">{{ t("playtimeCard.topLabel") }}</div>
      <div class="pt-top-list">
        <div v-for="t in top3" :key="t.id" class="pt-top-row">
          <span class="pt-top-name text-ellipsis">{{ t.name }}</span>
          <span class="pt-top-time">{{ fmtDuration(t.seconds) }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.pt-card {
  display: flex;
  align-items: center;
  gap: 32px;
  padding: 14px 20px;
  flex-wrap: wrap;
}
.pt-label {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  margin-bottom: 4px;
}
.pt-total {
  min-width: 110px;
}
.pt-value {
  font-size: var(--fs-xl);
  font-weight: var(--fw-bold);
  color: var(--accent);
}
.pt-chart {
  /* 不设宽度上限：卡片是整行通栏的，限到 420 会在中间留出一大片空白，
     柱子也被压成一条细线。让它吃掉剩余宽度，柱距才是正常柱状图的比例。 */
  flex: 1;
  min-width: 220px;
}
.pt-bars {
  display: flex;
  align-items: flex-end;
  gap: 3px;
  height: 48px;
  /* 基线：让柱子落在可见的轨道上，而不是悬空 */
  border-bottom: 1px solid var(--border);
  padding: 0 2px;
}
.pt-bar {
  flex: 1;
  min-width: 4px;
  border-radius: 2px 2px 0 0;
  /* 有记录的日子用实打实可见的柱体，没记录的日子只留一条浅轨道。
     原来统一用 --accent-04（4% 透明度），在浅色玻璃上几乎等于没画。 */
  background: var(--accent-35);
  transition:
    background var(--dur-xs) var(--ease-out),
    height var(--dur-md) var(--ease-out);
}
.pt-bar.is-empty {
  background: var(--w-08);
}
.pt-bar:hover {
  background: var(--accent);
}
.pt-range {
  display: flex;
  justify-content: space-between;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  /* 与柱状图拉开空隙 */
  margin-top: 6px;
}
.pt-top {
  min-width: 180px;
}
.pt-top-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.pt-top-row {
  display: flex;
  justify-content: space-between;
  gap: 12px;
  font-size: var(--fs-xs);
}
.pt-top-name {
  color: var(--text-2);
}
.pt-top-time {
  color: var(--text-3);
  flex-shrink: 0;
}
</style>
