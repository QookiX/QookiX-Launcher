<script setup lang="ts">
/**
 * 「检查更新」弹窗里的更新内容面板：列出当前版本 → 最新版本之间跨过的
 * 所有版本（版本号 + 日期 + 按分类配色的条目）。
 *
 * 设置·关于的「检查更新」弹窗与启动时的自动检查弹窗共用此组件，
 * 两处样式天然一致。组件自带 scoped 样式，即使在 naive-ui dialog 的
 * 渲染函数（h()）里使用也能正常生效，不会再退化成内联样式。
 */
import { useI18n } from "vue-i18n";
import { entryTypeClass, type ChangelogRelease } from "../updater";

defineProps<{ releases: ChangelogRelease[] }>();

const { t } = useI18n();
</script>

<template>
  <div class="update-notes">
    <h4 class="update-notes-title">{{ t("updaterCheck.releaseNotes") }}</h4>
    <div class="cl-entry-list">
      <template v-for="r in releases" :key="r.version">
        <!-- 跨版本更新：每个版本一个小标题（版本号 + 日期），条目列在下面 -->
        <div class="update-notes-ver">
          v{{ r.version }}<span class="update-notes-date">{{ r.date }}</span>
        </div>
        <div v-for="(e, i) in r.entries" :key="i" class="cl-entry">
          <span class="cl-tag" :class="entryTypeClass(e.text)">{{ e.category }}</span>
          <span class="cl-text">{{ e.text }}</span>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.update-notes {
  border: 1px solid var(--border);
  border-radius: 10px;
  padding: 12px 14px;
  max-height: 320px;
  overflow-y: auto;
}
.update-notes-title {
  margin: 0 0 8px;
  font-size: 13px;
  font-weight: 700;
  color: var(--text-1);
}
.update-notes-ver {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin: 12px 0 6px;
  font-size: 12px;
  font-weight: 700;
  color: var(--text-2);
}
.update-notes-ver:first-child {
  margin-top: 0;
}
.update-notes-date {
  font-size: 11px;
  font-weight: 400;
  color: var(--text-3);
}
.cl-entry-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.cl-entry {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  font-size: 13px;
  line-height: 1.5;
}
.cl-tag {
  flex-shrink: 0;
  font-size: 11px;
  font-weight: 600;
  padding: 2px 7px;
  border-radius: 5px;
  white-space: nowrap;
}
.cl-tag.cat-fix {
  color: var(--danger);
  background: rgba(229, 83, 75, 0.12);
}
.cl-tag.cat-feature {
  color: #3fb950;
  background: rgba(63, 185, 80, 0.12);
}
.cl-tag.cat-improve {
  color: #4493f8;
  background: rgba(68, 147, 248, 0.12);
}
.cl-tag.cat-refactor {
  color: #a371f7;
  background: rgba(163, 113, 247, 0.12);
}
.cl-tag.cat-other {
  color: var(--text-2);
  background: var(--bg-3, rgba(128, 128, 128, 0.12));
}
.cl-text {
  color: var(--text-1);
  word-break: break-word;
  flex: 1;
  min-width: 0;
}
</style>
