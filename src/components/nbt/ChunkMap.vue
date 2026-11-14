<script setup lang="ts">
/**
 * 区块地图：把存档里已生成的区块画成平面图（读的是存档真实地形，
 * 不是按种子推算）。点击地图上的位置可以直接跳到那个区块。
 *
 * 两个缓存层，避免"拖出去再拖回来又渲染一遍"：
 * 1. 前端把每次渲染的结果按世界坐标存成瓦片（离屏 canvas）。平移、缩放时
 *    先把已有瓦片按位置画出来，**视野被已有瓦片完全覆盖时直接不发请求**。
 * 2. 后端按 (维度, 区块) 缓存地表数据，只要有一格命中缓存就不读 region 文件。
 *
 * 缩放模型：`zoom` = 每个像素代表多少方块。视野超过 512 像素时让后端
 * 降采样（step），保证一次请求的图不超过 512×512。
 */
import { onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { useI18n } from "vue-i18n";
import { listen } from "@tauri-apps/api/event";
import { api } from "../../api";
import type { MapImageData } from "../../types";

const props = defineProps<{
  instanceId: string;
  world: string;
  dim: string;
  selected: { cx: number; cz: number } | null;
}>();
const emit = defineEmits<{ (e: "pick", cx: number, cz: number): void }>();

const { t } = useI18n();

/** 单次渲染的最大像素边长，跟后端 MAX_SIDE 对应 */
const MAX_SIDE = 512;
/** 采样粒度上限，跟后端 clamp 一致：缩到最远（1 像素 64 格）时整屏仍要在 512 像素内 */
const MAX_STEP = 256;
const HEIGHT = 420;
/** 打开时的默认缩放：一像素一格（视野小、解压的区块少，打开更快） */
const DEFAULT_ZOOM = 1;
/** 最多留几张瓦片（一张最大 512×512×4 ≈ 1MB） */
const MAX_TILES = 16;

/** 一张已渲染好的图，按世界坐标摆着 */
interface Tile {
  x0: number;
  z0: number;
  x1: number;
  z1: number;
  /** 每个像素代表多少方块 */
  step: number;
  /** 加入顺序：同一档粒度时后画的盖住先画的 */
  seq: number;
  canvas: HTMLCanvasElement;
}

const wrapRef = ref<HTMLDivElement | null>(null);
const canvasRef = ref<HTMLCanvasElement | null>(null);

const cssW = ref(720);
const zoom = ref(DEFAULT_ZOOM);
const centerX = ref(0);
const centerZ = ref(0);
const tiles = shallowRef<Tile[]>([]);
const rendering = ref(false);
/** 渲染进度百分比 */
const pct = ref(0);
const empty = ref(false);
let unlisten: (() => void) | null = null;
let resizeObs: ResizeObserver | null = null;
let pending: number | null = null;
let tileSeq = 0;
/** 请求序号：只有最新一次请求影响进度显示 */
let seq = 0;

/** 视野左上角的方块坐标 */
const viewX0 = () => centerX.value - (cssW.value * zoom.value) / 2;
const viewZ0 = () => centerZ.value - (HEIGHT * zoom.value) / 2;

/** palette + data → 离屏 canvas（下标 0 透明，表示未生成） */
function toOffscreen(im: MapImageData): HTMLCanvasElement {
  const cv = document.createElement("canvas");
  cv.width = im.width;
  cv.height = im.height;
  const ctx = cv.getContext("2d")!;
  const out = ctx.createImageData(im.width, im.height);
  const bytes = Uint8Array.from(atob(im.data), (c) => c.charCodeAt(0));
  for (let i = 0; i < im.width * im.height; i++) {
    const idx = bytes[i] ?? 0;
    if (idx === 0) continue; // 未生成：留透明
    const p = idx * 3;
    out.data[i * 4] = im.palette[p] ?? 128;
    out.data[i * 4 + 1] = im.palette[p + 1] ?? 128;
    out.data[i * 4 + 2] = im.palette[p + 2] ?? 128;
    out.data[i * 4 + 3] = 255;
  }
  ctx.putImageData(out, 0, 0);
  return cv;
}

function addTile(r: MapImageData) {
  const tile: Tile = {
    x0: r.originX,
    z0: r.originZ,
    x1: r.originX + r.width * r.step,
    z1: r.originZ + r.height * r.step,
    step: r.step,
    seq: ++tileSeq,
    canvas: toOffscreen(r),
  };
  const rest = tiles.value.filter(
    (o) =>
      o.step === tile.step &&
      !(o.x0 === tile.x0 && o.z0 === tile.z0 && o.x1 === tile.x1 && o.z1 === tile.z1)
  );
  // 只留同一粒度的瓦片：换粒度后旧图既不会被画出来，留着也是白占内存
  tiles.value = [...rest, tile].slice(-MAX_TILES);
}

function draw() {
  const cv = canvasRef.value;
  const ctx = cv?.getContext("2d");
  if (!cv || !ctx) return;
  const dpr = window.devicePixelRatio || 1;
  const pxW = Math.round(cssW.value * dpr);
  const pxH = Math.round(HEIGHT * dpr);
  if (cv.width !== pxW || cv.height !== pxH) {
    cv.width = pxW;
    cv.height = pxH;
  }
  // 全程按设备像素算、按整数画：之前用 CSS 坐标 + setTransform(dpr) 时，
  // 视野原点一被拖动就带小数，地形像素和网格线各自落在不同的半像素上，
  // 于是"格子对不齐"。现在地形格子边界和网格线共用同一套取整，必然重合。
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.imageSmoothingEnabled = false;
  ctx.fillStyle = "#101319";
  ctx.fillRect(0, 0, pxW, pxH);

  const { step } = viewRect();
  /** 方块 → 设备像素 */
  const scale = dpr / zoom.value;
  const ox = -viewX0() * scale;
  const oz = -viewZ0() * scale;

  // 只画当前粒度的瓦片。混着不同分辨率一起画时，低清瓦片会从高清瓦片没盖到的
  // 地方露出来，整张图就是"一块一块的平地 + 对不上的格子"。
  let list = tiles.value.filter((t) => t.step === step);
  if (!list.length) {
    // 切换粒度后新图还没回来，先用旧图垫着，免得缩放时闪空
    list = [...tiles.value];
  }
  list.sort((a, b) => a.seq - b.seq);
  for (const tile of list) {
    const x0 = Math.round(ox + tile.x0 * scale);
    const z0 = Math.round(oz + tile.z0 * scale);
    const x1 = Math.round(ox + tile.x1 * scale);
    const z1 = Math.round(oz + tile.z1 * scale);
    if (x1 <= 0 || z1 <= 0 || x0 >= pxW || z0 >= pxH) continue;
    ctx.drawImage(tile.canvas, x0, z0, x1 - x0, z1 - z0);
  }

  // 区块网格：一格在屏幕上够大才画
  const pxPerChunk = 16 * scale;
  if (pxPerChunk >= 9 * dpr) {
    const lw = Math.max(1, Math.round(dpr));
    const half = lw % 2 ? 0.5 : 0; // 奇数线宽压在整像素缝隙上才不发虚
    ctx.strokeStyle = "rgba(255,255,255,0.09)";
    ctx.lineWidth = lw;
    const c0x = Math.floor(viewX0() / 16);
    const c1x = Math.ceil((viewX0() + cssW.value * zoom.value) / 16);
    const c0z = Math.floor(viewZ0() / 16);
    const c1z = Math.ceil((viewZ0() + HEIGHT * zoom.value) / 16);
    ctx.beginPath();
    for (let cx = c0x; cx <= c1x; cx++) {
      const x = Math.round(ox + cx * 16 * scale) + half;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, pxH);
    }
    for (let cz = c0z; cz <= c1z; cz++) {
      const z = Math.round(oz + cz * 16 * scale) + half;
      ctx.moveTo(0, z);
      ctx.lineTo(pxW, z);
    }
    ctx.stroke();
  }

  // 当前选中的区块
  const sel = props.selected;
  if (sel) {
    const x = Math.round(ox + sel.cx * 16 * scale);
    const z = Math.round(oz + sel.cz * 16 * scale);
    const w = Math.round(ox + (sel.cx * 16 + 16) * scale) - x;
    const h = Math.round(oz + (sel.cz * 16 + 16) * scale) - z;
    if (x + w > 0 && z + h > 0 && x < pxW && z < pxH) {
      ctx.strokeStyle =
        getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() ||
        "#e89a4b";
      ctx.lineWidth = Math.max(2, Math.round(2 * dpr));
      const pad = Math.round(ctx.lineWidth / 2);
      ctx.strokeRect(x + pad, z + pad, w - pad * 2, h - pad * 2);
    }
  }
}

