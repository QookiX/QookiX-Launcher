<script setup lang="ts">
import { vSegmented } from "../composables/vSegmented";
/**
 * NBT 存档编辑器。
 * 三种视图：世界属性（表单 / 树形）、玩家数据、背包物品。
 * 安全设计：写回只改提交的字段（后端保证未知字段不丢），
 * 保存前自动单文件备份，游戏运行中或只读时不写盘。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { NButton, NCollapse, NCollapseItem, NInput, NInputNumber, NModal, NProgress, NSelect, NSwitch, useDialog, useMessage } from "naive-ui";
import { useTauriListenerScope } from "../composables/tauriListeners";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import { useInstancesStore } from "../stores/instances";
import { fmtDate, fmtSize } from "../utils/format";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import type { PlayerSummary, WorldForm, WorldSummary } from "../types";
import {
  IconBox,
  IconFolder,
  IconInfo,
  IconRefresh,
  IconRestore,
  IconSave,
  IconUser,
} from "../components/icons";
import PlayerEditor from "../components/nbt/PlayerEditor.vue";
import ItemEditor from "../components/nbt/ItemEditor.vue";
import NbtTreeEditor from "../components/nbt/NbtTreeEditor.vue";
import ChunkEditor from "../components/nbt/ChunkEditor.vue";
import BackupPanel from "../components/nbt/BackupPanel.vue";

const { listen } = useTauriListenerScope();

const { t, te } = useI18n();
const message = useMessage();
const dialog = useDialog();
const instances = useInstancesStore();

const MODE_KEY = "qookix:nbt:mode";
const TAB_KEY = "qookix:nbt:tab";
/** 树形模式的操作说明只自动弹一次 */
const HELP_KEY = "qookix:nbt:treeHelp";

type Mode = "form" | "tree";
type Tab = "world" | "chunk" | "player" | "item";

const instanceId = ref("");
const worlds = ref<WorldSummary[]>([]);
const loadingWorlds = ref(false);
const current = ref<WorldSummary | null>(null);
/** 手动指定的世界（不在实例 saves 下） */
const manual = ref(false);

const mode = ref<Mode>((localStorage.getItem(MODE_KEY) as Mode) || "form");
const tab = ref<Tab>((localStorage.getItem(TAB_KEY) as Tab) || "world");

const form = ref<WorldForm | null>(null);
const original = ref<WorldForm | null>(null);
const running = ref(false);
const editable = ref(true);
const readonlyReason = ref<string | null>(null);
const saving = ref(false);
const lastBackup = ref("");

const players = ref<PlayerSummary[]>([]);
const activePlayer = ref<PlayerSummary | null>(null);
const backupShow = ref(false);
const helpShow = ref(false);

/**
 * 当前正在进行的耗时操作。
 * 存档读写是本地磁盘操作，世界大时会有几秒无响应，
 * 这里给用户看到"正在做什么 + 进度"，避免误以为程序卡死。
 */
const busy = ref<{
  kind: "scan" | "work";
  label: string;
  detail: string;
  pct: number | null;
} | null>(null);
let unlisten: (() => void) | null = null;

function startBusy(kind: "scan" | "work", label: string) {
  busy.value = { kind, label, detail: "", pct: kind === "scan" ? 0 : null };
}
function endBusy() {
  busy.value = null;
}

/** 常用游戏规则（布尔）：值在 level.dat 里是字符串 "true"/"false" */
const BOOL_RULES = [
  "keepInventory",
  "doDaylightCycle",
  "doWeatherCycle",
  "doMobSpawning",
  "mobGriefing",
  "doFireTick",
  "doTileDrops",
  "doMobLoot",
  "naturalRegeneration",
  "doInsomnia",
  "disableRaids",
  "fallDamage",
  "fireDamage",
  "pvp",
  "doImmediateRespawn",
  "showCoordinates",
  "announceAdvancements",
  "commandBlockOutput",
];
/** 数值型规则（用输入框） */
const NUMERIC_RULES = [
  "randomTickSpeed",
  "spawnRadius",
  "playersSleepingPercentage",
  "maxEntityCramming",
];

