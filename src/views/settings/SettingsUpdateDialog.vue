<script setup lang="ts">
import { NModal, useMessage } from "naive-ui";
import { openDownloadsDrawer } from "../../composables/useDownloadsDrawer";
import { downloadUpdate, type UpdateOffer, type ChangelogRelease } from "../../updater";
import UpdateNotes from "../../components/UpdateNotes.vue";
import { error as devError } from "../../utils/logger";
import { useI18n } from "vue-i18n";

const { t } = useI18n();
const message = useMessage();
const show = defineModel<boolean>("show", { required: true });
const props = defineProps<{ offer: UpdateOffer | null; notes?: ChangelogRelease[]; previewVersion?: string }>();

async function doInstall() {
  const offer = props.offer;
  if (!offer) return;
  // 打开下载抽屉让用户实时看到进度（取代原 /downloads 页面跳转）。
  openDownloadsDrawer();
  try {
    const downloaded = await downloadUpdate(offer);
    if (!downloaded) return;
    // 只下载不安装：安装与重启由标题栏「重启以更新」按钮触发。
    message.success(t("settings.update.installed"));
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    message.error(detail || t("settings.update.installFailed"));
    devError("[updater] install error:", err);
  }
}
</script>

<template>
  <n-modal v-model:show="show" preset="card" :title="t('settings.update.newVersionFound')" class="settings-dialog update-modal" :style="{ width: '420px', maxWidth: '90vw' }">
    <div class="update-modal-body">
      <p class="update-modal-ver">{{ t("settings.update.newVersionContent", { version: offer?.version ?? previewVersion }) }}</p>
      <UpdateNotes v-if="notes?.length" :releases="notes" />
      <div class="update-modal-actions">
        <button class="mini-btn" @click="show = false">{{ t("settings.update.later") }}</button>
        <button class="mini-btn primary" :disabled="!offer" @click="show = false; void doInstall()">
          {{ t("settings.update.downloadAndInstall") }}
        </button>
      </div>
    </div>
  </n-modal>
</template>
