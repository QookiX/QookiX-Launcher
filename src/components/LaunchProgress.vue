<script setup lang="ts">
import { computed, watch } from "vue";
import { useI18n } from "vue-i18n";
import { useTasksStore } from "../stores/tasks";

const tasks = useTasksStore();
const { t } = useI18n();
const overlay = computed(() => tasks.launchOverlay);
const done = computed(() => overlay.value?.done ?? false);
const step = computed(() => done.value ? t("launchProgress.launchSuccess") : overlay.value?.step ?? "");
const progress = computed(() => overlay.value?.progress ?? 0);

watch(overlay, (current, _, onCleanup) => {
  if (!current?.done) return;
  const timer = setTimeout(() => tasks.dismissLaunchOverlay(current.taskId), 2000);
  onCleanup(() => clearTimeout(timer));
});
</script>

<template>
  <Transition name="lp-slide">
    <div v-if="overlay" class="launch-progress">
      <div class="lp-bar" role="progressbar" :aria-label="step" :aria-valuenow="progress">
        <div class="lp-fill" :class="{ done }" :style="{ width: progress + '%' }"></div>
      </div>
      <div class="lp-info">
        <span class="lp-step" role="status">{{ step }}</span>
        <span class="lp-pct" aria-hidden="true">{{ progress }}%</span>
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
  border-radius: var(--r-md);
  padding: 14px 18px;
  box-shadow: 0 8px 32px var(--k-40);
  backdrop-filter: blur(var(--glass-blur, 8px));
  -webkit-backdrop-filter: blur(var(--glass-blur, 8px));
  z-index: var(--z-launch-progress);
}
.lp-bar {
  height: 4px;
  background: var(--w-08);
  border-radius: var(--r-xs);
  overflow: hidden;
  margin-bottom: 8px;
}
.lp-fill {
  height: 100%;
  background: linear-gradient(90deg, var(--accent, #e89a4b), var(--accent-deep, #d97f33));
  border-radius: var(--r-xs);
  /* 进度条是按事件跳变的，"ease" 会在每段开头突然加速；
     换成统一的快起缓收曲线后，连续跳数看着像在匀速爬 */
  transition: width 0.32s var(--ease-out);
}
.lp-fill.done {
  background: var(--success);
}
.lp-info {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: var(--fs-sm);
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
  transition: opacity var(--dur-md) var(--ease-out), bottom 0.28s var(--ease-out);
}
.lp-slide-enter-from,
.lp-slide-leave-to {
  opacity: 0;
  bottom: 0;
}
</style>
