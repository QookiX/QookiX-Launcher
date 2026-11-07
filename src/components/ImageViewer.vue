<script setup lang="ts">
/**
 * 标准图片查看器（内容详情页图库、投影库图集等通用场景）。
 *
 * 常规看图交互：暗底居中、滚轮缩放、放大后拖拽平移、双击 1x/2x、
 * 左右键或两侧箭头切图、底栏页码与缩放比例、Esc 关闭、点空白关闭。
 *
 * 时光机那套「大图 + 右侧信息栏」是它自己的形态，走 `TimeMachineLightbox.vue`，
 * 不要用这个组件去承载它。
 *
 * 用法：
 *   <ImageViewer v-model:open="open" v-model:index="index" :images="urls" :titles="titles" />
 */
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import {
  IconChevronLeft,
  IconChevronRight,
  IconClose,
  IconMinus,
  IconPlus,
  IconRestore,
} from "./icons";

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** 图片地址列表（空串表示这张还没加载出来） */
    images: string[];
    index?: number;
    /** 与 images 对应的标题，可省略 */
    titles?: string[];
  }>(),
  { index: 0, titles: () => [] }
);

const emit = defineEmits<{
  "update:open": [boolean];
  "update:index": [number];
}>();

const { t } = useI18n();

const count = computed(() => props.images.length);
const safeIndex = computed(() =>
  count.value ? Math.min(Math.max(props.index, 0), count.value - 1) : 0
);
const current = computed(() => props.images[safeIndex.value] ?? "");
const title = computed(() => props.titles[safeIndex.value] ?? "");

/* ---------------- 缩放 / 平移 ---------------- */

const MIN_SCALE = 0.25;
const MAX_SCALE = 6;

const scale = ref(1);
const tx = ref(0);
const ty = ref(0);
const dragging = ref(false);
const stage = ref<HTMLElement | null>(null);

const zoomText = computed(() => `${Math.round(scale.value * 100)}%`);
const zoomed = computed(() => scale.value > 1.001);

function clampScale(v: number) {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, v));
}

function resetView() {
  scale.value = 1;
  tx.value = 0;
  ty.value = 0;
}

/** 放大后别把图拖出可视区：按舞台尺寸估一个上下左右上限 */
function clampPan() {
  const el = stage.value;
  if (!el) return;
  const maxX = (el.clientWidth * (scale.value - 1)) / 2;
  const maxY = (el.clientHeight * (scale.value - 1)) / 2;
  tx.value = Math.min(maxX, Math.max(-maxX, tx.value));
  ty.value = Math.min(maxY, Math.max(-maxY, ty.value));
}

/** 以舞台内某点（默认中心）为锚点缩放，鼠标指哪放哪 */
function zoomAt(next: number, cx = 0, cy = 0) {
  const from = scale.value;
  const to = clampScale(next);
  if (Math.abs(to - from) < 0.0001) return;
  if (to <= 1) {
    scale.value = to;
    tx.value = 0;
    ty.value = 0;
    return;
  }
  const k = to / from;
  tx.value = cx - k * (cx - tx.value);
  ty.value = cy - k * (cy - ty.value);
  scale.value = to;
  clampPan();
}

function onWheel(e: WheelEvent) {
  const el = stage.value;
  if (!el) return;
  const r = el.getBoundingClientRect();
  const cx = e.clientX - r.left - r.width / 2;
  const cy = e.clientY - r.top - r.height / 2;
  zoomAt(scale.value * (e.deltaY < 0 ? 1.18 : 1 / 1.18), cx, cy);
}

/* ---------------- 拖拽 ---------------- */

let startX = 0;
let startY = 0;
let originX = 0;
let originY = 0;
/** 拖拽后紧跟的 click 不应该当成「点空白关闭」 */
let moved = false;

