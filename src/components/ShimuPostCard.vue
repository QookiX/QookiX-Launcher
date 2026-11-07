<script setup lang="ts">
/**
 * 投影帖卡片（发现页「投影」分类 / 投影库列表共用）。
 *
 * 结构刻意对齐 `ProjectCard`：同一套 p-card 骨架 + grid / list / compact 三视图，
 * 底部右侧是安装按钮 —— 让投影和模组等内容长得像同一类东西，而不是两套卡片。
 * 点击卡片 = 看帖；点击安装 = 把该帖的投影文件装进工具栏里选中的实例；
 * 点击预览 = 下载该帖的投影并直接打开投影工坊预览（不需要先选实例）。
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { IconBookOpen, IconCheck, IconDownload, IconEye, IconHeart, IconBox } from "./icons";
import { shimuAbsoluteUrl, useShimuStore } from "../stores/shimu";
import type { ShimuPost } from "../types";

const props = defineProps<{
  post: ShimuPost;
  view: "grid" | "list" | "compact";
  /** 正在安装（下载 + 复制到实例） */
  installing?: boolean;
  /** 正在下载并准备预览 */
  previewing?: boolean;
  /** 本次会话里已装进目标实例 */
  installed?: boolean;
  /** 目标实例名，仅用于按钮提示 */
  instanceName?: string;
}>();

const emit = defineEmits<{
  open: [ShimuPost];
  install: [ShimuPost];
  preview: [ShimuPost];
}>();

const { t } = useI18n();
const store = useShimuStore();

const coverBroken = ref(false);

const title = computed(() => props.post.title?.trim() || t("shimu.untitled"));

const cover = computed(() => {
  const raw = props.post.cover?.trim() || props.post.images?.split(",")[0]?.trim() || "";
  return raw ? shimuAbsoluteUrl(raw, store.config?.host) : "";
});

const tags = computed(() => props.post.tagNames?.trim() || props.post.categoryName?.trim() || "");

const paid = computed(() => {
  const p = props.post;
  if (p.isPaid !== "1") return "";
  return p.payTarget === "1" ? t("shimu.paidFileShort") : t("shimu.paidContentShort");
});
</script>

