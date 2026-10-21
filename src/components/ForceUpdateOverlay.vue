<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { NModal, NButton, NProgress } from "naive-ui";
import { useI18n } from "vue-i18n";
import { forceUpdateInfo, downloadUpdate, applyUpdateNow } from "../updater";
import { error as devError } from "../utils/logger";

const { t } = useI18n();

const phase = ref<"idle" | "downloading" | "restarting" | "failed">("idle");
const fraction = ref(0);
const errorMsg = ref("");

const visible = computed(() => forceUpdateInfo.value !== null);

const bodyText = computed(() => {
  const info = forceUpdateInfo.value;
  if (!info) return "";
  if (info.currentVersion) {
    return t("updaterCheck.forceBody", {
      current: info.currentVersion,
      version: info.version ?? "?",
    });
  }
  return t("updaterCheck.forceBodyGeneric");
});

async function startUpdate() {
  phase.value = "downloading";
  fraction.value = 0;
  errorMsg.value = "";
  try {
    const ok = await downloadUpdate((p) => {
      if (p.fraction != null) fraction.value = p.fraction;
    });
    if (!ok) {
      phase.value = "failed";
      errorMsg.value = t("updaterCheck.updateFailed");
      return;
    }
    // 下载完成 → 安装并自动重启（applyUpdateNow 内部会调 relaunch）
    phase.value = "restarting";
    await applyUpdateNow();
  } catch (err) {
    devError("[force-update] download/install failed:", err);
    phase.value = "failed";
    errorMsg.value = err instanceof Error ? err.message : String(err);
  }
}

// 一旦检测到强制更新，立即开始下载（不等用户点按钮——锁死界面下没有理由等）
watch(visible, (v) => {
  if (v && phase.value === "idle") {
    void startUpdate();
  }
});
</script>

<template>
  <n-modal
    :show="visible"
    :mask-closable="false"
    :close-on-esc="false"
    :auto-focus="false"
    transform-origin="center"
  >
    <div class="force-overlay">
      <div class="force-card">
        <div class="force-icon">⚠</div>
        <h2 class="force-title">{{ t("updaterCheck.forceTitle") }}</h2>
        <p class="force-body">{{ bodyText }}</p>

        <!-- 下载进度 -->
        <div v-if="phase === 'downloading'" class="force-progress">
          <n-progress
            type="line"
            :percentage="Math.round(fraction * 100)"
            :height="8"
            border-radius="4"
            :show-indicator="false"
          />
          <span class="force-progress-label">{{ t("updaterCheck.forceDownloading") }}</span>
        </div>

        <!-- 重启中 -->
        <div v-else-if="phase === 'restarting'" class="force-progress">
          <span class="force-progress-label">{{ t("updaterCheck.forceRestarting") }}</span>
        </div>

        <!-- 失败 -->
        <div v-else-if="phase === 'failed'" class="force-failed">
          <p class="force-error">{{ errorMsg || t("updaterCheck.updateFailed") }}</p>
        </div>

        <!-- 操作按钮 -->
        <div class="force-actions">
          <n-button
            v-if="phase === 'failed'"
            type="primary"
            size="medium"
            @click="void startUpdate()"
          >
            {{ t("updaterCheck.forceRetry") }}
          </n-button>
          <n-button
            v-else-if="phase === 'idle'"
            type="primary"
            size="medium"
            :loading="false"
            @click="void startUpdate()"
          >
            {{ t("updaterCheck.forceUpdateNow") }}
          </n-button>
          <!-- downloading / restarting: 无按钮，只显示进度 -->
        </div>
      </div>
    </div>
  </n-modal>
</template>

<style scoped>
.force-overlay {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 100vw;
  height: 100vh;
}

.force-card {
  width: 440px;
  max-width: 90vw;
  padding: 32px 28px 24px;
  border-radius: 14px;
  background: var(--card-bg, #1e1e1e);
  box-shadow: 0 8px 40px rgba(0, 0, 0, 0.45);
  display: flex;
  flex-direction: column;
  align-items: center;
  text-align: center;
  gap: 14px;
}

.force-icon {
  font-size: 42px;
  line-height: 1;
  color: #f0a020;
}

.force-title {
  margin: 0;
  font-size: 18px;
  font-weight: 600;
}

.force-body {
  margin: 0;
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-2, #999);
}

.force-progress {
  width: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
}

.force-progress-label {
  font-size: 12px;
  color: var(--text-3, #777);
}

.force-failed {
  width: 100%;
}

.force-error {
  margin: 0;
  font-size: 12px;
  color: #e55;
  word-break: break-word;
}

.force-actions {
  margin-top: 4px;
}
</style>
