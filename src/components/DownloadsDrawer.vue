<script setup lang="ts">
import { computed, nextTick, watch, ref } from "vue";
import { useI18n } from "vue-i18n";
import { useRouter } from "vue-router";
import { useTasksStore, type TaskEntry } from "../stores/tasks";
import { fmtBytes, fmtSpeed, fmtTimeMs as fmtTime } from "../utils/format";
import { useInstancesStore } from "../stores/instances";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import { useHeightTransition } from "../composables/useHeightTransition";
import { closeDownloadsDrawer, useDownloadsDrawerOpen } from "../composables/useDownloadsDrawer";
import {
  IconChevronDown,
  IconChevronRight,
  IconDownload,
  IconClose,
  IconTrash,
} from "./icons";

const { t } = useI18n();
const tasks = useTasksStore();
const instances = useInstancesStore();
const router = useRouter();
const expanded = ref<Set<number>>(new Set());
const activeTab = ref<"active" | "finished">("active");
const open = useDownloadsDrawerOpen();

// 顶部 tab 的滑动高亮指示器
const tabBox = ref<HTMLElement | null>(null);
const { indicatorStyle: tabIndicatorStyle, refresh: refreshTabIndicator, snap: snapTabIndicator } = useSlidingIndicator(
  tabBox,
  () => Array.from(tabBox.value?.querySelectorAll<HTMLElement>(".tabs button") ?? []),
  () => (activeTab.value === "active" ? 0 : 1),
  { axis: "horizontal" }
);
const activeTasks = computed(() => tasks.taskList.filter((t) => !t.finished));
const finishedTasks = computed(() => tasks.taskList.filter((t) => t.finished));
const visibleTasks = computed(() => activeTab.value === "active" ? activeTasks.value : finishedTasks.value);

watch(activeTab, () => nextTick(() => refreshTabIndicator()));
watch([() => activeTasks.value.length, () => finishedTasks.value.length], () => nextTick(() => snapTabIndicator()));
// 抽屉打开时重新校准 tab 指示器（隐藏期间几何可能是旧值）
watch(open, (v) => {
  if (v) nextTick(() => snapTabIndicator());
});

const STAGE_LABELS: Record<string, string> = {
  manifest: "downloads.stage.manifest",
  client: "downloads.stage.client",
  libraries: "downloads.stage.libraries",
  natives: "downloads.stage.natives",
  assets: "downloads.stage.assets",
  logging: "downloads.stage.logging",
  loader: "downloads.stage.loader",
  content: "downloads.stage.content",
  modpack: "downloads.stage.modpack",
  "modpack-install": "downloads.stage.modpack-install",
  runtime: "downloads.stage.runtime",
  done: "downloads.stage.done",
  prepare: "downloads.stage.prepare",
  download: "downloads.stage.download",
  extract: "downloads.stage.extract",
  verify: "downloads.stage.verify",
  install: "downloads.stage.install",
  fetch: "downloads.stage.fetch",
  resolve: "downloads.stage.resolve",
  copy: "downloads.stage.copy",
  write: "downloads.stage.write",
};

function stageLabel(task: TaskEntry) {
  const key = STAGE_LABELS[task.stage];
  return key ? t(key) : task.stage;
}