function onPointerDown(e: PointerEvent) {
  if (e.button !== 0 || !zoomed.value) return;
  dragging.value = true;
  moved = false;
  startX = e.clientX;
  startY = e.clientY;
  originX = tx.value;
  originY = ty.value;
  (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
}

function onPointerMove(e: PointerEvent) {
  if (!dragging.value) return;
  const dx = e.clientX - startX;
  const dy = e.clientY - startY;
  if (Math.abs(dx) > 3 || Math.abs(dy) > 3) moved = true;
  tx.value = originX + dx;
  ty.value = originY + dy;
  clampPan();
}

function onPointerUp() {
  dragging.value = false;
  // 拖拽结束紧跟的 click 不能当成「点空白关闭」，等这一轮事件跑完再解除拦截
  window.setTimeout(() => {
    moved = false;
  }, 0);
}

function onDblClick(e: MouseEvent) {
  if (zoomed.value) {
    resetView();
    return;
  }
  const el = stage.value;
  if (!el) return;
  const r = el.getBoundingClientRect();
  zoomAt(2, e.clientX - r.left - r.width / 2, e.clientY - r.top - r.height / 2);
}

/* ---------------- 切换 / 关闭 ---------------- */

function close() {
  emit("update:open", false);
}

/** 循环切换：到头/到尾自动绕回 */
function step(delta: number) {
  if (count.value < 2) return;
  emit("update:index", (safeIndex.value + delta + count.value) % count.value);
}

function onBackdrop() {
  if (moved) {
    moved = false;
    return;
  }
  close();
}

function onKey(e: KeyboardEvent) {
  if (!props.open) return;
  if (e.key === "Escape") {
    e.preventDefault();
    close();
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    step(-1);
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    step(1);
  }
}

/** 锁住页面滚动（遮罩内部不需要滚动） */
function lockScroll(locked: boolean) {
  const el = document.getElementById("app-content");
  if (el) el.style.overflow = locked ? "hidden" : "";
}

watch(
  () => props.open,
  (open) => {
    if (open) {
      window.addEventListener("keydown", onKey);
      lockScroll(true);
    } else {
      window.removeEventListener("keydown", onKey);
      lockScroll(false);
      resetView();
    }
  }
);

// 换图时回到适应窗口，别继承上一张的缩放
watch(current, () => resetView());

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKey);
  lockScroll(false);
});
</script>

<template>
  <Teleport to="body">
    <transition name="iv-fade">
      <div v-if="open" class="iv" @click.self="onBackdrop">
        <header class="iv-bar">
          <span v-if="title" class="iv-title" :title="title">{{ title }}</span>
          <button class="iv-icon" :title="t('common.close')" :aria-label="t('common.close')" @click="close">
            <IconClose />
          </button>
        </header>

        <div
          ref="stage"
          class="iv-stage"
          :class="{ zoomed, dragging }"
          @click.self="onBackdrop"
          @wheel.prevent="onWheel"
          @pointerdown="onPointerDown"
          @pointermove="onPointerMove"
          @pointerup="onPointerUp"
          @pointercancel="onPointerUp"
          @dblclick="onDblClick"
        >
          <button
            v-if="count > 1"
            class="iv-nav prev"
            :title="t('common.prev')"
            :aria-label="t('common.prev')"
            @pointerdown.stop
            @click.stop="step(-1)"
          >
            <IconChevronLeft />
          </button>

          <img
            v-if="current"
            :key="current"
            :src="current"
            class="iv-img"
            :class="{ drag: dragging }"
            :style="{ transform: `translate3d(${tx}px, ${ty}px, 0) scale(${scale})` }"
            alt=""
            draggable="false"
            @click.stop
          />
          <div v-else class="iv-empty">···</div>

          <button
            v-if="count > 1"
            class="iv-nav next"
            :title="t('common.next')"
            :aria-label="t('common.next')"
            @pointerdown.stop
            @click.stop="step(1)"
          >
            <IconChevronRight />
          </button>
        </div>

        <footer class="iv-foot">
          <div class="iv-pager">
            <button
              v-if="count > 1"
              class="iv-icon"
              :title="t('common.prev')"
              @click="step(-1)"
            >
              <IconChevronLeft />
            </button>
            <span class="iv-count">{{ safeIndex + 1 }} / {{ count || 1 }}</span>
            <button
              v-if="count > 1"
              class="iv-icon"
              :title="t('common.next')"
              @click="step(1)"
            >
              <IconChevronRight />
            </button>
          </div>

          <div class="iv-zoom">
            <button class="iv-icon" :title="t('common.zoomOut')" @click="zoomAt(scale / 1.25)">
              <IconMinus />
            </button>
            <button class="iv-pct" :title="t('common.resetZoom')" @click="resetView">{{ zoomText }}</button>
            <button class="iv-icon" :title="t('common.zoomIn')" @click="zoomAt(scale * 1.25)">
              <IconPlus />
            </button>
            <button class="iv-icon" :title="t('common.resetZoom')" @click="resetView">
              <IconRestore />
            </button>
          </div>
        </footer>
      </div>
    </transition>
  </Teleport>
