<script setup lang="ts">
import { NAlert, NEmpty, NSpin } from "naive-ui";
import { IconSearch } from "./icons";

defineProps<{ kind: "loading" | "empty" | "error"; text: string; compact?: boolean }>();
</script>

<template>
  <NAlert v-if="kind === 'error'" type="error" :bordered="false">
    <span role="alert">{{ text }}</span>
    <div v-if="$slots.default" class="status-actions"><slot /></div>
  </NAlert>
  <NEmpty v-else-if="kind === 'empty'" class="qx-status" :description="text" :size="compact ? 'small' : 'medium'">
    <template #icon><IconSearch aria-hidden="true" /></template>
    <template v-if="$slots.default" #extra><slot /></template>
  </NEmpty>
  <div v-else class="qx-status" :class="{ 'qx-status--compact': compact }">
    <NSpin size="small" aria-hidden="true">
      <template v-if="$slots.spinner" #icon><slot name="spinner" /></template>
    </NSpin>
    <span role="status">{{ text }}</span>
    <slot />
  </div>
</template>

<style scoped>
.status-actions { margin-top: var(--sp-3); }
</style>
