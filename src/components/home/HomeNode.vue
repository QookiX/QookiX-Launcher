<script setup lang="ts">
import { computed, inject, ref, watch } from "vue";
import { HOME_CTX } from "../../home/context";
import type { HomeAction, HomeNode } from "../../home/types";
import { IconChevronRight } from "../icons";

defineOptions({ name: "HomeNode" });

const props = defineProps<{ node: HomeNode }>();
const ctx = inject(HOME_CTX);
if (!ctx) throw new Error("HomeNode must be used inside HomeLayout");

/**
 * 模板里按 type 分支渲染，字段访问走这个宽松视图。
 * 结构合法性已由解析层（normalizeNode）保证，这里不需要再做联合类型收窄。
 */
interface NodeView {
  type: string;
  title?: string;
  text?: string;
  size?: number;
  bold?: boolean;
  muted?: boolean;
  color?: string;
  align?: "left" | "center" | "right";
  width?: number;
  theme?: string;
  src?: string;
  fallback?: string;
  height?: number;
  variant?: string;
  tooltip?: string;
  disabled?: boolean;
  columns?: string[];
  logo?: string;
  info?: string;
  onClick?: HomeAction | HomeAction[];
  children?: HomeNode[];
  collapsible?: boolean;
  collapsed?: boolean;
  gap?: number;
}
const n = computed(() => props.node as unknown as NodeView);

/* —— 卡片折叠 —— */
const collapsed = ref(false);
watch(
  () => props.node,
  (node) => {
    const view = node as unknown as NodeView;
    // 没有标题就没有可点的头部，此时不折叠，否则内容会永远藏起来
    collapsed.value = !!view.collapsed && !!view.collapsible && !!view.title;
  },
  { immediate: true },
);
function toggle() {
  if (n.value.collapsible && n.value.title) collapsed.value = !collapsed.value;
}

/* —— 排版换算 —— */
const justify = computed(() => {
  const align = n.value.align;
  if (align === "center") return "center";
  if (align === "right") return "flex-end";
  return "flex-start";
});

const textStyle = computed(() => ({
  fontSize: n.value.size ? `${n.value.size}px` : undefined,
  fontWeight: n.value.bold ? 600 : undefined,
  color: n.value.color ?? (n.value.muted ? "var(--text-3)" : undefined),
  textAlign: n.value.align,
  maxWidth: n.value.width ? `${n.value.width}px` : undefined,
}));

const hintClass = computed(() => {
  if (n.value.theme === "yellow") return "qx-hint--yellow";
  if (n.value.theme === "red") return "qx-hint--red";
  if (n.value.theme === "blue") return "qx-hint--blue";
  return "";
});

const btnClass = computed(() => {
  switch (n.value.variant) {
    case "primary":
      return "qx-btn--primary";
    case "ghost":
      return "qx-btn--ghost";
    case "danger":
      return "qx-btn--danger";
    default:
      return "";
  }
});

/** 只接受 CSS 合法的列宽写法，防止布局文件写错把页面撑坏 */
const gridTemplate = computed(() => {
  const valid = (n.value.columns ?? []).filter((col) =>
    /^(auto|\d+(\.\d+)?(fr|px|%|em|rem))$/i.test(col.trim()),
  );
  return valid.length ? valid.join(" ") : "repeat(auto-fit, minmax(150px, 1fr))";
});

/* —— 图片：加载失败时切到备用地址 —— */
const useFallback = ref(false);
watch(
  () => n.value.src,
  () => {
    useFallback.value = false;
  },
);
const resolvedSrc = computed(() =>
  useFallback.value && n.value.fallback ? ctx.imgSrc(n.value.fallback) : ctx.imgSrc(n.value.src),
);
function onImgError() {
  if (!useFallback.value && n.value.fallback) useFallback.value = true;
}
</script>