const isBoolRule = (v: string | undefined) => v === "true" || v === "false";
/** 规则名 → 大白话短名；没收录的规则直接显示原本的键名 */
function ruleLabel(key: string) {
  const k = `nbt.gamerule.${key}`;
  return te(k) ? t(k) : key;
}
/** 白名单之外的全部规则（折叠区展示） */
const otherRules = computed(() => {
  const rules = form.value?.gameRules;
  if (!rules) return [];
  return Object.keys(rules)
    .filter((k) => !BOOL_RULES.includes(k) && !NUMERIC_RULES.includes(k))
    .sort();
});
function setRule(key: string, value: string) {
  if (!form.value?.gameRules) return;
  form.value.gameRules[key] = value;
}
const gameModeOptions = computed(() => [
  { label: t("nbt.modeSurvival"), value: 0 },
  { label: t("nbt.modeCreative"), value: 1 },
  { label: t("nbt.modeAdventure"), value: 2 },
  { label: t("nbt.modeSpectator"), value: 3 },
]);
const difficultyOptions = computed(() => [
  { label: t("nbt.diffPeaceful"), value: 0 },
  { label: t("nbt.diffEasy"), value: 1 },
  { label: t("nbt.diffNormal"), value: 2 },
  { label: t("nbt.diffHard"), value: 3 },
]);
const yesNoOptions = computed(() => [
  { label: t("nbt.no"), value: 0 },
  { label: t("nbt.yes"), value: 1 },
]);
const instanceOptions = computed(() =>
  instances.instances.map((i) => ({ label: `${i.name}（${i.mc_version}）`, value: i.id }))
);
const noInstance = computed(() => !instanceId.value);

async function loadWorlds() {
  if (!instanceId.value) {
    worlds.value = [];
    return;
  }
  loadingWorlds.value = true;
  startBusy("scan", t("nbt.scanningWorlds"));
  try {
    const r = await api.nbtListWorlds(instanceId.value);
    worlds.value = r.worlds;
  } catch (e) {
    message.error(String(e));
    worlds.value = [];
  } finally {
    loadingWorlds.value = false;
    endBusy();
  }
}

async function openWorld(w: WorldSummary) {
  current.value = w;
  lastBackup.value = "";
  activePlayer.value = null;
  startBusy("work", t("nbt.readingWorld"));
  try {
    const r = await api.nbtOpenWorld(instanceId.value, w.dir);
    form.value = r.form;
    original.value = JSON.parse(JSON.stringify(r.form));
    running.value = r.running;
    editable.value = r.editable;
    readonlyReason.value = r.reason;
    if (r.running) message.warning(t("nbt.runningWarn"));
    else if (r.reason) message.warning(r.reason);
    // 手动打开时卡片元信息是空的，用 level.dat 补上
    if (w.levelName == null) w.levelName = r.form.levelName;
    startBusy("work", t("nbt.readingPlayers"));
    const pl = await api.nbtListPlayers(instanceId.value, w.dir);
    players.value = pl.players;
    if (pl.warnings.length) message.warning(pl.warnings.slice(0, 3).join("\n"));
  } catch (e) {
    message.error(String(e));
    current.value = null;
  } finally {
    endBusy();
  }
}

/** 手动指定世界目录（无实例或存档不在实例目录下时用） */
async function openOtherWorld() {
  const picked = await open({ directory: true, multiple: false, title: t("nbt.openOther") });
  if (!picked || Array.isArray(picked)) return;
  const path = String(picked);
  manual.value = true;
  await openWorld({
    dir: path,
    levelName: null,
    gameType: null,
    difficulty: null,
    seed: null,
    size: 0,
    modified: 0,
  });
}

/** 卡片 / 标题上显示的世界名：目录名（手动选择时是路径的最后一段） */
function worldLabel(w: WorldSummary) {
  return w.levelName || w.dir.split(/[\\/]/).filter(Boolean).pop() || w.dir;
}

function back() {
  current.value = null;
  form.value = null;
  activePlayer.value = null;
  manual.value = false;
}

const modeSegRef = ref<HTMLElement | null>(null);
const tabSegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: modeSegStyle, refresh: refreshModeSeg } = useSlidingIndicator(
  modeSegRef,
  () => Array.from(modeSegRef.value?.querySelectorAll<HTMLElement>("button") ?? []),
  () => (mode.value === "form" ? 0 : 1)
);
const { indicatorStyle: tabSegStyle, refresh: refreshTabSeg } = useSlidingIndicator(
  tabSegRef,
  () => Array.from(tabSegRef.value?.querySelectorAll<HTMLElement>("button") ?? []),
  () => (["world", "chunk", "player", "item"] as Tab[]).indexOf(tab.value)
);
watch(mode, () => nextTick(() => refreshModeSeg()));
watch(tab, () => {
  nextTick(() => refreshTabSeg());
  if (tab.value === "world") nextTick(() => refreshModeSeg());
});