</template>

<style scoped>
.iv {
  position: fixed;
  inset: 0;
  z-index: 3000;
  display: flex;
  flex-direction: column;
  background: rgba(0, 0, 0, 0.92);
  backdrop-filter: blur(4px);
  -webkit-backdrop-filter: blur(4px);
  user-select: none;
}
.iv-fade-enter-active,
.iv-fade-leave-active {
  transition: opacity 0.18s ease;
}
.iv-fade-enter-from,
.iv-fade-leave-to {
  opacity: 0;
}

/* —— 顶部条 —— */
.iv-bar {
  flex: none;
  height: 48px;
  padding: 0 12px 0 20px;
  display: flex;
  align-items: center;
  gap: 12px;
  color: #e6e7ec;
}
.iv-title {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: #d5d7df;
}
.iv-bar .iv-icon {
  margin-left: auto;
}

/* —— 舞台 —— */
.iv-stage {
  flex: 1 1 auto;
  min-height: 0;
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 4px 64px;
  overflow: hidden;
  cursor: default;
}
.iv-stage.zoomed {
  cursor: grab;
}
.iv-stage.dragging {
  cursor: grabbing;
}
.iv-img {
  max-width: 100%;
  max-height: 100%;
  border-radius: 4px;
  box-shadow: 0 18px 56px rgb(0 0 0 / 55%);
  transition: transform 0.18s ease;
  will-change: transform;
  -webkit-user-drag: none;
}
.iv-img.drag {
  transition: none;
}
.iv-empty {
  font-size: 20px;
  color: rgba(255, 255, 255, 0.4);
}

/* —— 两侧箭头 —— */
.iv-nav {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  width: 40px;
  height: 40px;
  border-radius: 50%;
  border: 1px solid rgba(255, 255, 255, 0.14);
  background: rgba(30, 30, 34, 0.72);
  color: #fff;
  display: grid;
  place-items: center;
  cursor: pointer;
  transition: background 0.15s ease;
}
.iv-nav:hover {
  background: rgba(60, 60, 66, 0.92);
}
.iv-nav.prev {
  left: 12px;
}
.iv-nav.next {
  right: 12px;
}

/* —— 底部工具条 —— */
.iv-foot {
  flex: none;
  height: 52px;
  padding: 0 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 16px;
  position: relative;
}
.iv-pager {
  display: flex;
  align-items: center;
  gap: 4px;
}
.iv-count {
  min-width: 56px;
  text-align: center;
  font-size: 12px;
  color: #b6b8c2;
  font-variant-numeric: tabular-nums;
}
.iv-zoom {
  position: absolute;
  right: 16px;
  top: 50%;
  transform: translateY(-50%);
  display: flex;
  align-items: center;
  gap: 2px;
}
.iv-icon {
  width: 32px;
  height: 32px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: #d5d7df;
  display: grid;
  place-items: center;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease;
}
.iv-icon:hover {
  background: rgba(255, 255, 255, 0.12);
  color: #fff;
}
.iv-icon:disabled {
  opacity: 0.4;
  cursor: default;
}
.iv-pct {
  min-width: 52px;
  height: 28px;
  padding: 0 8px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: #b6b8c2;
  font-family: inherit;
  font-size: 12px;
  font-variant-numeric: tabular-nums;
  cursor: pointer;
}
.iv-pct:hover {
  background: rgba(255, 255, 255, 0.1);
  color: #fff;
}

@media (max-width: 640px) {
  .iv-stage {
    padding: 4px 8px;
  }
  .iv-nav {
    display: none;
  }
  .iv-foot {
    justify-content: flex-start;
    padding-left: 12px;
  }
}
</style>
