<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import FileWorkspace from "./FileWorkspace.vue";
import { hostedFileOperations } from "./fileWorkspace";

const props = defineProps<{ serverId: string }>();
const { t } = useI18n();
const operations = computed(() => hostedFileOperations(props.serverId));
</script>

<template>
  <FileWorkspace :key="serverId" :operations="operations" locale="serverFileManager" :root-label="t('serverFileManager.serverDir')" />
</template>