function setMode(m: Mode) {
  mode.value = m;
  localStorage.setItem(MODE_KEY, m);
  // 第一次切到树形模式，主动弹一次操作说明
  if (m === "tree" && !localStorage.getItem(HELP_KEY)) {
    localStorage.setItem(HELP_KEY, "1");
    helpShow.value = true;
  }
}
function setTab(x: Tab) {
  tab.value = x;
  localStorage.setItem(TAB_KEY, x);
}

/** 确认摘要和提交补丁使用同一组改动。key 用真实 NBT 字段名供树形模式高亮。 */
const savePlan = computed(() => {
  const changes: { label: string; from: string; to: string; key: string }[] = [];
  const patch: Record<string, unknown> = {};
  const before = original.value;
  const after = form.value;
  if (!before || !after) return { changes, patch };

  const text = (value: unknown) => String(value ?? "-");
  const record = <T,>(key: string, label: string, oldValue: T, newValue: T, format: (value: T) => string = text) => {
    if (oldValue === newValue) return false;
    changes.push({ key, label, from: format(oldValue), to: format(newValue) });
    return true;
  };
  const diff = <T,>(key: string, label: string, oldValue: T, newValue: T, format?: (value: T) => string) => {
    if (record(key, label, oldValue, newValue, format)) patch[key] = newValue;
  };
  const optionLabel = (choices: { label: string; value: number }[]) => (value: number | null) =>
    choices.find((option) => option.value === value)?.label ?? text(value);

  diff("LevelName", t("nbt.levelName"), before.levelName, after.levelName);
  diff(
    "GameType",
    t("nbt.gameMode"),
    before.gameType,
    after.gameType,
    optionLabel(gameModeOptions.value),
  );
  diff(
    "Difficulty",
    t("nbt.difficulty"),
    before.difficulty,
    after.difficulty,
    optionLabel(difficultyOptions.value),
  );
  diff("RandomSeed", t("nbt.seed"), before.seed, after.seed);
  diff("Time", t("nbt.time"), before.time, after.time);
  diff("DayTime", t("nbt.dayTime"), before.dayTime, after.dayTime);
  diff("raining", t("nbt.weatherRain"), before.raining, after.raining);
  diff("thundering", t("nbt.weatherThunder"), before.thundering, after.thundering);
  diff("SpawnX", t("nbt.spawn"), before.spawnX, after.spawnX);
  diff("SpawnY", t("nbt.spawn"), before.spawnY, after.spawnY);
  diff("SpawnZ", t("nbt.spawn"), before.spawnZ, after.spawnZ);
  diff("allowCommands", t("nbt.allowCheats"), before.allowCommands, after.allowCommands);
  diff("DifficultyLocked", t("nbt.lockDifficulty"), before.difficultyLocked, after.difficultyLocked);
  diff("hardcore", t("nbt.hardcore"), before.hardcore, after.hardcore);

  const gameRules = Object.fromEntries(
    Object.entries(after.gameRules ?? {}).filter(([key, value]) =>
      record(key, ruleLabel(key), before.gameRules?.[key], value),
    ),
  );
  if (Object.keys(gameRules).length) patch.gameRules = gameRules;

  if (before.border && after.border) {
    const border: Partial<NonNullable<WorldForm["border"]>> = {};
    const fields = [
      ["centerX", "CenterX", t("nbt.center")],
      ["centerZ", "CenterZ", t("nbt.center")],
      ["size", "Size", t("nbt.size")],
    ] as const;
    for (const [property, field, label] of fields) {
      const key = after.borderFlat ? `Border${field}` : field;
      if (record(key, label, before.border[property], after.border[property])) {
        border[property] = after.border[property];
      }
    }
    if (Object.keys(border).length) patch.border = border;
  }
  return { changes, patch };
});

const changes = computed(() => savePlan.value.changes);

function setWeather(kind: "clear" | "rain" | "thunder") {
  if (!form.value) return;
  form.value.raining = kind === "clear" ? 0 : 1;
  form.value.thundering = kind === "thunder" ? 1 : 0;
}
function setTime(preset: "sunrise" | "noon" | "sunset" | "midnight") {
  if (!form.value) return;
  const map = { sunrise: 0, noon: 6000, sunset: 12000, midnight: 18000 };
  form.value.dayTime = String(map[preset]);
}