function pct(done: number, total: number) {
  if (!total) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

function downloadPct(t: TaskEntry) {
  if (t.fraction != null && t.fraction >= 0) return Math.min(100, Math.round(t.fraction * 100));
  if (t.bytesTotal > 0) return pct(t.bytesDone, t.bytesTotal);
  return pct(t.fileDone, t.fileTotal);
}

function statusText(task: TaskEntry) {
  if (task.finished) return task.ok === false ? t("downloads.status.failed") : t("downloads.status.done");
  return t("downloads.status.running");
}

function toggle(t: TaskEntry) {
  const next = new Set(expanded.value);
  if (next.has(t.id)) next.delete(t.id);
  else next.add(t.id);
  expanded.value = next;
}

// 任务卡片逐个进场（同首页 pin-card / 内容页卡片的做法）：延迟由内联 style 下发。
// 新任务出现时单张进场提示下载开始；切换 tab / 任务在活跃与已完成间移动时重播。
function cardDelay(i: number): string {
  const step = Math.min(0.045, 0.6 / Math.max(1, visibleTasks.value.length));
  return `${(0.06 + i * step).toFixed(3)}s`;
}

// 整合包会自动创建新实例，不是用户选择的目标实例，所以不显示"目标实例"
function isModpackTask(t: TaskEntry) {
  return (t.source ?? "").startsWith("整合包");
}

/**
 * 任务里的 `instanceId` 指向的实例是否已经真正可跳转。
 *
 * 实例详情页是从 instances store 里取数据的，而 store 只在启动时拉一次。
 * 整合包 / 导入这类"先下载、后建实例"的流程，任务一开始就带上了
 * instanceId，此时实例还没写进 instances.json（或者 store 还是旧快照），
 * 直接跳过去只会显示「实例不存在或已删除」。所以必须等到 store 里真的
 * 能查到这个实例才允许跳转。
 */
function instanceReady(t: TaskEntry) {
  return !!t.instanceId && !!instances.get(t.instanceId);
}

function gotoInstance(t: TaskEntry) {
  if (!instanceReady(t)) return;
  closeDownloadsDrawer();
  router.push(`/instance/${t.instanceId}`);
}

// 任务结束（成功或失败）时刷新实例列表：新建的实例这时才会进 store，
// 「目标实例」也随之从不可点击变成可跳转。
watch(
  () => tasks.taskList.filter((t) => t.finished).length,
  () => {
    void instances.load();
  }
);

// 展开 / 收起动画（实现见 useHeightTransition.ts；gap 对应 .task-card 的 flex gap）
const { onEnter: onExpandEnter, onLeave: onExpandLeave } = useHeightTransition({ gap: 10 });

// Escape 关闭抽屉
function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") closeDownloadsDrawer();
}
watch(open, (v) => {
  if (v) window.addEventListener("keydown", onKeydown);
  else window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <Teleport to="body">
    <!-- 半透明遮罩：点击关闭（浅淡不抢焦点，任务仍在后台跑） -->
    <Transition name="dl-drawer-fade">
      <div v-if="open" class="dl-mask" @click="closeDownloadsDrawer()"></div>
    </Transition>

    <!-- 右抽屉本体：悬浮岛样式（右侧留空 + 圆角 + 环绕投影 + 磨砂）。
         ⚠ 滑入用 right（layout）而不是 transform：transform 会创建独立
         合成层，抽屉的 backdrop-filter 磨砂在滑动期间采样不到背后内容。 -->
    <Transition name="dl-drawer-slide">
      <aside v-if="open" class="dl-drawer glass">
        <header class="dl-head">
          <div class="dl-title">
            <IconDownload class="dl-title-icon" />
            <span>{{ t("nav.downloads") }}</span>
          </div>
          <button
            v-if="finishedTasks.length"
            class="dl-clear-btn"
            :title="t('titlebar.clearFinished')"
            @click="tasks.clearFinished()"
          >
            <IconTrash />
          </button>
          <button class="dl-close-btn" :title="t('titlebar.close')" @click="closeDownloadsDrawer()">
            <IconClose />
          </button>
        </header>

        <div ref="tabBox" class="tabs">
          <div class="indicator" :style="tabIndicatorStyle"></div>
          <button :class="{ active: activeTab === 'active' }" @click="activeTab = 'active'">
            {{ t('downloads.tab.active') }} <span v-if="activeTasks.length" class="tab-count">{{ activeTasks.length }}</span>
          </button>
          <button :class="{ active: activeTab === 'finished' }" @click="activeTab = 'finished'">
            {{ t('downloads.tab.finished') }} <span v-if="finishedTasks.length" class="tab-count">{{ finishedTasks.length }}</span>
          </button>
        </div>

        <div v-if="!visibleTasks.length" class="empty">
          <div class="empty-icon"><IconDownload /></div>
          <p>{{ activeTab === 'active' ? t('downloads.empty.noActive') : t('downloads.empty.noFinished') }}</p>
        </div>

        <div v-else class="task-list">
          <div
            v-for="(task, i) in visibleTasks"
            :key="task.id"
            class="task-card glass"
            :style="{ animationDelay: cardDelay(i) }"
          >
            <div class="task-top" @click="toggle(task)">
              <div class="task-main">
                <div class="task-title text-ellipsis">
                  {{ task.source ?? task.message }}
                  <span class="status" :class="task.finished ? (task.ok === false ? 'fail' : 'ok') : 'run'">
                    {{ statusText(task) }}
                  </span>
                  <IconChevronDown class="caret" :class="{ open: expanded.has(task.id) }" />
                </div>
                <div class="task-meta">
                  <span class="meta-item">{{ fmtTime(task.startedAt) }}</span>
                  <span
                    v-if="task.instanceName && !isModpackTask(task)"
                    class="meta-item"
                    :class="instanceReady(task) ? 'link' : 'pending'"
                    :title="instanceReady(task) ? t('downloads.instanceLink') : t('downloads.instancePending')"
                    @click.stop="gotoInstance(task)"
                  >
                    {{ t('downloads.targetInstance', { name: task.instanceName }) }}
                    <IconChevronRight v-if="instanceReady(task)" />
                    <span v-else class="pending-tag">{{ t('downloads.creating') }}</span>
                  </span>
                  <span class="meta-item">{{ stageLabel(task) }}</span>
                </div>
              </div>
              <div class="task-side">
                <template v-if="task.activity === 'download' && !task.finished">
                  <div class="speed">{{ fmtSpeed(task.speed) }}</div>
                  <div v-if="task.fraction != null" class="stage">{{ Math.round(task.fraction * 100) }}%</div>
                  <div v-else class="stage">{{ task.fileDone }} / {{ task.fileTotal }} {{ t('downloads.fileUnit') }}</div>
                </template>
                <template v-else-if="!task.finished">
                  <div class="stage install">{{ t('downloads.installStage') }}</div>
                  <div v-if="task.stepTotal" class="stage">{{ task.stepDone }} / {{ task.stepTotal }}</div>
                </template>
              </div>
            </div>

            <!-- error message for failed tasks -->
            <div v-if="task.finished && task.ok === false" class="task-error">
              {{ task.message }}
            </div>

            <!-- download progress -->
            <div v-if="task.activity === 'download' || task.finished" class="task-progress">
              <div class="bar">
                <div
                  class="fill"
                  :style="{ width: downloadPct(task) + '%' }"
                ></div>
              </div>
              <div class="bar-info">
                <span v-if="task.fraction != null">{{ Math.round(task.fraction * 100) }}%</span>
                <span v-else>{{ task.fileDone }} / {{ task.fileTotal }} {{ t('downloads.fileUnit') }}</span>
                <span v-if="task.bytesTotal">
                  {{ fmtBytes(task.bytesDone) }} / {{ fmtBytes(task.bytesTotal) }}
                </span>
                <span v-else-if="task.bytesDone">{{ fmtBytes(task.bytesDone) }}</span>
              </div>
            </div>

            <!-- install step progress -->
            <div v-else class="task-progress">
              <div class="bar">
                <div
                  class="fill"
                  :style="{ width: pct(task.stepDone, task.stepTotal) + '%' }"
                ></div>
              </div>
              <div class="bar-info">
                <span>{{ task.message }}</span>
                <span v-if="task.stepTotal">{{ task.stepDone }} / {{ task.stepTotal }}</span>
              </div>
            </div>

            <!-- details -->
            <Transition
              :css="false"
              @enter="onExpandEnter"
              @leave="onExpandLeave"
            >
              <div v-if="expanded.has(task.id)" class="task-detail-wrap">
                <div class="task-detail">
                  <div class="detail-row">
                    <span class="dl-label">{{ t('downloads.detail.downloading') }}</span>
                    <span class="dl-value">{{ task.activeFiles.length }} {{ t('downloads.fileUnit') }}</span>
                  </div>
                  <div class="detail-row">
                    <span class="dl-label">{{ t('downloads.detail.avgSpeed') }}</span>
                    <span class="dl-value">{{ fmtSpeed(task.speed) }}</span>
                  </div>
                  <div v-if="task.files.length || task.activeFiles.length" class="detail-row files">
                    <span class="dl-label">{{ t('downloads.detail.files') }}</span>
                    <div class="dl-files">
                      <div v-for="(f, i) in task.activeFiles" :key="'a'+i" class="file-current">
                        <div class="file-current-row">
                          <span class="file-status">→</span>
                          <span class="file-name text-ellipsis">{{ f.name }}</span>
                          <span v-if="f.bytesTotal" class="file-progress">{{ pct(f.bytesDone, f.bytesTotal) }}%</span>
                        </div>
                        <div v-if="f.bytesTotal" class="file-mini-bar">
                          <div class="file-mini-fill" :style="{ width: pct(f.bytesDone, f.bytesTotal) + '%' }"></div>
                        </div>
                      </div>
                      <div
                        v-for="(f, i) in task.files.slice(-30).reverse()"
                        :key="i"
                        class="file-row"
                        :class="f.ok ? 'ok' : 'fail'"
                      >
                        <span class="file-status">{{ f.ok ? '✓' : '✗' }}</span>
                        <span class="file-name text-ellipsis">{{ f.name }}</span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </Transition>
          </div>
        </div>
      </aside>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* —— 遮罩 —— */
.dl-mask {
  position: fixed;
  inset: 0;
  z-index: 90;
  background: rgba(0, 0, 0, 0.18);
}
/* —— 抽屉本体 —— */
.dl-drawer {
  position: fixed;
  top: 50px; /* 标题栏 40px + 10px 悬浮间距 */
  bottom: 10px;
  right: 10px;
  width: 420px;
  max-width: calc(100vw - 40px);
  z-index: 95;
  display: flex;
  flex-direction: column;
  padding: 14px 14px 12px;
  border: 1px solid var(--border);
  border-radius: 18px;
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.18);
  overflow: hidden;
}
.dl-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  flex-shrink: 0;
}
.dl-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 15px;
  font-weight: 700;
  color: var(--text-1);
  flex: 1;
  min-width: 0;
}
.dl-title-icon {
  font-size: 16px;
  color: var(--accent);
}
.dl-clear-btn,
.dl-close-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: var(--text-3);
  font-size: 14px;
  cursor: pointer;
  transition: all 0.12s;
}
.dl-clear-btn:hover,
.dl-close-btn:hover {
  background: var(--panel-hover);
  color: var(--text-1);
}
/* —— 遮罩淡入淡出 —— */
.dl-drawer-fade-enter-active,
.dl-drawer-fade-leave-active {
  transition: opacity 0.2s ease;
}
.dl-drawer-fade-enter-from,
.dl-drawer-fade-leave-to {
  opacity: 0;
}
/* —— 抽屉滑入滑出（right 位移，layout 属性，磨砂全程有效） —— */
.dl-drawer-slide-enter-active {
  transition: right 0.26s cubic-bezier(0.22, 1, 0.36, 1);
}
.dl-drawer-slide-leave-active {
  transition: right 0.2s ease-in;
}
.dl-drawer-slide-enter-from,
.dl-drawer-slide-leave-to {
  right: -440px;
}
/* —— tabs —— */
.tabs {
  position: relative;
  display: inline-flex;
  gap: 4px;
  padding: 4px;
  background: var(--w-05);
  border-radius: 10px;
  align-self: flex-start;
  flex-shrink: 0;
  margin-bottom: 12px;
}
.tabs .indicator {
  position: absolute;
  top: 4px;
  bottom: 4px;
  border-radius: 7px;
  background: var(--accent-soft);
  pointer-events: none;
}
.tabs button {
  position: relative;
  border: none;
  background: transparent;
  color: var(--text-3);
  padding: 6px 14px;
  border-radius: 7px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  transition: all 0.15s;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.tabs button:hover {
  color: var(--text-1);
}
.tabs button.active {
  color: var(--accent);
}
.tab-count {
  font-size: 11px;
  background: var(--w-10);
  border-radius: 6px;
  padding: 1px 7px;
  font-weight: 700;
}
/* —— 列表 —— */
.empty {
  padding: 48px 24px;
  text-align: center;
  color: var(--text-3);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
}
.empty-icon {
  font-size: 36px;
  color: var(--text-3);
  opacity: 0.6;
}
.task-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  overflow-y: auto;
  padding-bottom: 4px;
  flex: 1;
  min-height: 0;
}
.task-card {
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  flex-shrink: 0;
  /* 卡片逐个进场（同首页 pin-card：translateY 28px 上浮 + 淡入，backwards 让
     未出场的卡片先保持隐藏）。延迟由模板内联 animationDelay 提供。 */
  animation: dl-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
@keyframes dl-card-enter {
  from {
    transform: translateY(28px);
    opacity: 0;
  }
  to {
    transform: none;
    opacity: 1;
  }
}
.task-top {
  display: flex;
  justify-content: space-between;
  gap: 14px;
  cursor: pointer;
  transition: transform 0.1s ease;
}
.task-top:active {
  transform: scale(0.98);
}
.task-main {
  min-width: 0;
  flex: 1;
}
.task-title {
  font-size: 13px;
  font-weight: 700;
  margin-bottom: 6px;
  display: flex;
  align-items: center;
  gap: 8px;
}
.caret {
  font-size: 13px;
  color: var(--text-3);
  margin-left: auto;
  flex-shrink: 0;
  /* 收起时指向右侧，展开时旋转 90° 指向下方；时长与详情区动画保持一致 */
  transform: rotate(-90deg);
  transition: transform 0.24s cubic-bezier(0.22, 1, 0.36, 1), color 0.15s;
}
.caret.open {
  transform: rotate(0deg);
  color: var(--accent);
}
.status {
  font-size: 11px;
  font-weight: 600;
  padding: 1px 8px;
  border-radius: 7px;
  flex-shrink: 0;
}
.status.run {
  color: var(--accent);
  background: var(--accent-soft);
}
.status.ok {
  color: var(--success);
  background: var(--success-12);
}
.status.fail {
  color: var(--danger);
  background: var(--danger-12);
}
.task-meta {
  display: flex;
  align-items: center;
  gap: 12px;
  font-size: 12px;
  color: var(--text-3);
  flex-wrap: wrap;
}
.meta-item {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}
.meta-item.link {
  color: var(--accent);
  cursor: pointer;
}
/* 实例还没建好，跳过去只会看到「实例不存在或已删除」，先禁用 */
.meta-item.pending {
  cursor: not-allowed;
  opacity: 0.7;
}
/* 动画容器：只负责裁剪与高度过渡，内层 .task-detail 保持 auto 以便测量 */
.task-detail-wrap {
  overflow: hidden;
}
.pending-tag {
  font-size: 10px;
  font-weight: 600;
  padding: 0 5px;
  border-radius: 5px;
  color: var(--text-3);
  background: var(--w-08);
}
.task-side {
  text-align: right;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
  justify-content: center;
}
.speed {
  font-size: 15px;
  font-weight: 700;
  color: var(--accent);
  font-variant-numeric: tabular-nums;
}
.stage {
  font-size: 11px;
  color: var(--text-3);
}
.stage.install {
  color: var(--accent);
  font-weight: 600;
}
.task-error {
  background: var(--danger-10);
  border: 1px solid var(--danger-30);
  border-radius: 8px;
  padding: 8px 12px;
  font-size: 12px;
  color: var(--danger);
  word-break: break-all;
  user-select: text;
  -webkit-user-select: text;
  cursor: text;
}
.task-progress {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.bar {
  height: 6px;
  border-radius: 4px;
  background: var(--w-08);
  overflow: hidden;
}
.fill {
  height: 100%;
  border-radius: 4px;
  background: linear-gradient(90deg, var(--accent-deep), var(--accent));
  transition: width 0.3s ease;
}
.bar-info {
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  gap: 12px;
}
.task-detail {
  border-top: 1px solid var(--border);
  padding-top: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  font-size: 12px;
}
.detail-row {
  display: flex;
  gap: 12px;
  align-items: baseline;
}
.dl-label {
  color: var(--text-3);
  flex-shrink: 0;
  width: 70px;
}
.dl-value {
  color: var(--text-2);
  min-width: 0;
}
.detail-row.files {
  flex-direction: column;
  gap: 6px;
  align-items: flex-start;
}
.dl-files {
  display: flex;
  flex-direction: column;
  gap: 3px;
  width: 100%;
  max-height: 320px;
  overflow-y: auto;
}
.file-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  padding: 4px 10px;
  border-radius: 6px;
  background: var(--w-03);
  font-family: "Consolas", "Segoe UI Mono", monospace;
}
.file-row.ok {
  background: rgba(78, 201, 160, 0.06);
}
.file-row.fail {
  background: rgba(229, 83, 75, 0.06);
}
.file-row.active {
  background: var(--accent-08);
}
.file-current {
  padding: 5px 10px 6px;
  border-radius: 6px;
  background: var(--accent-08);
  font-size: 12px;
  font-family: "Consolas", "Segoe UI Mono", monospace;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.file-current-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
.file-mini-bar {
  height: 4px;
  border-radius: 2px;
  background: var(--w-10);
}
.file-mini-fill {
  height: 100%;
  border-radius: 2px;
  background: linear-gradient(90deg, var(--accent-deep, var(--accent)), var(--accent));
  transition: width 0.2s ease;
}
.file-status {
  font-weight: 700;
  flex-shrink: 0;
  width: 16px;
  text-align: center;
}
.file-row.ok .file-status {
  color: var(--success);
}
.file-row.fail .file-status {
  color: var(--danger);
}
.file-row.active .file-status {
  color: var(--accent, #e89a4b);
}
.file-name {
  color: var(--text-2);
  min-width: 0;
  flex: 1;
}
.file-progress {
  color: var(--accent, #e89a4b);
  font-weight: 600;
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}
</style>
