<script setup lang="ts">
import { nextTick, useId, watch } from "vue";
import { NDrawer } from "naive-ui";

const props = withDefaults(defineProps<{ open: boolean; label: string; wide?: boolean }>(), { wide: false });
const emit = defineEmits<{ close: [] }>();
const id = useId();

function focusDrawer() {
  const drawer = document.getElementById(id);
  const focused = document.activeElement;
  if (!props.open || !drawer || drawer.contains(focused) || focused?.closest('[aria-modal="true"]')) return;
  const target = drawer.querySelector<HTMLElement>("[data-drawer-close]")
    ?? drawer.querySelector<HTMLElement>("button:not([disabled])")
    ?? drawer;
  target.focus({ preventScroll: true });
}

watch(() => props.open, (open) => { if (open) void nextTick(focusDrawer); }, { flush: "post" });
</script>

<template>
  <NDrawer
    :id="id"
    class="qx-drawer glass"
    content-class="qx-drawer__body"
    tabindex="-1"
    :aria-label="label"
    :show="open"
    :width="wide ? 420 : 380"
    :style="{ '--drawer-offscreen': wide ? '-440px' : '-400px' }"
    @after-enter="focusDrawer"
    @update:show="(show) => { if (!show) emit('close'); }"
  >
    <slot />
  </NDrawer>
</template>
