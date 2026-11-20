<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import FileWorkspace from "./FileWorkspace.vue";
import { instanceFileOperations } from "./fileWorkspace";

const props = defineProps<{ instanceId: string }>();
const { t } = useI18n();
const operations = computed(() => instanceFileOperations(props.instanceId));
</script>

<template>
  <FileWorkspace :key="instanceId" :operations="operations" locale="fileManager" :root-label="t('fileManager.instanceDir')" />
</template>
