<script setup lang="ts">
import { NModal } from "naive-ui";

withDefaults(defineProps<{ open: boolean; label: string; duration?: number }>(), { duration: 180 });
const emit = defineEmits<{ close: []; keydown: [KeyboardEvent] }>();
</script>

<template>
  <NModal :show="open" :mask-closable="false" @update:show="(show) => { if (!show) emit('close'); }">
    <div class="qx-media-dialog" :style="{ '--media-fade-duration': `${duration}ms` }" :aria-label="label" @keydown="emit('keydown', $event)">
      <slot />
    </div>
  </NModal>
</template>
