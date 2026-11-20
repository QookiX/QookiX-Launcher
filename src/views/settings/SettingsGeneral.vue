<script setup lang="ts">
import { vSegmented } from "../../composables/vSegmented";
import { nextTick, ref, watch } from "vue";
import { useSettingsStore } from "../../stores/settings";
import { useSlidingIndicator } from "../../composables/useSlidingIndicator";
import LanSyncPanel from "../../components/LanSyncPanel.vue";
import { useI18n } from "vue-i18n";

import SettingsDataDirectory from "./SettingsDataDirectory.vue";

const settings = useSettingsStore();
const { t } = useI18n();

// 关闭行为 seg 滑动高亮（每次询问 / 最小化到后台 / 退出程序）
const closeSegRef = ref<HTMLElement | null>(null);
const closeBehaviors = ["ask", "minimize", "quit"] as const;
const { indicatorStyle: closeSegStyle, refresh: refreshCloseSeg } = useSlidingIndicator(
  closeSegRef,
  () => Array.from(closeSegRef.value?.querySelectorAll<HTMLElement>(".seg button") ?? []),
  () => Math.max(0, closeBehaviors.indexOf((settings.settings?.close_behavior ?? "ask") as (typeof closeBehaviors)[number])),
  { axis: "horizontal" }
);
watch(() => settings.settings?.close_behavior, () => nextTick(() => refreshCloseSeg()));
</script>

<template>
  <div v-if="settings.settings" class="settings-body">
    <div class="settings-pane">
      <div class="grid">
        <div class="card glass" id="general-behavior">
          <h3>{{ t("settings.general.behavior") }}</h3>
          <div class="choice-row">
            <span>{{ t("settings.general.closeWindow") }}</span>
            <div ref="closeSegRef" v-segmented="{ label: t('settings.general.closeWindow') }" class="seg qx-seg qx-seg--sm">
              <div class="qx-seg__indicator" :style="closeSegStyle"></div>
              <button
                :class="{ active: settings.settings.close_behavior === 'ask' }"
                @click="settings.patch({ close_behavior: 'ask' })"
              >
                {{ t("settings.general.closeAsk") }}
              </button>
              <button
                :class="{ active: settings.settings.close_behavior === 'minimize' }"
                @click="settings.patch({ close_behavior: 'minimize' })"
              >
                {{ t("settings.general.closeMinimize") }}
              </button>
              <button
                :class="{ active: settings.settings.close_behavior === 'quit' }"
                @click="settings.patch({ close_behavior: 'quit' })"
              >
                {{ t("settings.general.closeQuit") }}
              </button>
            </div>
          </div>
          <div class="choice-row">
            <div class="choice-info">
              <span class="choice-label">{{ t("settings.general.autoUpdate") }}</span>
              <p class="choice-hint">{{ t("settings.general.autoUpdateHint") }}</p>
            </div>
            <button
              class="toggle"
              :class="{ on: settings.settings.auto_update }"
              role="switch"
              :aria-checked="settings.settings.auto_update"
              @click="settings.patch({ auto_update: !settings.settings.auto_update })"
            >
              <span class="knob"></span>
            </button>
          </div>
        </div>

        <SettingsDataDirectory />

        <div class="card glass" id="general-lan-sync">
          <LanSyncPanel />
        </div>
      </div>
    </div>
  </div>
</template>