<template>
  <div class="p-card glass" :class="`view-${view}`" @click="emit('open', post)">
    <template v-if="view === 'grid'">
      <!-- 网格：大图打头，像作品集而不是软件列表（和模组卡片同族但更像看建筑） -->
      <div class="p-cover">
        <img
          v-if="cover && !coverBroken"
          :src="cover"
          alt=""
          loading="lazy"
          @error="coverBroken = true"
        />
        <div v-else class="p-cover-ph">
          <span>{{ title }}</span>
        </div>
        <span v-if="paid" class="paid-badge">{{ paid }}</span>
      </div>
      <div class="p-grid-body">
        <div class="p-title" :title="title">{{ title }}</div>
        <div class="p-author">
          {{ post.authorName || "" }}
          <span v-if="post.createTime" class="p-updated">{{ post.createTime }}</span>
        </div>
        <div class="p-cats">
          <span v-for="c in tags.split(',').filter(Boolean).slice(0, 3)" :key="c" class="cat">{{ c.trim() }}</span>
        </div>
      </div>
      <div class="p-foot">
        <div class="p-stats">
          <span class="dl"><IconBookOpen /> {{ post.viewCount ?? 0 }}</span>
          <span class="fl"><IconHeart /> {{ post.likeCount ?? 0 }}</span>
        </div>
        <div class="p-actions">
          <button
            class="preview-btn"
            :disabled="previewing"
            :title="t('shimu.previewPost')"
            @click.stop="emit('preview', post)"
          >
            <IconEye /> {{ previewing ? t("shimu.preparingPreview") : t("shimu.downloadPreview") }}
          </button>
          <button
            class="install-btn"
            :class="{ installed }"
            :disabled="installed || installing"
            :title="instanceName ? t('shimu.installTo', { name: instanceName }) : t('projectCard.install')"
            @click.stop="emit('install', post)"
          >
            <IconCheck v-if="installed" /><IconDownload v-else /> {{ installed ? t("projectCard.installed") : t("projectCard.install") }}
          </button>
        </div>
      </div>
    </template>

    <template v-else>
      <div class="p-icon-wrap">
        <img
          v-if="cover && !coverBroken"
          :src="cover"
          class="p-icon"
          alt=""
          loading="lazy"
          @error="coverBroken = true"
        />
        <div v-else class="p-icon ph"><IconBox /></div>
        <span v-if="paid" class="paid-badge">{{ paid }}</span>
      </div>
      <div class="p-body">
        <div class="p-meta">
          <div class="p-title text-ellipsis" :title="title">{{ title }}</div>
          <div class="p-author">{{ post.authorName || "" }}</div>
        </div>
        <div class="p-cats">
          <span v-for="c in tags.split(',').filter(Boolean).slice(0, 3)" :key="c" class="cat">{{ c.trim() }}</span>
        </div>
      </div>
      <div class="p-side">
        <div class="p-stats">
          <span class="dl"><IconBookOpen /> {{ post.viewCount ?? 0 }}</span>
          <span class="fl"><IconHeart /> {{ post.likeCount ?? 0 }}</span>
          <span v-if="post.createTime" class="up">{{ post.createTime }}</span>
        </div>
        <div class="p-side-actions">
          <button
            class="preview-btn"
            :disabled="previewing"
            :title="t('shimu.previewPost')"
            @click.stop="emit('preview', post)"
          >
            <IconEye /> {{ previewing ? t("shimu.preparingPreview") : t("shimu.downloadPreview") }}
          </button>
          <button
            class="install-btn"
            :class="{ installed }"
            :disabled="installed || installing"
            :title="instanceName ? t('shimu.installTo', { name: instanceName }) : t('projectCard.install')"
            @click.stop="emit('install', post)"
          >
            <IconCheck v-if="installed" /><IconDownload v-else /> {{ installed ? t("projectCard.installed") : t("projectCard.install") }}
          </button>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
/* 与 ProjectCard 保持同一套尺寸与间距，三视图切换时两边的卡片高度才一致 */
.p-card {
  display: flex;
  flex-direction: column;
  padding: 14px;
  gap: 12px;
  cursor: pointer;
  transition: transform 0.1s ease;
  position: relative;
  overflow: hidden;
}
.p-card:active {
  transform: scale(0.97);
}

