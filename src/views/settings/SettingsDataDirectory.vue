<script setup lang="ts">
import { onMounted, ref } from "vue";
import { NModal, useMessage } from "naive-ui";
import { open } from "@tauri-apps/plugin-dialog";
import { relaunch } from "@tauri-apps/plugin-process";
import { useSettingsStore } from "../../stores/settings";
import { api } from "../../api";
import { useI18n } from "vue-i18n";

const settings = useSettingsStore();
const { t } = useI18n();
const message = useMessage();

async function openPath() {
  try {
    await api.revealDataDir();
  } catch (e) {
    message.error(t("settings.general.openFailed") + String(e));
  }
}

// 数据目录迁移
const migrating = ref(false);
const scheduled = ref(false);
const migrationError = ref<string | null>(null);

onMounted(async () => {
  try {
    migrationError.value = await api.dataRootMigrationError();
  } catch (error) {
    message.error(String(error));
  }
});

async function cancelFailedMigration() {
  try {
    await api.cancelFailedDataRootMigration();
    migrationError.value = null;
  } catch (error) {
    message.error(String(error));
  }
}
const migrateModal = ref(false);
const migratePhase = ref<"select" | "done">("select");
const pendingNewDir = ref("");
const migrateMode = ref<"move" | "copy" | "pointer">("move");

async function pickDataDir() {
  try {
    const dir = await open({ directory: true, title: t("settings.migrate.pickTitle") });
    if (!dir || typeof dir !== "string") return;
    pendingNewDir.value = dir;
    migrateMode.value = "move";
    migratePhase.value = "select";
    migrateModal.value = true;
  } catch {
    /* ignore */
  }
}

async function confirmMigrate() {
  if (!pendingNewDir.value) return;
  migrating.value = true;
  try {
    const res = await api.changeDataDir(pendingNewDir.value, migrateMode.value);
    scheduled.value = res.scheduled;
    if (!res.scheduled && settings.settings) settings.settings.data_dir = res.new_dir;
    migratePhase.value = "done";
  } catch (e) {
    message.error(String(e));
  } finally {
    migrating.value = false;
  }
}

async function relaunchNow() {
  migrateModal.value = false;
  try {
    await relaunch();
  } catch (e) {
    message.error(t("settings.migrate.relaunchFailed", { error: String(e) }));
  }
}
</script>

<template>
  <div v-if="settings.settings" class="card glass" id="general-datadir">
    <h3>{{ t("settings.general.dataDir") }}</h3>
    <div class="dir-row">
      <code class="mono dir">{{ settings.settings.data_dir }}</code>
      <button class="mini-btn" @click="openPath()">{{ t("settings.general.open") }}</button>
      <button class="mini-btn" @click="pickDataDir">{{ t("settings.general.change") }}</button>
    </div>
    <p class="hint">{{ t("settings.general.dataDirHint") }}</p>
    <div v-if="migrationError" class="migrate-warn" role="alert">
      {{ migrationError }}
      <button class="mini-btn" @click="cancelFailedMigration">{{ t("settings.migrate.cancelFailed") }}</button>
    </div>
  </div>

  <n-modal v-model:show="migrateModal" preset="card" :title="t('settings.migrate.modalTitle')" class="settings-dialog migrate-modal" :style="{ width: '480px', maxWidth: '90vw' }">
    <div v-if="migratePhase === 'select'" class="migrate-body">
      <p class="migrate-label">{{ t("settings.migrate.newDir") }}</p>
      <code class="mono dir">{{ pendingNewDir }}</code>
      <p class="migrate-label">{{ t("settings.migrate.mode") }}</p>
      <div class="migrate-modes">
        <button
          type="button"
          class="migrate-mode"
          :class="{ active: migrateMode === 'move' }"
          @click="migrateMode = 'move'"
        >
          <span class="mm-title">{{ t("settings.migrate.moveTitle") }}</span>
          <span class="mm-desc">{{ t("settings.migrate.moveDesc") }}</span>
        </button>
        <button
          type="button"
          class="migrate-mode"
          :class="{ active: migrateMode === 'copy' }"
          @click="migrateMode = 'copy'"
        >
          <span class="mm-title">{{ t("settings.migrate.copyTitle") }}</span>
          <span class="mm-desc">{{ t("settings.migrate.copyDesc") }}</span>
        </button>
        <button
          type="button"
          class="migrate-mode"
          :class="{ active: migrateMode === 'pointer' }"
          @click="migrateMode = 'pointer'"
        >
          <span class="mm-title">{{ t("settings.migrate.pointerTitle") }}</span>
          <span class="mm-desc">{{ t("settings.migrate.pointerDesc") }}</span>
        </button>
      </div>
      <p class="migrate-warn">{{ t("settings.migrate.warn") }}</p>
      <div class="migrate-actions">
        <button class="mini-btn" @click="migrateModal = false">{{ t("common.cancel") }}</button>
        <button class="mini-btn primary" :disabled="migrating" @click="confirmMigrate">
          {{ migrating ? t("settings.migrate.migrating") : t("settings.migrate.start") }}
        </button>
      </div>
    </div>
    <div v-else class="migrate-body">
      <p class="migrate-ok">{{ t(scheduled ? "settings.migrate.scheduled" : "settings.migrate.done") }}</p>
      <code class="mono dir">{{ pendingNewDir }}</code>
      <p class="migrate-warn">{{ t("settings.migrate.restartWarn") }}</p>
      <div class="migrate-actions">
        <button class="mini-btn" @click="migrateModal = false">{{ t("settings.migrate.later") }}</button>
        <button class="mini-btn primary" @click="relaunchNow">{{ t("settings.migrate.now") }}</button>
      </div>
    </div>
  </n-modal>
</template>
