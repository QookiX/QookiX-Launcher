<script setup lang="ts">
/**
 * 区块编辑：地图（点一下选区块）或按坐标列表选区块，
 * 选中后复用 NbtTreeEditor 改区块 NBT。
 * 区块数据在 `.mca`（region 文件）里，写盘前会自动备份该 region 文件。
 */
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { NSelect, useMessage } from "naive-ui";
import { api } from "../../api";
import type { ChunkEntry } from "../../types";
import { IconRefresh } from "../icons";
import { useSlidingIndicator } from "../../composables/useSlidingIndicator";
import ChunkMap from "./ChunkMap.vue";
import NbtTreeEditor from "./NbtTreeEditor.vue";

/** 改完区块要让地图上的那张旧图作废 */
const mapRef = ref<{ refresh: () => void } | null>(null);

function onChunkSaved(name: string) {
  emit("backed-up", name);
  mapRef.value?.refresh();
}

const props = defineProps<{
  instanceId: string;
  world: string;
  running: boolean;
  readonly?: boolean;
}>();
const emit = defineEmits<{ (e: "backed-up", name: string): void }>();

const { t } = useI18n();
const message = useMessage();

const dim = ref("overworld");
const view = ref<"map" | "list">("map");
// 地图 / 列表切换的滑动高亮
const viewSegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: viewSegStyle, refresh: refreshViewSeg } = useSlidingIndicator(
  viewSegRef,
  () => Array.from(viewSegRef.value?.querySelectorAll<HTMLElement>("button") ?? []),
  () => (view.value === "map" ? 0 : 1),
);
watch(view, () => nextTick(() => refreshViewSeg()));
const chunks = ref<ChunkEntry[]>([]);
const loading = ref(false);
const selected = ref<{ cx: number; cz: number } | null>(null);

const dimOptions = computed(() => [
  { label: t("nbt.dimOverworld"), value: "overworld" },
  { label: t("nbt.dimNether"), value: "nether" },
  { label: t("nbt.dimEnd"), value: "end" },
]);
const chunkOptions = computed(() =>
  chunks.value.map((c) => ({ label: `${c.cx}, ${c.cz}`, value: `${c.cx},${c.cz}` }))
);
const chunkValue = computed(() =>
  selected.value ? `${selected.value.cx},${selected.value.cz}` : null
);

function pickChunk(v: string | null) {
  if (!v) {
    selected.value = null;
    return;
  }
  const [cx, cz] = v.split(",").map(Number);
  selected.value = { cx, cz };
}

async function loadChunks() {
  loading.value = true;
  selected.value = null;
  try {
    const r = await api.nbtListChunks(props.instanceId, props.world, dim.value);
    chunks.value = r.chunks;
  } catch (e) {
    message.error(String(e));
    chunks.value = [];
  } finally {
    loading.value = false;
  }
}

// 切维度要重新清点；地图模式下也顺便知道区块总数
watch(dim, loadChunks, { immediate: true });
</script>

<template>
  <div class="chunk-editor">
    <div class="bar">
      <span class="lbl">{{ t("nbt.dimension") }}</span>
      <NSelect v-model:value="dim" :options="dimOptions" size="small" style="width: 132px" />

      <div ref="viewSegRef" class="seg-group qx-seg qx-seg--sm">
        <div class="qx-seg__indicator" :style="viewSegStyle"></div>
        <button class="seg" :class="{ active: view === 'map' }" @click="view = 'map'">
          {{ t("nbt.mapView") }}
        </button>
        <button class="seg" :class="{ active: view === 'list' }" @click="view = 'list'">
          {{ t("nbt.listView") }}
        </button>
      </div>

      <template v-if="view === 'list'">
        <NSelect
          :value="chunkValue"
          :options="chunkOptions"
          size="small"
          filterable
          clearable
          virtual-scroll
          :placeholder="loading ? t('nbt.loading') : t('nbt.chunkPickHint')"
          style="min-width: 190px"
          @update:value="pickChunk"
        />
        <button class="mini-btn" :disabled="loading" @click="loadChunks">
          <IconRefresh /> {{ t("nbt.refresh") }}
        </button>
      </template>
      <span class="count">{{ t("nbt.chunkCount", { n: chunks.length }) }}</span>
    </div>

    <ChunkMap
      v-if="view === 'map'"
      ref="mapRef"
      :instance-id="instanceId"
      :world="world"
      :dim="dim"
      :selected="selected"
      @pick="(cx: number, cz: number) => (selected = { cx, cz })"
    />
    <div v-if="view === 'list' && !loading && !chunks.length" class="empty">
      {{ t("nbt.chunkEmpty") }}
    </div>

    <NbtTreeEditor
      v-if="selected"
      :key="`${dim}:${selected.cx}:${selected.cz}`"
      :instance-id="instanceId"
      :world="world"
      :running="running"
      :readonly="readonly"
      :chunk="{ dim, cx: selected.cx, cz: selected.cz }"
      @backed-up="onChunkSaved"
    />
    <div v-else-if="view === 'map'" class="hint">{{ t("nbt.mapHint") }}</div>
  </div>
</template>

<style scoped>
.chunk-editor {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.bar {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.lbl {
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.count {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
/* 地图 / 列表切换：外观与滑块走全局 .qx-seg--sm */
.empty,
.hint {
  padding: 24px;
  text-align: center;
  color: var(--text-3);
  font-size: var(--fs-xs);
}
.hint {
  padding: 8px;
}
</style>
