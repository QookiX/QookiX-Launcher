<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { NPopover } from "naive-ui";
import { useI18n } from "vue-i18n";
import type { ProjectVersion } from "../types";

const props = defineProps<{
  versions: ProjectVersion[];
  modelValue: string | null;
  disabled?: boolean;
  placeholder?: string;
  loading?: boolean;
}>();
const emit = defineEmits<{
  "update:modelValue": [v: string];
  open: [];
}>();

const { t } = useI18n();

const open = ref(false);
const typeFilter = ref<"all" | "release" | "beta" | "alpha">("all");

// 每次打开重置为「全部」，不残留上次筛选；并通知父组件懒加载
watch(open, (v) => {
  if (v) {
    typeFilter.value = "all";
    emit("open");
  }
});

function vtype(v: ProjectVersion) {
  return v.version_type ?? "release";
}

const filtered = computed(() => {
  const seen = new Set<string>();
  const out: ProjectVersion[] = [];
  for (const v of props.versions) {
    if (seen.has(v.id)) continue;
    if (typeFilter.value !== "all" && vtype(v) !== typeFilter.value) continue;
    seen.add(v.id);
    out.push(v);
  }
  return out;
});

const current = computed(() => props.versions.find((v) => v.id === props.modelValue));

function typeLabel(ty: string) {
  return (
    {
      release: t("installDialog.versionType.release"),
      beta: t("installDialog.versionType.beta"),
      alpha: t("installDialog.versionType.alpha"),
    } as Record<string, string>
  )[ty] ?? ty;
}

function pick(v: ProjectVersion) {
  emit("update:modelValue", v.id);
  open.value = false;
}
</script>

<template>
  <n-popover
    v-model:show="open"
    trigger="click"
    placement="bottom-end"
    :disabled="disabled"
    :show-arrow="false"
    :content-style="{ padding: '0', minWidth: '260px', maxWidth: '320px' }"
  >
    <template #trigger>
      <button class="vp-trigger" :disabled="disabled" type="button">
        <span class="vp-trigger-ver">{{ current ? (current.version_number || current.name || current.id) : (placeholder || "—") }}</span>
        <span v-if="current" class="vp-trigger-type" :class="vtype(current)">{{ typeLabel(vtype(current)) }}</span>
        <svg class="vp-caret" viewBox="0 0 10 6" width="10" height="6"><path d="M1 1 L5 5 L9 1" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
      </button>
    </template>
    <div class="vp-panel">
      <div class="vp-tabs">
        <button
          v-for="ty in (['all','release','beta','alpha'] as const)"
          :key="ty"
          class="vp-tab"
          :class="{ active: typeFilter === ty }"
          type="button"
          @click.stop="typeFilter = ty"
        >{{ ty === "all" ? t("installDialog.form.typeAll") : typeLabel(ty) }}</button>
      </div>
      <div class="vp-list" :key="typeFilter">
        <button
          v-for="v in filtered"
          :key="v.id"
          class="vp-item"
          :class="{ active: v.id === modelValue }"
          type="button"
          @click="pick(v)"
        >
          <span class="vp-item-ver">{{ v.version_number || v.name || v.id }}</span>
          <span class="vp-item-type" :class="vtype(v)">{{ typeLabel(vtype(v)) }}</span>
        </button>
        <div v-if="loading && !filtered.length" class="vp-empty">{{ t("checklist.loading") }}</div>
        <div v-else-if="!filtered.length" class="vp-empty">{{ t("checklist.noVersion") }}</div>
      </div>
    </div>
  </n-popover>
</template>

<style>
.vp-trigger {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 30px;
  padding: 0 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--w-04, rgba(255, 255, 255, 0.03));
  color: var(--text-1);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  max-width: 200px;
  transition: border-color 0.12s, background 0.12s;
}
.vp-trigger:hover:not(:disabled) {
  border-color: var(--text-3);
  background: var(--w-08, rgba(255, 255, 255, 0.06));
}
.vp-trigger:disabled {
  opacity: 0.5;
  cursor: default;
}
.vp-trigger-ver {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}
.vp-trigger-type {
  flex-shrink: 0;
  font-size: 9px;
  font-weight: 700;
  padding: 1px 5px;
  border-radius: 4px;
  line-height: 1.4;
}
.vp-trigger-type.release { background: rgba(76, 175, 80, 0.18); color: #4caf50; }
.vp-trigger-type.beta { background: rgba(232, 154, 75, 0.16); color: #e8a04b; }
.vp-trigger-type.alpha { background: rgba(244, 67, 54, 0.16); color: #f44336; }
.vp-caret {
  flex-shrink: 0;
  color: var(--text-3);
  margin-left: 2px;
}
/* popover 内容：NPopover 提供卡片背景/边框，这里只做布局 */
.vp-panel {
  display: flex;
  flex-direction: column;
}
.vp-tabs {
  display: flex;
  gap: 4px;
  padding: 8px 8px 6px;
  border-bottom: 1px solid var(--border);
}
.vp-tab {
  flex: 1;
  border: none;
  background: transparent;
  color: var(--text-3);
  font-size: 11px;
  font-weight: 600;
  font-family: inherit;
  padding: 4px 0;
  border-radius: 6px;
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}
.vp-tab:hover { background: var(--w-08, rgba(255, 255, 255, 0.06)); color: var(--text-1); }
.vp-tab.active { background: var(--accent, #50c878); color: #fff; }
.vp-list {
  max-height: 280px;
  overflow-y: auto;
  padding: 4px;
}
.vp-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  border: none;
  background: transparent;
  color: var(--text-1);
  font-size: 12px;
  font-family: inherit;
  padding: 7px 8px;
  border-radius: 7px;
  cursor: pointer;
  text-align: left;
  transition: background 0.1s;
}
.vp-item:hover { background: var(--w-08, rgba(255, 255, 255, 0.06)); }
.vp-item.active { background: rgba(76, 175, 80, 0.16); }
.vp-item-ver {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-weight: 600;
}
.vp-item-type {
  flex-shrink: 0;
  font-size: 9px;
  font-weight: 700;
  padding: 1px 5px;
  border-radius: 4px;
  line-height: 1.4;
}
.vp-item-type.release { background: rgba(76, 175, 80, 0.18); color: #4caf50; }
.vp-item-type.beta { background: rgba(232, 154, 75, 0.16); color: #e8a04b; }
.vp-item-type.alpha { background: rgba(244, 67, 54, 0.16); color: #f44336; }
.vp-empty {
  padding: 16px 0;
  text-align: center;
  font-size: 12px;
  color: var(--text-3);
}
</style>
