<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { parseMcMotd, motdHasObfuscated, type MotdSpan } from "../utils/mcMotd";

const props = defineProps<{
  /** 原始 MOTD 字符串（含 § / & 格式化码，可多行）。 */
  text: string;
  /** text 为空时显示的占位文本。 */
  defaultText?: string;
}>();

const source = computed(() => {
  let raw = props.text ?? "";
  if (raw.trim().length === 0) raw = props.defaultText ?? "";
  // 兼容 Java properties 转义：\u00A7 → §、\n → 换行。后端列表已解码，
  // 这里主要给 server.properties 表单的实时预览兜底（那处传的是文件原始值）。
  raw = raw.replace(/\\u00a7/gi, "§").replace(/\\n/g, "\n");
  return raw;
});

const lines = computed<MotdSpan[][]>(() => parseMcMotd(source.value));
const hasObf = computed(() => motdHasObfuscated(lines.value));

// §k 混淆：像 MC 客户端那样把字符替换成同宽随机字符并不断翻动。
const OBF_POOL = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
function scramble(text: string): string {
  let out = "";
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    // 保留空格，其余替换为随机字符
    out += ch === " " ? ch : OBF_POOL[Math.floor(Math.random() * OBF_POOL.length)];
  }
  return out;
}

const tick = ref(0);
let timer: ReturnType<typeof setInterval> | null = null;
function ensureTimer(need: boolean) {
  if (need && !timer) timer = setInterval(() => tick.value++, 90);
  else if (!need && timer) {
    clearInterval(timer);
    timer = null;
  }
}
onMounted(() => ensureTimer(hasObf.value));
onBeforeUnmount(() => ensureTimer(false));
watch(hasObf, (v) => ensureTimer(v));

// 依赖 tick 触发重算，仅混淆段会变；普通段保持稳定文本。
const rendered = computed(() => {
  void tick.value;
  return lines.value.map((line) =>
    line.map((s) => ({ ...s, display: s.obfuscated ? scramble(s.text) : s.text })),
  );
});

function spanStyle(s: MotdSpan): Record<string, string> {
  const st: Record<string, string> = {};
  if (s.color) st.color = s.color;
  if (s.bold) st.fontWeight = "700";
  if (s.italic) st.fontStyle = "italic";
  const deco: string[] = [];
  if (s.underline) deco.push("underline");
  if (s.strikethrough) deco.push("line-through");
  if (deco.length) st.textDecoration = deco.join(" ");
  return st;
}
</script>

<template>
  <span class="mc-motd">
    <span v-for="(line, li) in rendered" :key="li" class="mc-motd-line">
      <span
        v-for="(s, si) in line"
        :key="si"
        :style="spanStyle(s)"
        :class="{ obf: s.obfuscated }"
      >{{ s.display }}</span>
    </span>
  </span>
</template>

<style scoped>
.mc-motd {
  display: block;
  min-width: 0;
}
.mc-motd-line {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mc-motd-line + .mc-motd-line {
  margin-top: 2px;
}
/* 混淆段用等宽字体，随机字符宽度一致，翻动时不抖动 */
.obf {
  font-family: ui-monospace, "Cascadia Mono", "Consolas", monospace;
}
</style>
