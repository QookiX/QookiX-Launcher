<script setup lang="ts">
/**
 * 时光机专用的「沉浸式看图」：大图 + 右侧信息栏（实例、触发方式、日志…）。
 *
 * 注意：这套带侧栏的形态只服务时光机。普通图片预览（内容详情页图库、投影库图集）
 * 请用 `ImageViewer.vue`，那才是通用的标准查看器，不要把两者混用。
 */
import { computed, onBeforeUnmount, watch } from "vue";
import { useI18n } from "vue-i18n";
import { IconChevronLeft, IconChevronRight, IconClose } from "./icons";

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** 图片地址列表（空串表示这张还在加载/生成） */
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
const safeIndex = computed(() => (count.value ? Math.min(Math.max(props.index, 0), count.value - 1) : 0));
const current = computed(() => props.images[safeIndex.value] ?? "");
const title = computed(() => props.titles[safeIndex.value] ?? "");

function close() {
  emit("update:open", false);
}

/** 循环切换：到头/到尾自动绕回，避免按钮变灰 */
function step(delta: number) {
  if (count.value < 2) return;
  emit("update:index", (safeIndex.value + delta + count.value) % count.value);
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

watch(
  () => props.open,
  (open) => {
    if (open) window.addEventListener("keydown", onKey);
    else window.removeEventListener("keydown", onKey);
  },
  { immediate: true }
);

onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <Teleport to="body">
    <transition name="lb">
      <div v-if="open" class="lb" @click.self="close">
        <button class="lb-x" :title="t('common.close')" :aria-label="t('common.close')" @click="close">
          <IconClose />
        </button>

        <div class="lb-stage" @click.self="close">
          <button
            v-if="count > 1"
            class="lb-nav prev"
            :title="t('common.prev')"
            :aria-label="t('common.prev')"
            @click.stop="step(-1)"
          >
            <IconChevronLeft />
          </button>
          <img v-if="current" :key="current" :src="current" class="lb-img" alt="" @click.stop />
          <div v-else class="lb-loading">···</div>
          <button
            v-if="count > 1"
            class="lb-nav next"
            :title="t('common.next')"
            :aria-label="t('common.next')"
            @click.stop="step(1)"
          >
            <IconChevronRight />
          </button>
        </div>

        <aside class="lb-side">
          <slot name="aside" />
          <div class="lb-meta">
            <span class="lb-count">{{ safeIndex + 1 }} / {{ count || 1 }}</span>
            <span v-if="title" class="lb-title">{{ title }}</span>
          </div>
        </aside>
      </div>
    </transition>
  </Teleport>
</template>

<style scoped>
.lb {
  position: fixed;
  inset: 0;
  z-index: 3000;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 18px;
  padding: 42px 24px 56px;
  background: rgba(8, 9, 12, 0.9);
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
}
.lb-enter-active,
.lb-leave-active {
  transition: opacity 0.26s ease;
}
.lb-enter-from,
.lb-leave-to {
  opacity: 0;
}
.lb-stage {
  position: relative;
  flex: 1 1 auto;
  min-width: 0;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}
.lb-img {
  max-width: 100%;
  max-height: calc(100vh - 150px);
  border-radius: 12px;
  box-shadow: 0 24px 64px rgb(0 0 0 / 45%);
  animation: lbIn 0.36s cubic-bezier(0.22, 1, 0.36, 1);
}
@keyframes lbIn {
  from {
    opacity: 0;
    transform: scale(0.965);
  }
  to {
    opacity: 1;
    transform: none;
  }
}
.lb-loading {
  font-size: 20px;
  color: rgba(255, 255, 255, 0.4);
}
.lb-nav,
.lb-x {
  border: 1px solid rgba(255, 255, 255, 0.14);
  background: rgba(255, 255, 255, 0.08);
  color: #fff;
  display: grid;
  place-items: center;
  cursor: pointer;
  transition: background 0.2s ease;
}
.lb-nav:hover,
.lb-x:hover {
  background: rgba(255, 255, 255, 0.18);
}
.lb-nav {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  width: 44px;
  height: 44px;
  border-radius: 50%;
  font-size: 18px;
}
.lb-nav.prev {
  left: -6px;
}
.lb-nav.next {
  right: -6px;
}
.lb-x {
  position: absolute;
  right: 20px;
  top: 16px;
  width: 34px;
  height: 34px;
  border-radius: 10px;
}
.lb-side {
  width: 300px;
  flex-shrink: 0;
  max-height: calc(100vh - 150px);
  overflow: auto;
  padding: 14px 16px;
  border-radius: 14px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: rgba(255, 255, 255, 0.06);
  display: flex;
  flex-direction: column;
  gap: 9px;
  font-size: 12px;
  color: #e8e9ee;
}
.lb-meta {
  margin-top: auto;
  padding-top: 10px;
  border-top: 1px solid rgba(255, 255, 255, 0.08);
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
}
.lb-count {
  font-size: 11px;
  color: #9a9daa;
  font-variant-numeric: tabular-nums;
}
.lb-title {
  font-size: 12px;
  color: #e8e9ee;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
@media (max-width: 900px) {
  .lb-side {
    display: none;
  }
}
</style>
