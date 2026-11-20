<script setup lang="ts">
import { NPagination } from "naive-ui";
import { useI18n } from "vue-i18n";
import { IconChevronLeft, IconChevronRight } from "./icons";

const { t } = useI18n();

const props = defineProps<{
  page: number;
  pageCount: number;
}>();

const emit = defineEmits<{
  "update:page": [page: number];
}>();

function selectPage(page: number) {
  if (page >= 1 && page <= props.pageCount && page !== props.page) {
    emit("update:page", page);
  }
}
</script>

<template>
  <NPagination
    :page="page"
    :page-count="pageCount"
    :page-slot="7"
    size="small"
    @update:page="selectPage"
  >
    <template #prev>
      <span :title="t('simplePagination.prevPage')" :aria-label="t('simplePagination.prevPage')">
        <IconChevronLeft />
      </span>
    </template>
    <template #next>
      <span :title="t('simplePagination.nextPage')" :aria-label="t('simplePagination.nextPage')">
        <IconChevronRight />
      </span>
    </template>
  </NPagination>
</template>
