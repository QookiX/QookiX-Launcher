<script setup lang="ts">
import { staggeredCardDelay } from "../composables/staggeredCardDelay";
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { IconMapPin, IconBox, IconEdit, IconImage } from "../components/icons";
import seedCover from "../assets/seed-cover.png";
import schematicCover from "../assets/schematic-cover.png";
import nbtCover from "../assets/nbt-cover.png";
import resourcepackCover from "../assets/resourcepack-cover.png";

const { t } = useI18n();

const tools = computed(() => [
  {
    key: "seed",
    title: t("toolbox.seedMapTitle"),
    desc: t("toolbox.seedMapDesc"),
    icon: IconMapPin,
    to: "/toolbox/seed",
    cover: "linear-gradient(135deg, #5aa2f0, #2f6bbd)",
    coverImg: seedCover,
    coverPos: "center",
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
    coverPos: "center",
    beta: false,
  },
  {
    key: "resourcepack",
    title: t("toolbox.resourcePackTitle"),
    desc: t("toolbox.resourcePackDesc"),
    icon: IconImage,
    to: "/toolbox/resourcepack",
    cover: "linear-gradient(135deg, #d8a24a, #8c5a24)",
    coverImg: resourcepackCover,
    // 原图 420×260，卡片按宽度铺满后上下各裁一截；居中正好露出角色、石阶与水面
    coverPos: "center",
    beta: false,
  },
  {
    key: "nbt",
    title: t("toolbox.nbtTitle"),
    desc: t("toolbox.nbtDesc"),
    icon: IconEdit,
    to: "/toolbox/nbt",
    cover: "linear-gradient(135deg, #2f8f6b, #1d5c45)",
    coverImg: nbtCover,
    // 原图 420×260，卡片封面按宽度铺满后上下会裁掉一截；
    // 默认 center 只露到海面和浅滩，把焦点压到下方才看得到森林与海岸线
    coverPos: "50% 82%",
    beta: true,
  },
]);

</script>

<template>
  <div id="toolbox-root" class="qx-page" data-tauri-drag-region>
    <div class="qx-grid qx-grid--card">
      <RouterLink
        v-for="(tool, i) in tools"
        :key="tool.key"
        class="tool-card glass clickable qx-card-enter"
        :style="{ animationDelay: staggeredCardDelay(i, tools.length) }"
        :to="tool.to"
      >
        <div class="tool-cover" :style="tool.coverImg ? { backgroundImage: `url(${tool.coverImg})`, backgroundPosition: tool.coverPos } : { background: tool.cover }">
          <component :is="tool.icon" class="tool-cover-icon" />
          <span v-if="tool.beta" class="beta-tag">BETA</span>
        </div>
        <div class="tool-body">
          <h3>{{ tool.title }}</h3>
          <p class="hint">{{ tool.desc }}</p>
        </div>
      </RouterLink>
    </div>
  </div>
</template>

<style scoped>
.tool-card {
  color: inherit;
  text-decoration: none;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  transition: box-shadow var(--dur-md) var(--ease-out);
}
/* 与 .inst-card / .pin-card / .server-card 同一套悬停反馈：
   此前这里没有任何抬升，同屏并排的卡片里只有它是「点下去没反应」的。 */
.tool-card:hover:not(:active) {
  box-shadow: var(--card-shadow-hover);
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
  border-radius: var(--r-xs);
  font-size: var(--fs-2xs);
  font-weight: 700;
  letter-spacing: 0.08em;
  line-height: 1.4;
  color: var(--accent);
  background: var(--accent-16);
  border: 1px solid var(--accent-35);
  backdrop-filter: blur(6px);
}
.tool-body { padding: 16px 18px 18px; }
.tool-body h3 { margin: 0 0 10px; font-size: var(--fs-md); font-weight: 600; }
.hint { margin: 0; font-size: var(--fs-xs); color: var(--text-3); line-height: 1.6; }
</style>