<template>
  <template v-if="ctx.visible(node)">
    <!-- 卡片 -->
    <div
      v-if="n.type === 'card'"
      class="qx-card"
      :class="{ 'qx-card--swap': n.collapsible, 'qx-card--plain': !n.title }"
    >
      <div v-if="n.title" class="qx-card__head" @click="toggle">
        <span class="qx-card__title">{{ ctx.interp(n.title) }}</span>
        <span v-if="n.collapsible" class="qx-card__arrow" :class="{ 'is-open': !collapsed }">
          <IconChevronRight />
        </span>
      </div>
      <div v-show="!collapsed" class="qx-card__body">
        <HomeNode v-for="(child, i) in n.children" :key="i" :node="child" />
      </div>
    </div>

    <!-- 文本 -->
    <p v-else-if="n.type === 'text'" class="hl-text" :style="textStyle">{{ ctx.interp(n.text) }}</p>

    <!-- 提示条 -->
    <div v-else-if="n.type === 'hint'" class="qx-hint hl-hint" :class="hintClass">
      {{ ctx.interp(n.text) }}
    </div>

    <!-- 图片 -->
    <div v-else-if="n.type === 'image'" class="hl-image" :style="{ justifyContent: justify }">
      <img
        v-if="resolvedSrc"
        class="hl-image__pic"
        :src="resolvedSrc"
        :style="{ height: n.height ? `${n.height}px` : undefined, width: n.width ? `${n.width}px` : undefined }"
        alt=""
        @error="onImgError"
      />
    </div>

    <!-- 按钮 -->
    <div v-else-if="n.type === 'button'" class="hl-align" :style="{ justifyContent: justify }">
      <button
        class="qx-btn"
        :class="btnClass"
        :style="{ width: n.width ? `${n.width}px` : undefined }"
        :disabled="n.disabled"
        :title="n.tooltip ? ctx.interp(n.tooltip) : undefined"
        @click="ctx.run(n.onClick)"
      >
        {{ ctx.interp(n.text) }}
      </button>
    </div>

    <!-- 一排按钮 -->
    <div v-else-if="n.type === 'buttons'" class="hl-buttons" :style="{ justifyContent: justify }">
      <HomeNode v-for="(child, i) in n.children" :key="i" :node="child" />
    </div>

    <!-- 网格 -->
    <div v-else-if="n.type === 'grid'" class="hl-grid" :style="{ gridTemplateColumns: gridTemplate }">
      <HomeNode v-for="(child, i) in n.children" :key="i" :node="child" />
    </div>

    <!-- 横向排列 -->
    <div
      v-else-if="n.type === 'row'"
      class="hl-row"
      :style="{ justifyContent: justify, gap: n.gap ? `${n.gap}px` : undefined }"
    >
      <HomeNode v-for="(child, i) in n.children" :key="i" :node="child" />
    </div>

    <!-- 列表项 -->
    <button
      v-else-if="n.type === 'list-item'"
      class="qx-listitem"
      :class="{ 'qx-listitem--clickable': !!n.onClick }"
      @click="ctx.run(n.onClick)"
    >
      <img v-if="n.logo" class="qx-listitem__logo" :src="ctx.imgSrc(n.logo)" alt="" />
      <span class="qx-listitem__main">
        <span class="qx-listitem__title">{{ ctx.interp(n.title) }}</span>
        <span v-if="n.info" class="qx-listitem__info">{{ ctx.interp(n.info) }}</span>
      </span>
    </button>

    <!-- 分割线 / 空白 -->
    <hr v-else-if="n.type === 'divider'" class="qx-divider" />
    <div v-else-if="n.type === 'spacer'" :style="{ height: `${n.size ?? 8}px` }"></div>
  </template>
</template>

<style scoped>
/* 卡片正文用 flex + gap 排版（见 components.css 的 .qx-card__body），
   所以这里所有块都不写 margin，避免和 gap 叠加。 */
.hl-text {
  margin: 0;
  font-size: var(--fs-sm);
  line-height: 1.75;
  color: var(--text-2);
  white-space: pre-wrap;
  word-break: break-word;
}
.hl-hint {
  white-space: pre-wrap;
}
.hl-image {
  display: flex;
}
.hl-image__pic {
  max-width: 100%;
  object-fit: contain;
  border-radius: var(--r-sm);
}
.hl-align {
  display: flex;
}
.hl-buttons {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--sp-3);
}
.hl-grid {
  display: grid;
  gap: var(--sp-3);
}
/* 网格里的按钮撑满单元格 */
.hl-grid > .hl-align :deep(.qx-btn) {
  width: 100%;
}
.hl-row {
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}
</style>