/** 当前视野对应的请求范围 */
function viewRect() {
  const vw = cssW.value * zoom.value;
  const vh = HEIGHT * zoom.value;
  // 留 2 像素余量：范围会对齐到 step，正好卡在 512 会被后端拒绝
  const step = Math.min(
    MAX_STEP,
    Math.max(1, Math.ceil(Math.max(vw, vh) / (MAX_SIDE - 2)))
  );
  const x0 = Math.floor(viewX0() / step) * step;
  const z0 = Math.floor(viewZ0() / step) * step;
  const x1 = x0 + Math.max(1, Math.ceil((viewX0() + vw - x0) / step)) * step;
  const z1 = z0 + Math.max(1, Math.ceil((viewZ0() + vh - z0) / step)) * step;
  return { x0, z0, x1, z1, step };
}

/** 已经有瓦片（同一粒度）把整个视野盖住了吗 */
function covered(r: { x0: number; z0: number; x1: number; z1: number; step: number }) {
  return tiles.value.some(
    (t) =>
      t.step === r.step && t.x0 <= r.x0 && t.z0 <= r.z0 && t.x1 >= r.x1 && t.z1 >= r.z1
  );
}

/** 拉取当前视野（带 120ms 防抖，拖拽时不会每一帧都请求） */
function scheduleRender() {
  if (pending) window.clearTimeout(pending);
  pending = window.setTimeout(render, 120);
}