async function save() {
  if (!form.value || !current.value || saving.value) return;
  if (running.value) {
    message.warning(t("nbt.runningWarn"));
    return;
  }
  if (!editable.value) {
    message.warning(readonlyReason.value || t("nbt.readonlyWarn"));
    return;
  }
  const { changes: list, patch } = savePlan.value;
  if (!list.length) {
    message.info(t("nbt.noChanges"));
    return;
  }
  const target = current.value;
  const targetInstance = instanceId.value;
  const submittedForm = JSON.stringify(form.value);
  saving.value = true;
  try {
    await api.nbtValidateWorldPatch(targetInstance, target.dir, patch);
  } catch (error) {
    message.error(String(error));
    return;
  } finally {
    saving.value = false;
  }
  if (current.value !== target || instanceId.value !== targetInstance) return;
  dialog.warning({
    title: t("nbt.confirmTitle"),
    content: () =>
      `${t("nbt.confirmSummary")}：\n` +
      list.map((c) => `• ${c.label}: ${c.from} → ${c.to}`).join("\n") +
      `\n\n${t("nbt.backupNotice")}`,
    positiveText: t("nbt.save"),
    negativeText: t("nbt.reset"),
    onPositiveClick: async () => {
      if (current.value !== target || instanceId.value !== targetInstance) return;
      saving.value = true;
      startBusy("work", t("nbt.writingSave"));
      try {
        // 落盘前再确认一次可写：文件可能在打开之后被改成只读
        const chk = await api.nbtCanEdit(targetInstance, target.dir);
        if (current.value !== target || instanceId.value !== targetInstance) return;
        if (!chk.editable) {
          editable.value = false;
          readonlyReason.value = chk.reason;
          message.warning(chk.reason || t("nbt.readonlyWarn"));
          return;
        }
        const r = await api.nbtSaveWorld(targetInstance, target.dir, patch);
        if (current.value !== target || instanceId.value !== targetInstance) return;
        lastBackup.value = r.backup;
        original.value = r.form;
        if (JSON.stringify(form.value) === submittedForm) {
          form.value = JSON.parse(JSON.stringify(r.form));
        }
        message.success(t("nbt.saved"));
      } catch (e) {
        message.error(String(e));
      } finally {
        saving.value = false;
        endBusy();
      }
    },
  });
}

function resetForm() {
  if (original.value) form.value = JSON.parse(JSON.stringify(original.value));
}

function onBackedUp(name: string) {
  lastBackup.value = name;
}

/** 恢复备份后重新读取世界（表单与玩家列表都要刷新） */
async function onRestored() {
  if (current.value) await openWorld(current.value);
}

onMounted(async () => {
  // 后端扫描世界时逐个汇报进度（"正在扫描 我的世界 3/12"）
  unlisten = await listen<{ current: number; total: number; label: string }>(
    "nbt-progress",
    (e) => {
      if (busy.value?.kind !== "scan") return;
      const { current, total, label } = e.payload;
      busy.value.pct = total ? Math.round((current / total) * 100) : null;
      busy.value.detail = `${label} ${current}/${total}`;
    }
  );
  if (!instances.instances.length) await instances.load();
  const inst = instances.instances.find((i) => i.installed) ?? instances.instances[0];
  if (inst) instanceId.value = inst.id;
});

onBeforeUnmount(() => {
  unlisten?.();
  unlisten = null;
});

watch(instanceId, () => {
  current.value = null;
  form.value = null;
  manual.value = false;
  loadWorlds();
});
</script>

