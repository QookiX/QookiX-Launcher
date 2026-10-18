<script setup lang="ts">
import { computed } from "vue";
import { useRouter } from "vue-router";
import { useI18n } from "vue-i18n";
import { IconMapPin, IconBox, IconEdit } from "../components/icons";
import seedCover from "../assets/seed-cover.png";
import schematicCover from "../assets/schematic-cover.png";

const { t } = useI18n();
const router = useRouter();

const tools = computed(() => [
  {
    key: "seed",
    title: t("toolbox.seedMapTitle"),
    desc: t("toolbox.seedMapDesc"),
    icon: IconMapPin,
    to: "/toolbox/seed",
    cover: "linear-gradient(135deg, #5aa2f0, #2f6bbd)",
    coverImg: seedCover,
    beta: false,
  },
  {
    key: "schematic",
    title: t("toolbox.schematicTitle"),
    desc: t("toolbox.schematicDesc"),
    icon: IconBox,
    to: "/toolbox/schematic",
    cover: "linear-gradient(135deg, #6b5b95, #352f44)",
    coverImg: schematicCover,
    beta: false,
  },
  {
    key: "nbt",
    title: t("toolbox.nbtTitle"),
    desc: t("toolbox.nbtDesc"),
    icon: IconEdit,
    to: "/toolbox/nbt",
    cover: "linear-gradient(135deg, #2f8f6b, #1d5c45)",
    coverImg: null as string | null,
    beta: true,
  },
]);

function go(to: string) {
  router.push(to);
}

// 工具卡片逐个进场（同首页 pin-card / 内容页卡片的做法）：延迟由内联 style 下发。
// 数量多时自动收紧步长，整批级联 ~0.7s 内跑完。
function cardDelay(i: number): string {
  const step = Math.min(0.045, 0.6 / Math.max(1, tools.value.length));
  return `${(0.06 + i * step).toFixed(3)}s`;
}
</script>

<template>
  <div id="toolbox-root" class="toolbox-view">
    <div class="grid">
      <section
        v-for="(tool, i) in tools"
        :key="tool.key"
        class="tool-card glass clickable"
        :style="{ animationDelay: cardDelay(i) }"
        @click="go(tool.to)"
      >
        <div class="tool-cover" :style="tool.coverImg ? { backgroundImage: `url(${tool.coverImg})` } : { background: tool.cover }">
          <component :is="tool.icon" class="tool-cover-icon" />
          <span v-if="tool.beta" class="beta-tag">BETA</span>
        </div>
        <div class="tool-body">
          <h3>{{ tool.title }}</h3>
          <p class="hint">{{ tool.desc }}</p>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.toolbox-view { display: flex; flex-direction: column; gap: 16px; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 16px; }
.tool-card {
  overflow: hidden;
  display: flex;
  flex-direction: column;
  /* 卡片逐个进场（同首页 pin-card：translateY 28px 上浮 + 淡入，backwards 让
     未出场的卡片先保持隐藏）。延迟由模板内联 animationDelay 提供。 */
  animation: tool-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
@keyframes tool-card-enter {
  from {
    transform: translateY(28px);
    opacity: 0;
  }
  to {
    transform: none;
    opacity: 1;
  }
}
.tool-cover {
  height: 124px;
  display: flex;
  align-items: center;
  justify-content: center;
  position: relative;
  background-size: cover;
  background-position: center;
}
.tool-cover-icon {
  font-size: 46px;
  color: rgba(255, 255, 255, 0.96);
  filter:
    drop-shadow(0 0 2px rgba(0, 0, 0, 0.85))
    drop-shadow(0 0 1px rgba(0, 0, 0, 0.85))
    drop-shadow(0 3px 12px rgba(0, 0, 0, 0.5));
}
.beta-tag {
  position: absolute;
  top: 10px;
  right: 10px;
  padding: 2px 7px;
  border-radius: 6px;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.08em;
  line-height: 1.4;
  color: var(--accent);
  background: var(--accent-16);
  border: 1px solid var(--accent-35);
  backdrop-filter: blur(6px);
}
.tool-body { padding: 16px 18px 18px; }
.tool-body h3 { margin: 0 0 10px; font-size: 15px; font-weight: 600; }
.hint { margin: 0; font-size: 12px; color: var(--text-3); line-height: 1.6; }
</style>