async function render() {
  const r = viewRect();
  // 视野里已经有图了，直接画，不再请求
  if (covered(r)) {
    draw();
    return;
  }

  const mine = ++seq;
  rendering.value = true;
  pct.value = 0;
  try {
    const data = await api.nbtRenderMap(
      props.instanceId,
      props.world,
      props.dim,
      r.x0,
      r.z0,
      r.x1,
      r.z1,
      r.step
    );
    // 每次返回都是对应范围的有效图，都存下来
    addTile(data);
    draw();
  } catch {
    // 渲染失败（比如范围过大）不打断交互，保持已有的图。
    // 但如果连一张当前粒度的图都没有，画布上剩下的可能是上一个世界/上一档
    // 缩放的残留，宁可清空也别让人看着一张假地图。
    if (!tiles.value.some((t) => t.step === r.step)) {
      tiles.value = [];
      draw();
    }
  } finally {
    if (mine === seq) {
      rendering.value = false;
      pct.value = 0;
    }
  }
}

/** 定位到存档：整块外框塞进画布（存档跨度大时会很空） */
async function fitToSave() {
  try {
    const b = await api.nbtMapBounds(props.instanceId, props.world, props.dim);
    empty.value = !!b.empty;
    if (!b.empty) {
      centerX.value = (((b.minCx ?? 0) + (b.maxCx ?? 0)) / 2) * 16 + 8;
      centerZ.value = (((b.minCz ?? 0) + (b.maxCz ?? 0)) / 2) * 16 + 8;
      const spanX = ((b.maxCx ?? 0) - (b.minCx ?? 0) + 1) * 16;
      const spanZ = ((b.maxCz ?? 0) - (b.minCz ?? 0) + 1) * 16;
      zoom.value = Math.min(
        64,
        Math.max(0.25, Math.max(spanX / cssW.value, spanZ / HEIGHT))
      );
    }
  } catch {
    empty.value = false;
  }
  draw();
  await render();
}

/** 打开时落在区块最密集的地方（一般是玩家主要活动区），而不是整张外框的中心 */
async function openInitialView() {
  try {
    const b = await api.nbtMapBounds(props.instanceId, props.world, props.dim);
    empty.value = !!b.empty;
    if (!b.empty && b.denseX !== undefined && b.denseZ !== undefined) {
      centerX.value = b.denseX;
      centerZ.value = b.denseZ;
      zoom.value = DEFAULT_ZOOM;
    }
  } catch {
    empty.value = false;
  }
  draw();
  await render();
}

/** 区块数据被改过：丢掉落盘的旧图重画 */
function refresh() {
  tiles.value = [];
  draw();
  render();
}

// ---- 交互 ----
let dragging = false;
let moved = 0;
let lastX = 0;
let lastY = 0;

function onDown(e: MouseEvent) {
  dragging = true;
  moved = 0;
  lastX = e.clientX;
  lastY = e.clientY;
}

function onMove(e: MouseEvent) {
  if (!dragging) return;
  const dx = e.clientX - lastX;
  const dy = e.clientY - lastY;
  moved += Math.abs(dx) + Math.abs(dy);
  lastX = e.clientX;
  lastY = e.clientY;
  centerX.value -= dx * zoom.value;
  centerZ.value -= dy * zoom.value;
  draw();
  scheduleRender();
}

function onUp(e: MouseEvent) {
  if (!dragging) return;
  dragging = false;
  if (moved > 4) return; // 拖过就不算点击
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  // 画布 CSS 宽度和测量值可能有零点几像素的差，按比例换算一下，
  // 否则点选到的区块会和看到的位置差一格（"明明有地形却提示尚未生成"）
  const k = rect.width > 0 ? cssW.value / rect.width : 1;
  const wx = viewX0() + (e.clientX - rect.left) * k * zoom.value;
  const wz = viewZ0() + (e.clientY - rect.top) * k * zoom.value;
  emit("pick", Math.floor(wx / 16), Math.floor(wz / 16));
}

