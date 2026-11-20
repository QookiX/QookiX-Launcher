<script setup lang="ts">
import { vSegmented } from "../../composables/vSegmented";
import { computed, nextTick, onActivated, onDeactivated, onMounted, ref, watch } from "vue";
import { fmtMem } from "../../utils/format";
import { NSlider, useMessage } from "naive-ui";
import { useMemoryInfo } from "../../composables/useMemoryInfo";
import { useSettingsStore } from "../../stores/settings";
import { useSlidingIndicator } from "../../composables/useSlidingIndicator";
import { IconSearch } from "../../components/icons";
import type { JavaInfo } from "../../types";
import { useI18n } from "vue-i18n";

const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();

// 内存模式 seg 滑动高亮（自动配置 / 手动配置）
const memModeSegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: memModeSegStyle, refresh: refreshMemModeSeg } = useSlidingIndicator(
  memModeSegRef,
  () => Array.from(memModeSegRef.value?.querySelectorAll<HTMLElement>(".seg button") ?? []),
  () => (settings.settings?.memory_mode === "auto" ? 0 : 1),
  { axis: "horizontal" }
);
watch(() => settings.settings?.memory_mode, () => nextTick(() => refreshMemModeSeg()));

async function selectMemoryMode(mode: string) {
  if (settings.settings?.memory_mode === mode) return;
  try {
    await settings.patch({ memory_mode: mode });
  } catch (e) {
    message.error(String(e));
  }
}

const javaCandidates = ref<JavaInfo[]>([]);
const detecting = ref(false);

const { memTotal, memUsed, memAvailable, startPolling, stopPolling } = useMemoryInfo();

const effectiveMemory = computed(() => {
  if (settings.settings?.memory_mode === "auto") {
    // Base: 40% of available (min 2048 MB), cap at 75% of available
    const cap = Math.max(512, Math.floor(memAvailable.value * 3 / 4));
    return Math.max(512, Math.min(Math.max(2048, Math.floor(memAvailable.value * 40 / 100)), cap, 8192));
  }
  return settings.settings?.max_memory_mb ?? 4096;
});
const usedPercent = computed(() => {
  if (!memTotal.value) return 0;
  return Math.min(100, Math.round((memUsed.value / memTotal.value) * 100));
});
const allocPercent = computed(() => {
  if (!memTotal.value) return 0;
  return Math.min(100, Math.round((effectiveMemory.value / memTotal.value) * 100));
});
const allocStart = computed(() => usedPercent.value);
const allocWidth = computed(() =>
  Math.max(0, Math.min(allocPercent.value, 100 - usedPercent.value))
);

async function detect() {
  detecting.value = true;
  try {
    await settings.loadJava(true);
    javaCandidates.value = settings.javaCandidates;
  } catch (e) {
    message.error(String(e));
  } finally {
    detecting.value = false;
  }
}

onMounted(async () => {
  javaCandidates.value = await settings.loadJava();
});
onActivated(startPolling);
onDeactivated(stopPolling);
</script>

<template>
  <div v-if="settings.settings" class="settings-body">
    <div class="settings-pane">
      <div class="grid">
        <div class="card glass" id="java-runtime">
          <h3>{{ t("settings.java.runtime") }}</h3>
          <div class="java-toolbar">
            <button class="mini-btn" :disabled="detecting" @click="detect">
              <IconSearch /> {{ detecting ? t("settings.java.detecting") : t("settings.java.detect") }}
            </button>
            <span class="hint-inline">{{ t("settings.java.detectHint") }}</span>
          </div>
          <div v-if="javaCandidates.length" class="java-list">
            <div v-for="j in javaCandidates" :key="j.path" class="java-item">
              <span class="java-name">Java {{ j.major }} ({{ j.version }})</span>
              <span class="java-path">{{ j.path }}</span>
            </div>
          </div>
          <p v-else-if="!detecting" class="hint">{{ t("settings.java.notFound") }}</p>
          <p class="hint">{{ t("settings.java.perInstance") }}</p>
        </div>

        <div class="card glass" id="java-memory">
          <h3>{{ t("settings.java.memory") }}</h3>
          <div class="mem-mode-row">
            <div ref="memModeSegRef" v-segmented="{ label: t('settings.java.memory') }" class="seg qx-seg qx-seg--sm">
              <div class="qx-seg__indicator" :style="memModeSegStyle"></div>
              <button
                :class="{ active: settings.settings.memory_mode === 'auto' }"
                @click="selectMemoryMode('auto')"
              >
                {{ t("settings.java.memAuto") }}
              </button>
              <button
                :class="{ active: settings.settings.memory_mode !== 'auto' }"
                @click="selectMemoryMode('custom')"
              >
                {{ t("settings.java.memManual") }}
              </button>
            </div>
          </div>
          <div v-if="settings.settings.memory_mode !== 'auto'" class="mem-row">
            <div>
              <label>{{ t("settings.java.maxMemory") }}</label>
              <NSlider
                v-model:value="settings.settings.max_memory_mb"
                :min="1024"
                :max="16384"
                :step="256"
                :tooltip="false"
              />
              <div class="mem-val">{{ settings.settings.max_memory_mb }} MB</div>
            </div>
          </div>
          <div class="mem-gauge">
            <div class="mem-gauge-track">
              <div class="mem-gauge-used" :style="{ width: usedPercent + '%' }"></div>
              <div
                class="mem-gauge-alloc"
                :style="{ left: allocStart + '%', width: allocWidth + '%' }"
              ></div>
            </div>
            <div class="mem-gauge-labels">
              <span><i class="dot used"></i>{{ t("settings.java.memUsed", { used: fmtMem(memUsed), percent: usedPercent }) }}</span>
              <span><i class="dot alloc"></i>{{ t("settings.java.memAlloc", { alloc: fmtMem(effectiveMemory), percent: allocPercent }) }}</span>
              <span><i class="dot total"></i>{{ t("settings.java.memTotal", { total: fmtMem(memTotal), available: fmtMem(memAvailable) }) }}</span>
            </div>
          </div>
        </div>

        <div class="card glass" id="java-jvmargs">
          <h3>{{ t("settings.java.jvmArgs") }}</h3>
          <textarea
            v-model="settings.settings.jvm_args"
            class="text-input mono"
            rows="3"
            :placeholder="t('settings.java.jvmArgsPlaceholder')"
          />
        </div>

        <div class="card glass" id="java-gameargs">
          <h3>{{ t("settings.java.gameArgs") }}</h3>
          <input
            v-model="settings.settings.game_args"
            class="text-input mono"
            :placeholder="t('settings.java.gameArgsPlaceholder')"
          />
        </div>
      </div>
    </div>
  </div>
</template>
