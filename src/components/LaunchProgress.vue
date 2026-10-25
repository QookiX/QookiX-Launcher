<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount } from "vue";
import { listen } from "@tauri-apps/api/event";
import { useI18n } from "vue-i18n";

const visible = ref(false);
const step = ref("");
const progress = ref(0);
const done = ref(false);
const { t } = useI18n();

let unlistenProgress: (() => void) | null = null;
let unlistenLog: (() => void) | null = null;
let unlistenExit: (() => void) | null = null;
let doneTimer: ReturnType<typeof setTimeout> | null = null;

onMounted(async () => {
  unlistenProgress = await listen<{ step: string; progress: number }>("launch://progress", (e) => {
    if (done.value) return;
    visible.value = true;
    step.value = e.payload.step;
    progress.value = e.payload.progress;
  });

  unlistenLog = await listen<{ line: string }>("launch://log", () => {
    if (!visible.value || done.value) return;
    done.value = true;
    step.value = t("launchProgress.launchSuccess");
    progress.value = 100;
    doneTimer = setTimeout(() => {
      visible.value = false;
      done.value = false;
    }, 2000);
  });

  unlistenExit = await listen("launch://exit", () => {
    visible.value = false;
    done.value = false;
    if (doneTimer) { clearTimeout(doneTimer); doneTimer = null; }
  });
});

onBeforeUnmount(() => {
  unlistenProgress?.();
  unlistenLog?.();
  unlistenExit?.();
  if (doneTimer) clearTimeout(doneTimer);
});
</script>

<template>
  <Transition name="lp-slide">
    <div v-if="visible" class="launch-progress">
      <div class="lp-bar">
        <div class="lp-fill" :class="{ done }" :style="{ width: progress + '%' }"></div>
      </div>
      <div class="lp-info">
        <span class="lp-step">{{ step }}</span>
        <span class="lp-pct">{{ progress }}%</span>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.launch-progress {
  position: fixed;
  bottom: 20px;
  /* 居中不用 transform：transform 会隔离合成层，让 backdrop-filter 磨砂失效 */
  left: 0;
  right: 0;
  margin: 0 auto;
  width: 360px;
  max-width: 90vw;
  background: var(--panel, #1e2230);
  border: 1px solid var(--border, var(--w-08));
  border-radius: 12px;
  padding: 14px 18px;
  box-shadow: 0 8px 32px var(--k-40);
  backdrop-filter: blur(var(--glass-blur, 8px));
  -webkit-backdrop-filter: blur(var(--glass-blur, 8px));
  z-index: 9999;
}
.lp-bar {
  height: 4px;
  background: var(--w-08);
  border-radius: 2px;
  overflow: hidden;
  margin-bottom: 8px;
}
.lp-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--accent, #e89a4b), var(--accent-deep, #d97f33));
  border-radius: 2px;
  transition: width 0.3s ease;
}
.lp-fill.done {
  background: var(--success);
}
.lp-info {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 13px;
}
.lp-step {
  color: var(--text-2, #c6c8d2);
}
.lp-pct {
  color: var(--text-3, #8b8e9c);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
/* 进场/退场也用 layout 位移（bottom），不用 transform，避免磨砂被合成层隔离 */
.lp-slide-enter-active,
.lp-slide-leave-active {
  transition: opacity 0.25s ease, bottom 0.25s ease;
}
.lp-slide-enter-from,
.lp-slide-leave-to {
  opacity: 0;
  bottom: 0;
}
</style>