function onWheel(e: WheelEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const k = rect.width > 0 ? cssW.value / rect.width : 1;
  const bx = viewX0() + (e.clientX - rect.left) * k * zoom.value;
  const bz = viewZ0() + (e.clientY - rect.top) * k * zoom.value;
  const next = Math.min(64, Math.max(0.25, zoom.value * (e.deltaY > 0 ? 1.25 : 0.8)));
  // 以鼠标位置为锚点缩放
  centerX.value = bx + (centerX.value - bx) * (next / zoom.value);
  centerZ.value = bz + (centerZ.value - bz) * (next / zoom.value);
  zoom.value = next;
  draw();
  scheduleRender();
}

function zoomBy(f: number) {
  zoom.value = Math.min(64, Math.max(0.25, zoom.value * f));
  draw();
  scheduleRender();
}

function measure() {
  if (!wrapRef.value) return;
  const w = wrapRef.value.clientWidth;
  // 差 1 像素也要重算：画布宽度和这里记的宽度对不上时，点选的区块会整体偏一格
  if (w > 0 && Math.abs(w - cssW.value) > 0.5) {
    cssW.value = w;
    draw();
    scheduleRender();
  }
}

defineExpose({ refresh });

watch(
  // 实例也要带上：不同实例里同名的世界（"新的世界" 这种）目录名完全一样，
  // 只盯着 world 的话切换实例不会触发重置，地图上会一直挂着上一个实例的地形
  () => [props.instanceId, props.world, props.dim].join("|"),
  async () => {
    tiles.value = [];
    await openInitialView();
  }
);
watch(() => props.selected, draw);

onMounted(async () => {
  measure();
  resizeObs = new ResizeObserver(measure);
  if (wrapRef.value) resizeObs.observe(wrapRef.value);
  unlisten = await listen<{ current: number; total: number }>("nbt-progress", (e) => {
    if (!rendering.value) return;
    const { current, total } = e.payload;
    if (total > 0) pct.value = Math.min(100, Math.round((current / total) * 100));
  });
  await openInitialView();
});

onBeforeUnmount(() => {
  unlisten?.();
  resizeObs?.disconnect();
  if (pending) window.clearTimeout(pending);
  seq++; // 让在途请求的结果失效
  tiles.value = [];
});
</script>

<template>
  <div ref="wrapRef" class="chunk-map">
    <canvas
      ref="canvasRef"
      :style="{ width: '100%', height: HEIGHT + 'px' }"
      @mousedown="onDown"
      @mousemove="onMove"
      @mouseup="onUp"
      @mouseleave="onUp"
      @wheel.prevent="onWheel"
    />

    <div class="ctrl">
      <button class="mini-btn icon-only solid" :title="t('nbt.mapZoomOut')" @click="zoomBy(1.25)">−</button>
      <button class="mini-btn icon-only solid" :title="t('nbt.mapZoomIn')" @click="zoomBy(0.8)">+</button>
      <button class="mini-btn solid" @click="fitToSave">{{ t("nbt.mapReset") }}</button>
    </div>

    <!-- 渲染进度：顶部细进度条 + 左下角百分比 -->
    <div v-if="rendering" class="topbar">
      <i :style="{ width: pct + '%' }" />
    </div>

    <div v-if="rendering" class="badge">{{ t("nbt.mapRendering") }} {{ pct }}%</div>
    <div v-else-if="empty" class="badge">{{ t("nbt.chunkEmpty") }}</div>
    <div v-else class="scale">
      {{ t("nbt.mapScale", { n: zoom < 1 ? (1 / zoom).toFixed(1) : Math.round(zoom) }) }}
    </div>
  </div>
</template>

<style scoped>
.chunk-map {
  position: relative;
  border-radius: var(--r-md);
  overflow: hidden;
  background: #101319;
}
canvas {
  display: block;
  cursor: crosshair;
}
.ctrl {
  position: absolute;
  top: 8px;
  right: 8px;
  display: flex;
  gap: 4px;
}
.badge,
.scale {
  position: absolute;
  left: 8px;
  bottom: 8px;
  padding: 3px 8px;
  border-radius: var(--r-sm);
  background: rgba(20, 22, 28, 0.82);
  color: var(--text-3);
  font-size: var(--fs-2xs);
}
.topbar {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 3px;
  background: rgba(255, 255, 255, 0.08);
}
.topbar i {
  display: block;
  height: 100%;
  background: var(--accent);
  transition: width 0.15s ease;
}
</style>
