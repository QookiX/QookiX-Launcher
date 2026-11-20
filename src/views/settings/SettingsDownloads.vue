<script setup lang="ts">
import { onMounted, ref } from "vue";
import { NSlider, useMessage } from "naive-ui";
import { useSettingsStore } from "../../stores/settings";
import { api } from "../../api";
import type { MirrorPreset } from "../../types";
import { useI18n } from "vue-i18n";

const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();

// 下载镜像源
const mirrors = ref<MirrorPreset[]>([]);
/** 每个镜像最近一次测速结果（毫秒）；null 表示不可用 */
const mirrorLatency = ref<Record<string, number | null>>({});
const testingMirror = ref("");

async function loadMirrors() {
  try {
    mirrors.value = await api.listMirrors();
  } catch {
    mirrors.value = [];
  }
}

async function selectMirror(id: string) {
  if (settings.settings?.mirror === id) return;
  try {
    await settings.patch({ mirror: id });
  } catch (e) {
    message.error(String(e));
  }
}

async function testMirror(id: string, base: string) {
  if (testingMirror.value) return;
  testingMirror.value = id;
  try {
    const res = await api.testMirror(base);
    mirrorLatency.value = { ...mirrorLatency.value, [id]: res.ms };
  } catch (e) {
    mirrorLatency.value = { ...mirrorLatency.value, [id]: null };
    message.error(String(e));
  } finally {
    testingMirror.value = "";
  }
}

function onCustomMirrorInput() {
  if (settings.settings && settings.settings.mirror !== "custom") {
    settings.settings.mirror = "custom";
  }
}

// 内容 API 镜像（发现页 / 内容详情的 Modrinth、CurseForge 查询接口）

onMounted(loadMirrors);
</script>

<template>
  <div v-if="settings.settings" class="settings-body">
    <div class="settings-pane">
      <div class="grid">
        <div class="card glass" id="download-parallel">
          <h3>{{ t("settings.download.parallel") }}</h3>
          <div class="row-label">
            <span class="row-label-text">{{ t("settings.download.threads", { count: settings.settings.download_threads }) }}</span>
            <NSlider
              v-model:value="settings.settings.download_threads"
              :min="1"
              :max="32"
              :step="1"
              :tooltip="false"
            />
          </div>
          <p class="hint">{{ t("settings.download.threadsHint") }}</p>
          <div class="row-label" style="margin-top: 16px;">
            <span class="row-label-text">{{ t("settings.download.chunkThreads", { count: settings.settings.download_chunk_threads }) }}</span>
            <NSlider
              v-model:value="settings.settings.download_chunk_threads"
              :min="1"
              :max="16"
              :step="1"
              :tooltip="false"
            />
          </div>
          <p class="hint">{{ t("settings.download.chunkThreadsHint") }}</p>
        </div>
        <div class="card glass" id="download-mirror">
          <h3>{{ t("settings.download.mirror") }}</h3>
          <div class="mirror-list">
            <button
              v-for="m in mirrors"
              :key="m.id"
              type="button"
              class="mirror-item"
              :class="{ active: settings.settings.mirror === m.id }"
              @click="selectMirror(m.id)"
            >
              <span class="mirror-main">
                <span class="mirror-name">{{ m.label }}</span>
                <span class="mirror-base">{{ m.base || t("settings.download.mirrorDirect") }}</span>
              </span>
              <span class="mirror-side">
                <span
                  v-if="mirrorLatency[m.id] !== undefined"
                  class="mirror-ms"
                  :class="{ bad: mirrorLatency[m.id] === null }"
                >
                  {{ mirrorLatency[m.id] === null ? t("settings.download.mirrorUnavailable") : `${mirrorLatency[m.id]} ms` }}
                </span>
                <span
                  class="mirror-btn"
                  :class="{ disabled: testingMirror === m.id }"
                  @click.stop="testMirror(m.id, m.base)"
                >
                  {{ testingMirror === m.id ? t("settings.download.testing") : t("settings.download.test") }}
                </span>
              </span>
            </button>
            <div
              class="mirror-custom"
              :class="{ active: settings.settings.mirror === 'custom' }"
              role="button"
              tabindex="0"
              @click="selectMirror('custom')"
              @keydown.enter.prevent="selectMirror('custom')"
              @keydown.space.prevent="selectMirror('custom')"
            >
              <span class="mirror-custom-head">{{ t("settings.download.customMirror") }}</span>
              <input
                v-model="settings.settings.mirror_custom"
                class="text-input mono"
                placeholder="https://your-mirror.example.com"
                @input="onCustomMirrorInput"
              />
              <span
                class="mirror-btn"
                :class="{ disabled: testingMirror === 'custom' || !settings.settings.mirror_custom }"
                @click="testMirror('custom', settings.settings.mirror_custom)"
              >
                {{ testingMirror === 'custom' ? t("settings.download.testing") : t("settings.download.test") }}
              </span>
            </div>
          </div>
          <p class="hint">
            {{ t("settings.download.mirrorHintLead") }}
            <b>{{ t("settings.download.mirrorHintBold") }}</b>{{ t("settings.download.mirrorHintTail") }}
          </p>
        </div>
        <div class="card glass" id="download-center">
          <h3>{{ t("settings.download.center") }}</h3>
          <p class="hint">{{ t("settings.download.centerHint") }}</p>
        </div>
      </div>
    </div>
  </div>
</template>