<template>
  <div class="nbt-view qx-page" data-tauri-drag-region>
    <div class="bar glass">
      <span class="bar-label">{{ t("nbt.selectInstance") }}</span>
      <NSelect
        v-model:value="instanceId"
        :options="instanceOptions"
        size="small"
        filterable
        clearable
        :placeholder="t('nbt.noInstanceOption')"
        style="min-width: 240px"
      />
      <button class="mini-btn" :disabled="loadingWorlds || !instanceId" @click="loadWorlds">
        <IconRefresh /> {{ loadingWorlds ? t("nbt.scanning") : t("nbt.refresh") }}
      </button>
      <div class="spacer" />
      <button class="mini-btn" @click="openOtherWorld">
        <IconFolder /> {{ t("nbt.openOther") }}
      </button>
    </div>

    <!-- 进行中的操作：进度条 + 正在做什么 -->
    <div v-if="busy" class="busy">
      <NProgress
        type="line"
        :percentage="busy.pct ?? 100"
        :processing="busy.pct === null"
        :height="6"
        :border-radius="3"
        :show-indicator="false"
      />
      <div class="busy-text">
        <span>{{ busy.label }}</span>
        <span v-if="busy.detail" class="busy-detail">{{ busy.detail }}</span>
      </div>
    </div>

    <!-- 世界列表 -->
    <div v-if="!current" class="world-list">
      <div v-if="noInstance" class="qx-empty glass">{{ t("nbt.needInstance") }}</div>
      <div v-else-if="loadingWorlds" class="qx-empty glass">{{ t("nbt.scanning") }}</div>
      <div v-else-if="!worlds.length" class="qx-empty glass">{{ t("nbt.noWorlds") }}</div>
      <div
        v-for="w in worlds"
        :key="w.dir"
        class="world-card glass clickable"
        @click="openWorld(w)"
      >
        <IconFolder class="world-icon" />
        <div class="c-info">
          <div class="c-name">{{ worldLabel(w) }}</div>
          <div class="c-meta">
            <span>{{ gameModeOptions.find((o) => o.value === w.gameType)?.label ?? "—" }}</span>
            <span>{{ difficultyOptions.find((o) => o.value === w.difficulty)?.label ?? "—" }}</span>
            <span v-if="w.playerName">
              {{ t("nbt.playerLabel") }} {{ w.playerName }}
              <template v-if="(w.playerCount ?? 0) > 1">+{{ (w.playerCount ?? 1) - 1 }}</template>
            </span>
            <span>{{ fmtSize(w.size) }}</span>
            <span v-if="w.modified">{{ fmtDate(w.modified) }}</span>
          </div>
        </div>
      </div>
    </div>

    <!-- 编辑器 -->
    <div v-else class="editor glass">
      <div class="editor-head qx-card-open">
        <button class="mini-btn" @click="back">← {{ t("nbt.backToList") }}</button>
        <span class="world-title">{{ worldLabel(current) }}</span>
        <span v-if="manual" class="tag manual">{{ t("nbt.manualTag") }}</span>
        <span v-if="running" class="tag warn">{{ t("nbt.runningWarn") }}</span>
        <span v-else-if="!editable" class="tag warn">{{ t("nbt.readonlyTag") }}</span>
        <div class="spacer" />
        <button class="mini-btn" @click="helpShow = true">
          <IconInfo /> {{ t("nbt.helpTitle") }}
        </button>
        <button class="mini-btn" @click="backupShow = true">
          <IconRestore /> {{ t("nbt.backupTitle") }}
        </button>
        <!-- 表单 / 树形 模式切换 -->
        <div v-if="tab === 'world'" ref="modeSegRef" v-segmented="{ label: t('common.editorMode') }" class="seg qx-seg qx-seg--sm">
          <div class="qx-seg__indicator" :style="modeSegStyle"></div>
          <button :class="{ active: mode === 'form' }" @click="setMode('form')">
            {{ t("nbt.formMode") }}
          </button>
          <button :class="{ active: mode === 'tree' }" @click="setMode('tree')">
            {{ t("nbt.treeMode") }}
          </button>
        </div>
      </div>

      <div v-if="readonlyReason && !running" class="readonly-bar">{{ readonlyReason }}</div>

      <!-- 三大分区 -->
      <div ref="tabSegRef" v-segmented="{ label: t('nbt.nbtTitle'), panel: 'nbt-tab-panel' }" class="seg qx-seg">
        <div class="qx-seg__indicator" :style="tabSegStyle"></div>
        <button :class="{ active: tab === 'world' }" @click="setTab('world')">
          {{ t("nbt.worldProps") }}
        </button>
        <button :class="{ active: tab === 'chunk' }" @click="setTab('chunk')">
          {{ t("nbt.chunks") }}
        </button>
        <button :class="{ active: tab === 'player' }" @click="setTab('player')">
          <IconUser /> {{ t("nbt.players") }}
        </button>
        <button :class="{ active: tab === 'item' }" @click="setTab('item')">
          <IconBox /> {{ t("nbt.inventory") }}
        </button>
      </div>

      <div id="nbt-tab-panel" class="nbt-tab-panel" role="tabpanel">
      <!-- 世界属性：表单模式 -->
      <div v-if="tab === 'world' && mode === 'form' && form" class="form-body">
        <div class="field">
          <label for="nbt-levelName">{{ t("nbt.levelName") }}</label>
          <NInput v-model:value="form.levelName" :input-props="{ id: 'nbt-levelName', spellcheck: false }" />
        </div>
        <div class="field-row">
          <div class="field">
            <label>{{ t("nbt.gameMode") }}</label>
            <NSelect v-model:value="form.gameType" :options="gameModeOptions" size="small" />
          </div>
          <div class="field">
            <label>{{ t("nbt.difficulty") }}</label>
            <NSelect v-model:value="form.difficulty" :options="difficultyOptions" size="small" />
          </div>
        </div>
        <div class="field">
          <label for="nbt-seed">{{ t("nbt.seed") }}</label>
          <NInput v-model:value="form.seed" :input-props="{ id: 'nbt-seed', spellcheck: false, inputmode: 'numeric' }" />
          <span class="hint">{{ t("nbt.seedHint") }}</span>
        </div>
        <div class="field">
          <label for="nbt-time">{{ t("nbt.time") }}</label>
          <NInput v-model:value="form.time" :input-props="{ id: 'nbt-time', spellcheck: false, inputmode: 'numeric' }" />
        </div>
        <div class="field">
          <label for="nbt-day-time">{{ t("nbt.dayTime") }}</label>
          <div class="inline">
            <NInput v-model:value="form.dayTime" class="num" :input-props="{ id: 'nbt-day-time', spellcheck: false, inputmode: 'numeric' }" />
            <button class="mini-btn" @click="setTime('sunrise')">{{ t("nbt.timeSunrise") }}</button>
            <button class="mini-btn" @click="setTime('noon')">{{ t("nbt.timeNoon") }}</button>
            <button class="mini-btn" @click="setTime('sunset')">{{ t("nbt.timeSunset") }}</button>
            <button class="mini-btn" @click="setTime('midnight')">{{ t("nbt.timeMidnight") }}</button>
          </div>
        </div>
        <div class="field">
          <label>{{ t("nbt.weather") }}</label>
          <div class="inline">
            <button class="mini-btn" :class="{ active: !form.raining && !form.thundering }" @click="setWeather('clear')">{{ t("nbt.weatherClear") }}</button>
            <button class="mini-btn" :class="{ active: form.raining && !form.thundering }" @click="setWeather('rain')">{{ t("nbt.weatherRain") }}</button>
            <button class="mini-btn" :class="{ active: !!form.thundering }" @click="setWeather('thunder')">{{ t("nbt.weatherThunder") }}</button>
          </div>
        </div>
        <div class="field">
          <label>{{ t("nbt.spawn") }}</label>
          <div class="inline">
            <label for="nbt-spawn-x">X</label>
            <NInputNumber v-model:value="form.spawnX" class="num" :show-button="false" :input-props="{ id: 'nbt-spawn-x' }" />
            <label for="nbt-spawn-y">Y</label>
            <NInputNumber v-model:value="form.spawnY" class="num" :show-button="false" :input-props="{ id: 'nbt-spawn-y' }" />
            <label for="nbt-spawn-z">Z</label>
            <NInputNumber v-model:value="form.spawnZ" class="num" :show-button="false" :input-props="{ id: 'nbt-spawn-z' }" />
          </div>
        </div>
        <div class="field-row">
          <div class="field">
            <label>{{ t("nbt.allowCheats") }}</label>
            <NSelect v-model:value="form.allowCommands" :options="yesNoOptions" size="small" />
          </div>
          <div class="field">
            <label>{{ t("nbt.lockDifficulty") }}</label>
            <div class="inline">
              <NSwitch :value="!!form.difficultyLocked" @update:value="(v: boolean) => (form!.difficultyLocked = v ? 1 : 0)" />
              <span class="hint">{{ t("nbt.lockHint") }}</span>
            </div>
          </div>
        </div>
        <div class="field">
          <label>{{ t("nbt.hardcore") }}</label>
          <div class="inline">
            <NSwitch
              :value="!!form.hardcore"
              @update:value="(v: boolean) => (form!.hardcore = v ? 1 : 0)"
            />
            <span class="hint">{{ t("nbt.hardcoreHint") }}</span>
          </div>
        </div>

        <template v-if="form.gameRules">
          <h3 class="sec-title">{{ t("nbt.gameRules") }}</h3>
          <div class="rules">
            <div v-for="k in BOOL_RULES" :key="k" class="rule-row">
              <span class="rule-label" :title="k">{{ ruleLabel(k) }}</span>
              <NSwitch
                size="small"
                :value="form.gameRules?.[k] === 'true'"
                @update:value="(v: boolean) => setRule(k, v ? 'true' : 'false')"
              />
            </div>
          </div>
          <div class="field-row">
            <div v-for="k in NUMERIC_RULES" :key="k" class="field">
              <label :title="k">{{ ruleLabel(k) }}</label>
              <input
                class="qx-input"
                type="number"
                :value="form.gameRules?.[k] ?? ''"
                @input="(e: Event) => setRule(k, (e.target as HTMLInputElement).value)"
              />
            </div>
          </div>
          <NCollapse v-if="otherRules.length" class="rules-all" display-directive="show">
            <NCollapseItem name="all-rules" :title="`${t('nbt.allRules')} (${otherRules.length})`">
              <div class="rules">
                <div v-for="k in otherRules" :key="k" class="rule-row">
                  <span class="rule-label" :title="k">{{ ruleLabel(k) }}</span>
                  <NSwitch
                    v-if="isBoolRule(form.gameRules?.[k])"
                    size="small"
                    :value="form.gameRules?.[k] === 'true'"
                    @update:value="(v: boolean) => setRule(k, v ? 'true' : 'false')"
                  />
                  <NInput
                    v-else
                    class="rule-input"
                    size="small"
                    :value="form.gameRules?.[k]"
                    :input-props="{ 'aria-label': ruleLabel(k) }"
                    @update:value="(value) => setRule(k, value)"
                  />
                </div>
              </div>
            </NCollapseItem>
          </NCollapse>
        </template>

        <template v-if="form.border">
          <h3 class="sec-title">{{ t("nbt.worldBorder") }}</h3>
          <div class="field-row">
            <div class="field">
              <label for="nbt-border-x">{{ t("nbt.center") }} X</label>
              <NInputNumber v-model:value="form.border.centerX" :step="0.5" :input-props="{ id: 'nbt-border-x' }" />
            </div>
            <div class="field">
              <label for="nbt-border-z">{{ t("nbt.center") }} Z</label>
              <NInputNumber v-model:value="form.border.centerZ" :step="0.5" :input-props="{ id: 'nbt-border-z' }" />
            </div>
            <div class="field">
              <label for="nbt-border-size">{{ t("nbt.size") }}</label>
              <NInputNumber v-model:value="form.border.size" :step="0.5" :input-props="{ id: 'nbt-border-size' }" />
            </div>
          </div>
        </template>

        <div class="actions">
          <NButton
            type="primary"
            :loading="saving"
            :disabled="running || !editable"
            @click="save"
          >
            <IconSave />&nbsp;{{ t("nbt.save") }}
          </NButton>
          <button class="mini-btn" @click="resetForm">{{ t("nbt.reset") }}</button>
          <span v-if="changes.length" class="dirty">{{ changes.length }} {{ t("nbt.pendingCount") }}</span>
        </div>
        <div v-if="lastBackup" class="backup-tip">{{ t("nbt.backupMade") }}：{{ lastBackup }}</div>
      </div>

      <!-- 世界属性：树形模式 -->
      <NbtTreeEditor
        v-else-if="tab === 'world' && mode === 'tree'"
        :instance-id="instanceId"
        :world="current.dir"
        :running="running"
        :readonly="!editable"
        :highlight="changes.map((c) => c.key)"
        @backed-up="onBackedUp"
      />

      <!-- 区块（.mca） -->
      <ChunkEditor
        v-else-if="tab === 'chunk'"
        :instance-id="instanceId"
        :world="current.dir"
        :running="running"
        :readonly="!editable"
        @backed-up="onBackedUp"
      />

      <!-- 玩家数据 -->
      <div v-else-if="tab === 'player'" class="tab-body">
        <div v-if="!players.length" class="center">{{ t("nbt.noPlayers") }}</div>
        <template v-else>
          <div class="player-list">
            <button
              v-for="p in players"
              :key="p.uuid"
              class="mini-btn player-btn"
              :class="{ active: activePlayer?.uuid === p.uuid }"
              @click="activePlayer = p"
            >
              <IconUser /> {{ p.name }}
            </button>
          </div>
          <PlayerEditor
            v-if="activePlayer"
            :instance-id="instanceId"
            :world="current.dir"
            :player="activePlayer"
            :running="running"
            :readonly="!editable"
            @backed-up="onBackedUp"
          />
          <div v-else class="center">{{ t("nbt.selectPlayer") }}</div>
        </template>
      </div>

      <!-- 背包物品 -->
      <div v-else-if="tab === 'item'" class="tab-body">
        <div v-if="!players.length" class="center">{{ t("nbt.noPlayers") }}</div>
        <template v-else>
          <div class="player-list">
            <button
              v-for="p in players"
              :key="p.uuid"
              class="mini-btn player-btn"
              :class="{ active: activePlayer?.uuid === p.uuid }"
              @click="activePlayer = p"
            >
              <IconUser /> {{ p.name }}
            </button>
          </div>
          <ItemEditor
            v-if="activePlayer"
            :instance-id="instanceId"
            :world="current.dir"
            :uuid="activePlayer.uuid"
            :running="running"
            :readonly="!editable"
            @backed-up="onBackedUp"
          />
          <div v-else class="center">{{ t("nbt.selectPlayer") }}</div>
        </template>
      </div>
      </div>
    </div>

    <!-- 操作说明：工具栏按钮打开，首次切到树形模式自动弹一次 -->
    <NModal
      v-model:show="helpShow"
      preset="card"
      :title="t('nbt.helpTitle')"
      style="max-width: 460px"
    >
      <p class="help-line">{{ t("nbt.helpLine") }}</p>
    </NModal>

    <BackupPanel
      v-if="current"
      v-model:show="backupShow"
      :instance-id="instanceId"
      :world="current.dir"
      @restored="onRestored"
    />
  </div>