/* —— 网格：封面通栏，图大一点，信息压在下面（和模组卡片同族，但更像作品集） —— */
.p-card.view-grid {
  padding: 0;
  gap: 0;
}
.p-card.view-grid:active {
  transform: scale(0.985);
}
.p-cover {
  position: relative;
  aspect-ratio: 16 / 10;
  background: var(--bg-2, var(--panel));
  overflow: hidden;
}
.p-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
  transition: transform 0.4s cubic-bezier(0.22, 0.61, 0.36, 1);
}
.p-card.view-grid:hover .p-cover img {
  transform: scale(1.045);
}
/* 无封面：渐变块 + 标题（最多三行），比空白色块好认 */
.p-cover-ph {
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 14px 16px;
  background: linear-gradient(135deg, #6b5b95, #352f44);
}
.p-cover-ph span {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  overflow: hidden;
  font-size: 13px;
  font-weight: 600;
  line-height: 1.5;
  text-align: center;
  overflow-wrap: anywhere;
  color: rgba(255, 255, 255, 0.94);
}
.p-grid-body {
  padding: 10px 12px 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}
.p-card.view-grid .p-title {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  line-height: 1.45;
}
.p-card.view-grid .p-foot {
  padding: 8px 12px 12px;
}

.p-icon-wrap {
  position: relative;
  flex-shrink: 0;
}
.p-icon {
  width: 56px;
  height: 56px;
  border-radius: 10px;
  object-fit: cover;
  background: var(--panel);
  display: block;
}
.p-icon.ph {
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 20px;
  border: 1px solid var(--border);
}
.paid-badge {
  position: absolute;
  right: 8px;
  top: 8px;
  z-index: 1;
  padding: 1px 5px;
  border-radius: 6px;
  font-size: 10px;
  line-height: 1.5;
  color: var(--accent-contrast);
  background: linear-gradient(135deg, var(--accent), var(--accent-deep));
  white-space: nowrap;
  box-shadow: 0 1px 4px rgb(0 0 0 / 25%);
}
/* 列表/紧凑的角标挂在 56px 缩略图右下角，不压住图本身 */
.p-icon-wrap .paid-badge {
  top: auto;
  right: -4px;
  bottom: -4px;
  box-shadow: none;
}
.p-body {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.p-meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.p-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-1);
}
.text-ellipsis {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.p-author {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  color: var(--text-3);
}
.p-updated,
.up {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  color: var(--text-3);
  white-space: nowrap;
}
.p-cats {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}
.cat {
  padding: 1px 6px;
  border-radius: 5px;
  font-size: 10px;
  color: var(--text-3);
  background: var(--w-06);
  border: 1px solid var(--border);
}
.p-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
  /* 卡片被网格拉高时（同一行标题行数不同），操作区始终贴住卡片底部 */
  margin-top: auto;
}
.p-stats {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 11px;
  color: var(--text-3);
}
.p-stats .dl,
.p-stats .fl {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.p-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  /* 窄卡片换行时按钮仍然靠右 */
  margin-left: auto;
}
.p-card.view-list,
.p-card.view-compact {
  flex-direction: row;
  align-items: center;
  gap: 14px;
}
.p-card.view-compact {
  padding: 5px 12px;
  gap: 10px;
}
.p-card.view-list .p-icon,
.p-card.view-compact .p-icon {
  width: 46px;
  height: 46px;
}
.p-card.view-compact .p-icon {
  width: 30px;
  height: 30px;
  border-radius: 7px;
}
.p-card.view-compact .paid-badge { display: none; }
.p-card.view-compact .p-title { font-size: 13px; }
.p-card.view-compact .p-author { font-size: 10px; }
.p-card.view-compact .p-cats { display: none; }
.p-side {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 8px;
  flex-shrink: 0;
}
.p-card.view-compact .p-side {
  flex-direction: row;
  align-items: center;
  gap: 6px;
}
.p-card.view-compact .p-stats {
  gap: 6px;
  font-size: 10px;
}
.p-card.view-compact .p-stats .up { display: none; }
.p-side-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.install-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: none;
  background: linear-gradient(135deg, var(--accent), var(--accent-deep));
  color: var(--accent-contrast);
  border-radius: 8px;
  padding: 6px 13px;
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
  font-family: inherit;
  transition: filter 0.12s, opacity 0.12s;
}
/* 预览：次级按钮，和主按钮「安装」并列但更轻，避免抢主操作 */
.preview-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border);
  background: var(--w-06);
  color: var(--text-1);
  border-radius: 8px;
  padding: 6px 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  font-family: inherit;
  white-space: nowrap;
  transition: background 0.12s, border-color 0.12s, color 0.12s;
}
.preview-btn:hover:not(:disabled) {
  background: var(--w-10);
  border-color: var(--accent-35);
  color: var(--accent);
}
.preview-btn:disabled {
  opacity: 0.65;
  cursor: default;
}
.install-btn:hover:not(:disabled) { filter: brightness(1.08); }
.install-btn:disabled { cursor: default; }
.install-btn.installed {
  background: var(--accent-soft);
  color: var(--accent);
  border: 1px solid var(--accent-05);
}
.p-card.view-compact .install-btn {
  padding: 4px 9px;
  font-size: 11px;
  border-radius: 6px;
}
.p-card.view-compact .preview-btn {
  padding: 4px 9px;
  font-size: 11px;
  border-radius: 6px;
}
</style>
