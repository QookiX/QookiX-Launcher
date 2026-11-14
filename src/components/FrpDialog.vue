<script setup lang="ts">
/** 内网穿透弹窗外壳：内容在 FrpPanel，联机页的「穿透」标签页复用同一个组件 */
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { NModal } from "naive-ui";
import FrpPanel from "./FrpPanel.vue";

const { t } = useI18n();

const show = ref(false);
defineExpose({
  open() {
    show.value = true;
  },
});
</script>

<template>
  <n-modal
    v-model:show="show"
    preset="card"
    :title="t('frp.title')"
    style="width: 680px; max-width: 94vw"
    :bordered="false"
  >
    <FrpPanel />
  </n-modal>
</template>