</template>

<style scoped>
.nbt-view {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 4px;
}
.bar {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  padding: 12px 16px;
}
.bar-label {
  font-size: var(--fs-sm);
  color: var(--text-2);
}
.spacer {
  flex: 1;
}
.busy {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.busy-text {
  display: flex;
  gap: 8px;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.busy-detail {
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.world-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.world-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 14px;
  cursor: pointer;
}
.world-icon {
  font-size: 22px;
  color: var(--accent);
  flex-shrink: 0;
}
.c-info {
  flex: 1;
  min-width: 0;
}
.c-name {
  font-size: var(--fs-base);
  font-weight: 600;
}
.c-meta {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  margin-top: 3px;
}
.editor {
  padding: 18px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.editor-head {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
}
.world-title {
  font-size: var(--fs-md);
  font-weight: 700;
}
.tag {
  font-size: var(--fs-2xs);
  padding: 2px 8px;
  border-radius: var(--r-pill);
  border: 1px solid var(--border);
  color: var(--text-3);
}
.tag.warn {
  color: var(--warn);
  border-color: #e0a00055;
}
.tag.manual {
  color: var(--accent);
  border-color: var(--accent-04);
}
.readonly-bar {
  padding: 8px 12px;
  border-radius: var(--r-sm);
  background: var(--w-04);
  color: var(--text-2);
  font-size: var(--fs-xs);
}
/* 分段控件外观走全局 .qx-seg（主标签）/ .qx-seg--sm（表单-树形切换） */
.nbt-tab-panel { display: flex; flex-direction: column; min-width: 0; }
.form-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.field-row {
  display: flex;
  gap: 14px;
  flex-wrap: wrap;
}
.field-row .field {
  flex: 1;
  min-width: 150px;
}
.field label {
  font-size: var(--fs-xs);
  color: var(--text-2);
  font-weight: 600;
}
.num {
  width: 110px;
}
.inline {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.hint {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.sec-title {
  font-size: var(--fs-sm);
  font-weight: 700;
  color: var(--text-2);
  margin-top: 4px;
}
.rules {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 6px 16px;
}
.rule-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 3px 0;
}
.rule-label {
  font-size: var(--fs-xs);
  color: var(--text-2);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.rule-input {
  width: 120px;
}
.rules-all {
  margin-top: 2px;
}
.rules-all .rules {
  margin-top: 6px;
}
.actions {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 6px;
}
.dirty {
  font-size: var(--fs-xs);
  color: var(--accent);
}
.backup-tip {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.tab-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.player-list {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.player-btn {
  font-size: var(--fs-xs);
}
.center {
  padding: 40px;
  text-align: center;
  color: var(--text-3);
}
.help-line {
  margin: 0;
  font-size: var(--fs-sm);
  line-height: 1.6;
  color: var(--text-1);
}
</style>
