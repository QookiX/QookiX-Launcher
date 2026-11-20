<script setup lang="ts">
import { computed, markRaw, nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import { copyText as doCopy } from "../utils/clipboard";
import { NButton, NInput, NModal, useMessage } from "naive-ui";
import type { FileWorkspaceOperations } from "./fileWorkspace";
import { fmtDate, fmtSize } from "../utils/format";
import type { ContextMenuItem, FsEntry } from "../types";
import CodeEditor from "./CodeEditor.vue";
import ContextMenu from "./ContextMenu.vue";
import {
  IconChevronLeft,
  IconChevronRight,
  IconClose,
  IconCopy,
  IconCornerUpLeft,
  IconEdit,
  IconExternal,
  IconFile,
  IconFolder,
  IconPlus,
  IconRefresh,
  IconSave,
  IconSearch,
  IconTrash,
  IconType,
} from "./icons";

const props = defineProps<{
  operations: FileWorkspaceOperations;
  locale: "fileManager" | "serverFileManager";
  rootLabel: string;
}>();
const message = useMessage();
const { t } = useI18n();
const label = (key: string, params: Record<string, unknown> = {}) => t(`${props.locale}.${key}`, params);
let closed = false;
let listRequest = 0;

/** 内置编辑器支持打开的最大文件体积，与后端 MAX_EDIT_BYTES 保持一致 */
const MAX_EDIT = 4 * 1024 * 1024;

const TEXT_EXT = new Set([
  "txt", "json", "json5", "properties", "cfg", "conf", "config", "toml", "yaml", "yml",
  "ini", "xml", "lang", "mcmeta", "snbt", "js", "mjs", "cjs", "ts", "lua", "py",
  "md", "log", "csv", "html", "css", "sh", "bat", "cmd", "gitignore",
]);
const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico"]);
const ARCHIVE_EXT = new Set(["jar", "zip", "mrpack", "litematic", "nbt"]);

function isEditable(e: FsEntry): boolean {
  if (e.is_dir || e.size > MAX_EDIT) return false;
  return TEXT_EXT.has(e.ext) || (e.ext === "" && e.size <= 256 * 1024);
}

function badge(e: FsEntry): string {
  if (e.is_dir) return "DIR";
  return e.ext ? e.ext.slice(0, 4).toUpperCase() : "···";
}

function badgeKind(e: FsEntry): string {
  if (e.is_dir) return "dir";
  if (IMAGE_EXT.has(e.ext)) return "img";
  if (ARCHIVE_EXT.has(e.ext)) return "arc";
  if (TEXT_EXT.has(e.ext)) return "code";
  if (!e.ext) return "code";
  return "bin";
}

// ---------------------------------------------------------------- 目录浏览

const cwd = ref("");
const entries = ref<FsEntry[]>([]);
const loading = ref(false);
const filter = ref("");
const backStack = ref<string[]>([]);
const fwdStack = ref<string[]>([]);

const crumbs = computed(() => {
  if (!cwd.value) return [];
  const parts = cwd.value.split("/");
  return parts.map((p, i) => ({ name: p, rel: parts.slice(0, i + 1).join("/") }));
});

const shown = computed(() => {
  const q = filter.value.trim().toLowerCase();
  if (!q) return entries.value;
  return entries.value.filter((e) => e.name.toLowerCase().includes(q));
});

async function reload() {
  const path = cwd.value;
  const request = ++listRequest;
  loading.value = true;
  try {
    const r = await props.operations.list(path);
    // 目录不存在（例如已被删除）时回退到实例根目录
    if (closed || request !== listRequest || r.rel !== cwd.value) return;
    entries.value = r.entries;
  } catch (e) {
    if (closed || request !== listRequest) return;
    entries.value = [];
    message.error(String(e));
  } finally {
    if (!closed && request === listRequest) loading.value = false;
  }
}

function navigate(rel: string) {
  if (rel === cwd.value) return;
  backStack.value.push(cwd.value);
  fwdStack.value = [];
  cwd.value = rel;
  filter.value = "";
  selected.value = null;
  reload();
}

function goBack() {
  const prev = backStack.value.pop();
  if (prev === undefined) return;
  fwdStack.value.push(cwd.value);
  cwd.value = prev;
  filter.value = "";
  reload();
}

function goForward() {
  const next = fwdStack.value.pop();
  if (next === undefined) return;
  backStack.value.push(cwd.value);
  cwd.value = next;
  filter.value = "";
  reload();
}

function goUp() {
  const i = cwd.value.lastIndexOf("/");
  navigate(i < 0 ? "" : cwd.value.slice(0, i));
}

async function reveal(rel: string) {
  try {
    await props.operations.reveal(rel);
  } catch (e) {
    message.error(String(e));
  }
}

// ---------------------------------------------------------------- 编辑器标签页

interface OpenTab {
  rel: string;
  name: string;
  content: string;
  original: string;
  size: number;
  modified: number;
}

const openTabs = ref<OpenTab[]>([]);
const activeRel = ref<string | null>(null);
const saving = ref(false);

const active = computed(() => openTabs.value.find((t) => t.rel === activeRel.value) ?? null);
const isDirty = (t: OpenTab) => t.content !== t.original;

/** 用 rel 定位写入，避免在模板里直接给 computed 的属性赋值 */
function onEdit(rel: string, value: string) {
  const t = openTabs.value.find((x) => x.rel === rel);
  if (t) t.content = value;
}

async function openEntry(e: FsEntry) {
  if (e.is_dir) {
    navigate(e.rel);
    return;
  }
  if (!isEditable(e)) {
    message.info(label("notEditable", { name: e.name }));
    reveal(e.rel);
    return;
  }
  if (openTabs.value.some((t) => t.rel === e.rel)) {
    activeRel.value = e.rel;
    return;
  }
  try {
    const r = await props.operations.read(e.rel);
    if (closed) return;
    if (openTabs.value.some((tab) => tab.rel === e.rel)) {
      activeRel.value = e.rel;
      return;
    }
    openTabs.value.push({
      rel: e.rel,
      name: e.name,
      content: r.content,
      original: r.content,
      size: r.size,
      modified: r.modified,
    });
    activeRel.value = e.rel;
  } catch (err) {
    message.error(String(err));
  }
}

async function saveTab(tab?: OpenTab | null) {
  const cur = tab ?? active.value;
  if (!cur || saving.value) return;
  const submitted = cur.content;
  saving.value = true;
  try {
    const r = await props.operations.write(cur.rel, submitted);
    if (closed) return;
    cur.original = submitted;
    cur.size = r.size;
    cur.modified = r.modified;
    message.success(label("saved", { name: cur.name }));
    await reload();
  } catch (e) {
    message.error(String(e));
  } finally {
    saving.value = false;
  }
}

function closeTab(tab: OpenTab) {
  const i = openTabs.value.findIndex((x) => x.rel === tab.rel);
  if (i < 0) return;
  openTabs.value.splice(i, 1);
  if (activeRel.value === tab.rel) {
    const next = openTabs.value[Math.min(i, openTabs.value.length - 1)];
    activeRel.value = next ? next.rel : null;
  }
}

function requestClose(tab: OpenTab) {
  if (!isDirty(tab)) {
    closeTab(tab);
    return;
  }
  confirmState.value = {
    title: label("unsavedChanges"),
    content: label("unsavedCloseConfirm", { name: tab.name }),
    positiveText: label("discardChanges"),
    onOk: () => closeTab(tab),
  };
}

// ---------------------------------------------------------------- 新建 / 重命名 / 删除

const creating = ref<{ isDir: boolean; name: string } | null>(null);
const createInput = ref<HTMLInputElement | null>(null);
const renaming = ref<{ rel: string; name: string } | null>(null);
const renameInput = ref<HTMLInputElement | null>(null);

// 位于 v-for 内部，必须用函数 ref，否则 Vue 会把它收集成数组
function setRenameInput(el: unknown) {
  renameInput.value = (el as HTMLInputElement | null) ?? null;
}

function startCreate(isDir: boolean) {
  renaming.value = null;
  creating.value = { isDir, name: "" };
  nextTick(() => createInput.value?.focus());
}

async function commitCreate() {
  const c = creating.value;
  const structure = props.operations.structure;
  if (!c || !structure) return;
  const name = c.name.trim();
  if (!name) {
    creating.value = null;
    return;
  }
  const rel = cwd.value ? `${cwd.value}/${name}` : name;
  try {
    await structure.create(rel, c.isDir);
    creating.value = null;
    await reload();
    if (!c.isDir) {
      const e = entries.value.find((x) => x.rel === rel);
      if (e) await openEntry(e);
    }
  } catch (e) {
    message.error(String(e));
  }
}

function startRename(e: FsEntry) {
  creating.value = null;
  renaming.value = { rel: e.rel, name: e.name };
  nextTick(() => renameInput.value?.select());
}

async function commitRename() {
  const r = renaming.value;
  const structure = props.operations.structure;
  if (!r || !structure) return;
  const name = r.name.trim();
  if (!name) {
    renaming.value = null;
    return;
  }
  try {
    const res = await structure.rename(r.rel, name);
    renaming.value = null;
    const prefix = r.rel + "/";
    for (const t of openTabs.value) {
      if (t.rel === r.rel) {
        t.rel = res.rel;
        t.name = res.name;
      } else if (t.rel.startsWith(prefix)) {
        t.rel = res.rel + t.rel.slice(r.rel.length);
      }
    }
    if (activeRel.value === r.rel) activeRel.value = res.rel;
    else if (activeRel.value?.startsWith(prefix)) activeRel.value = res.rel + activeRel.value.slice(r.rel.length);
    await reload();
    message.success(label("renamed"));
  } catch (e) {
    message.error(String(e));
  }
}

function askDelete(e: FsEntry) {
  confirmState.value = {
    title: e.is_dir ? label("deleteFolder") : label("deleteFile"),
    content: e.is_dir
      ? label("confirmDeleteFolder", { name: e.name })
      : label("confirmDeleteFile", { name: e.name }),
    positiveText: label("delete"),
    onOk: () => doDelete(e),
  };
}

async function doDelete(e: FsEntry) {
  const structure = props.operations.structure;
  if (!structure) return;
  try {
    await structure.remove(e.rel);
    const prefix = e.rel + "/";
    openTabs.value = openTabs.value.filter(
      (t) => t.rel !== e.rel && !t.rel.startsWith(prefix)
    );
    if (!openTabs.value.some((t) => t.rel === activeRel.value)) {
      const last = openTabs.value[openTabs.value.length - 1];
      activeRel.value = last ? last.rel : null;
    }
    message.success(label("deleted", { name: e.name }));
    await reload();
  } catch (err) {
    message.error(String(err));
  }
}

// ---------------------------------------------------------------- 确认弹窗

const confirmState = ref<{
  title: string;
  content: string;
  positiveText: string;
  onOk: () => void | Promise<void>;
} | null>(null);
const confirmLoading = ref(false);

async function handleConfirm() {
  if (!confirmState.value) return;
  confirmLoading.value = true;
  try {
    await confirmState.value.onOk();
    confirmState.value = null;
  } finally {
    confirmLoading.value = false;
  }
}

// ---------------------------------------------------------------- 右键菜单

const menu = ref<{ show: boolean; x: number; y: number; items: ContextMenuItem[] }>({
  show: false,
  x: 0,
  y: 0,
  items: [],
});

function openMenu(x: number, y: number, items: ContextMenuItem[]) {
  menu.value = { show: true, x, y, items };
}
function closeMenu() {
  if (menu.value.show) menu.value = { ...menu.value, show: false };
}

/** 当前被选中（点击或右键）的条目，用于 F2 / Delete 快捷键 */
const selected = ref<string | null>(null);

async function copyText(text: string, tip: string) {
  const ok = await doCopy(text);
  if (ok) message.success(tip);
  else message.error(label("copyFailed"));
}

function entryMenu(e: FsEntry): ContextMenuItem[] {
  const items: ContextMenuItem[] = [];
  if (e.is_dir) {
    items.push({
      key: "open",
      label: label("open"),
      icon: markRaw(IconFolder),
      action: () => navigate(e.rel),
    });
  } else if (isEditable(e)) {
    items.push({
      key: "open",
      label: label("openInEditor"),
      icon: markRaw(IconEdit),
      action: () => openEntry(e),
    });
  }
  items.push(
    {
      key: "reveal",
      label: e.is_dir ? label("openLocation") : label("revealInFileManager"),
      icon: markRaw(IconExternal),
      action: () => reveal(e.rel),
    },
    { key: "s1", sep: true },
    {
      key: "copy-rel",
      label: label("copyRelPath"),
      icon: markRaw(IconCopy),
      action: () => copyText(e.rel, label("copiedPath")),
    },
    {
      key: "copy-name",
      label: label("copyFileName"),
      icon: markRaw(IconCopy),
      action: () => copyText(e.name, label("copiedFileName")),
    },
  );
  if (props.operations.structure) items.push(
    { key: "s2", sep: true },
    {
      key: "rename",
      label: label("rename"),
      icon: markRaw(IconType),
      shortcut: "F2",
      action: () => startRename(e),
    },
    {
      key: "delete",
      label: e.is_dir ? label("deleteFolder") : label("delete"),
      icon: markRaw(IconTrash),
      shortcut: "Delete",
      danger: true,
      action: () => askDelete(e),
    },
  );
  return items;
}

function onRowContext(ev: MouseEvent, e: FsEntry) {
  selected.value = e.rel;
  openMenu(ev.clientX, ev.clientY, entryMenu(e));
}

function onBlankContext(ev: MouseEvent) {
  selected.value = null;
  const items: ContextMenuItem[] = [];
  if (props.operations.structure) items.push(
    {
      key: "new-file",
      label: label("newFile"),
      icon: markRaw(IconFile),
      action: () => startCreate(false),
    },
    {
      key: "new-dir",
      label: label("newFolder"),
      icon: markRaw(IconFolder),
      action: () => startCreate(true),
    },
    { key: "s1", sep: true },
  );
  items.push(
    {
      key: "refresh",
      label: label("refresh"),
      icon: markRaw(IconRefresh),
      shortcut: "F5",
      action: () => reload(),
    },
    {
      key: "reveal",
      label: label("openInSystemFileManager"),
      icon: markRaw(IconExternal),
      action: () => reveal(cwd.value),
    },
  );
  openMenu(ev.clientX, ev.clientY, items);
}

function onTabContext(ev: MouseEvent, tab: OpenTab) {
  activeRel.value = tab.rel;
  const others = openTabs.value.filter((x) => x.rel !== tab.rel);
  openMenu(ev.clientX, ev.clientY, [
    {
      key: "save",
      label: label("save"),
      icon: markRaw(IconSave),
      shortcut: "Ctrl+S",
      disabled: !isDirty(tab),
      action: () => saveTab(tab),
    },
    {
      key: "copy-rel",
      label: label("copyRelPath"),
      icon: markRaw(IconCopy),
      action: () => copyText(tab.rel, label("copiedPath")),
    },
    { key: "s1", sep: true },
    {
      key: "close",
      label: label("close"),
      icon: markRaw(IconClose),
      action: () => requestClose(tab),
    },
    {
      key: "close-others",
      label: label("closeOthers"),
      disabled: !others.length,
      action: async () => {
        for (const o of others) {
          if (!isDirty(o)) closeTab(o);
        }
        const rest = openTabs.value.filter((x) => x.rel !== tab.rel);
        if (rest.length) {
          message.info(label("tabsUnsaved", { count: rest.length }));
        }
      },
    },
    {
      key: "close-all",
      label: label("closeAll"),
      disabled: !openTabs.value.length,
      action: () => {
        const dirty = openTabs.value.filter(isDirty).length;
        if (!dirty) {
          openTabs.value = [];
          activeRel.value = null;
          return;
        }
        confirmState.value = {
          title: label("unsavedChanges"),
          content: label("confirmCloseAll", { count: dirty }),
          positiveText: label("closeAllConfirm"),
          onOk: () => {
            openTabs.value = [];
            activeRel.value = null;
          },
        };
      },
    },
  ]);
}

// ---- 编辑器内右键 ----

const editorRef = ref<InstanceType<typeof CodeEditor> | null>(null);

function onEditorContext(p: { x: number; y: number }) {
  const tab = active.value;
  const hasSel = editorRef.value?.hasSelection() ?? false;
  const items: ContextMenuItem[] = [
    {
      key: "cut",
      label: label("cut"),
      shortcut: "Ctrl+X",
      disabled: !hasSel,
      action: () => editorRef.value?.cutSelection(),
    },
    {
      key: "copy",
      label: label("copy"),
      shortcut: "Ctrl+C",
      disabled: !hasSel,
      action: () => editorRef.value?.copySelection(),
    },
    {
      key: "paste",
      label: label("paste"),
      shortcut: "Ctrl+V",
      action: () => editorRef.value?.pasteClipboard(),
    },
    { key: "s1", sep: true },
    {
      key: "select-all",
      label: label("selectAll"),
      shortcut: "Ctrl+A",
      action: () => editorRef.value?.selectAll(),
    },
  ];
  if (tab) {
    items.push(
      { key: "s2", sep: true },
      {
        key: "save",
        label: label("save"),
        icon: markRaw(IconSave),
        shortcut: "Ctrl+S",
        disabled: !isDirty(tab),
        action: () => saveTab(tab),
      },
    );
  }
  openMenu(p.x, p.y, items);
}

function onGlobalKeydown(e: KeyboardEvent) {
  if (e.defaultPrevented) return;
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
    e.preventDefault();
    const t = active.value;
    if (t && isDirty(t)) saveTab(t);
    return;
  }

  const tag = (e.target as HTMLElement | null)?.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA") return;

  // 文件列表快捷键：F2 重命名、Delete 删除
  // F5 不拦截——留给页面刷新用；文件列表刷新用工具栏按钮或右键菜单
  if (props.operations.structure && selected.value) {
    const entry = entries.value.find((x) => x.rel === selected.value);
    if (entry && e.key === "F2") {
      e.preventDefault();
      startRename(entry);
    } else if (entry && e.key === "Delete") {
      e.preventDefault();
      askDelete(entry);
    }
  }
}

onMounted(() => {
  window.addEventListener("keydown", onGlobalKeydown);
  reload();
});

onBeforeUnmount(() => {
  closed = true;
  window.removeEventListener("keydown", onGlobalKeydown);
  closeMenu();
});
</script>

<template>
  <div class="qx-file-manager">
    <!-- 左侧：文件浏览 -->
    <aside class="fm-side glass">
      <div class="fm-path">
        <button class="nav" :disabled="!backStack.length" :title="label('back')" @click="goBack">
          <IconChevronLeft />
        </button>
        <button class="nav" :disabled="!fwdStack.length" :title="label('forward')" @click="goForward">
          <IconChevronRight />
        </button>
        <button class="nav" :disabled="!cwd" :title="label('up')" @click="goUp">
          <IconCornerUpLeft />
        </button>
        <div class="fm-crumbs">
          <button class="crumb root" @click="navigate('')">{{ rootLabel }}</button>
          <template v-for="c in crumbs" :key="c.rel">
            <span class="sep">/</span>
            <button class="crumb" :title="c.name" @click="navigate(c.rel)">{{ c.name }}</button>
          </template>
        </div>
        <button class="nav" :title="label('refresh')" @click="reload"><IconRefresh /></button>
      </div>

      <div class="fm-tools">
        <NInput v-model:value="filter" class="fm-search" size="small" :placeholder="label('filterPlaceholder')" :input-props="{ spellcheck: false }" clearable>
          <template #prefix><IconSearch /></template>
        </NInput>
        <button v-if="operations.structure" class="tool" :title="label('newFile')" @click="startCreate(false)">
          <IconFile /><IconPlus class="plus" />
        </button>
        <button v-if="operations.structure" class="tool" :title="label('newFolder')" @click="startCreate(true)">
          <IconFolder /><IconPlus class="plus" />
        </button>
        <button class="tool" :title="label('openInSystemFileManager')" @click="reveal(cwd)">
          <IconExternal />
        </button>
      </div>

      <div v-if="creating" class="fm-row creating">
        <span class="fm-badge" :class="creating.isDir ? 'dir' : 'code'">
          <IconFolder v-if="creating.isDir" /><IconFile v-else />
        </span>
        <input
          ref="createInput"
          v-model="creating.name"
          class="fm-input"
          :placeholder="creating.isDir ? label('folderNamePlaceholder') : label('fileNamePlaceholder')"
          spellcheck="false"
          @keydown.enter="commitCreate"
          @keydown.esc="creating = null"
        />
        <button class="ok" :title="label('ok')" @click="commitCreate">{{ label("ok") }}</button>
        <button class="cancel" :title="label('cancel')" @click="creating = null">{{ label("cancel") }}</button>
      </div>

      <div class="fm-list" @contextmenu.prevent="onBlankContext">
        <div v-if="loading && !entries.length" class="fm-empty">{{ label("loading") }}</div>
        <div v-else-if="!shown.length" class="fm-empty">
          {{ filter ? label("noMatch") : label("emptyFolder") }}
        </div>

        <template v-else>
          <div
            v-for="e in shown"
            :key="e.rel"
            class="fm-row"
            :class="{ active: activeRel === e.rel, selected: selected === e.rel }"
            @click="
              selected = e.rel;
              openEntry(e);
            "
            @contextmenu.prevent.stop="onRowContext($event, e)"
          >
            <span class="fm-badge" :class="badgeKind(e)">
              <IconFolder v-if="e.is_dir" />
              <template v-else>{{ badge(e) }}</template>
            </span>

            <div v-if="renaming && renaming.rel === e.rel" class="fm-rename">
              <input
                :ref="setRenameInput"
                v-model="renaming.name"
                class="fm-input"
                spellcheck="false"
                @click.stop
                @keydown.enter="commitRename"
                @keydown.esc="renaming = null"
              />
              <button class="ok" @click.stop="commitRename">{{ label("ok") }}</button>
              <button class="cancel" @click.stop="renaming = null">{{ label("cancel") }}</button>
            </div>

            <template v-else>
              <div class="fm-meta">
                <div class="fm-name" :title="e.name">{{ e.name }}</div>
                <div class="fm-sub">
                  {{ e.is_dir ? label("folder") : fmtSize(e.size)
                  }}<template v-if="e.modified"> · {{ fmtDate(e.modified) }}</template>
                </div>
              </div>
              <div class="fm-acts">
                <button
                  v-if="isEditable(e)"
                  class="act"
                  :title="label('openInEditor')"
                  @click.stop="openEntry(e)"
                >
                  <IconEdit />
                </button>
                <button v-if="operations.structure" class="act" :title="label('rename')" @click.stop="startRename(e)">
                  <IconType />
                </button>
                <button v-if="operations.structure" class="act danger" :title="label('delete')" @click.stop="askDelete(e)">
                  <IconTrash />
                </button>
              </div>
            </template>
          </div>
        </template>
      </div>
    </aside>

    <!-- 右侧：编辑器 -->
    <section class="fm-main glass">
      <div class="fm-tabs">
        <div
          v-for="tab in openTabs"
          :key="tab.rel"
          class="fm-tab"
          :class="{ active: tab.rel === activeRel }"
          :title="tab.rel"
          @click="activeRel = tab.rel"
          @contextmenu.prevent.stop="onTabContext($event, tab)"
        >
          <span class="dot" v-if="isDirty(tab)"></span>
          <span class="fm-tab-name">{{ tab.name }}</span>
          <button class="x" :title="label('close')" @click.stop="requestClose(tab)"><IconClose /></button>
        </div>
        <div class="fm-tabs-right">
          <button
            class="save-btn"
            :disabled="!active || saving || !isDirty(active)"
            @click="saveTab()"
          >
            <IconSave /> {{ saving ? label("saving") : label("save") }}
          </button>
        </div>
      </div>

      <div v-if="active" class="fm-editor">
        <CodeEditor
          ref="editorRef"
          :model-value="active.content"
          :filename="active.name"
          @update:model-value="onEdit(active.rel, $event)"
          @save="saveTab(active)"
          @contextmenu="onEditorContext"
        />
      </div>
      <div v-else class="fm-placeholder">
        <IconFile />
        <p>{{ label("placeholderTitle") }}</p>
        <span>{{ label("placeholderDesc") }}</span>
      </div>
    </section>

    <!-- 确认弹窗 -->
    <NModal
      :show="confirmState !== null"
      :title="confirmState?.title"
      preset="card"
      class="qx-confirm-modal"
      :mask-closable="!confirmLoading"
      :close-on-esc="!confirmLoading"
      :closable="!confirmLoading"
      @update:show="(show) => { if (!show) confirmState = null; }"
    >
      <div v-if="confirmState">
        <p>{{ confirmState.content }}</p>
        <div class="qx-confirm-actions">
          <NButton :disabled="confirmLoading" @click="confirmState = null">
            {{ label("cancel") }}
          </NButton>
          <NButton type="error" secondary :loading="confirmLoading" :disabled="confirmLoading" @click="handleConfirm">
            {{ confirmState.positiveText }}
          </NButton>
        </div>
      </div>
    </NModal>

    <!-- 右键菜单 -->
    <ContextMenu
      :show="menu.show"
      :x="menu.x"
      :y="menu.y"
      :items="menu.items"
      @close="closeMenu"
    />
  </div>
</template>

<style scoped>
.tool .plus {
  position: absolute;
  right: -3px;
  bottom: -2px;
  font-size: var(--fs-2xs);
  color: var(--accent);
}
.fm-row.creating {
  cursor: default;
}
.fm-row.creating:hover {
  background: transparent;
}
.fm-badge.img {
  color: #57c257;
  background: rgba(87, 194, 87, 0.14);
}
.fm-badge.arc {
  color: #f08a67;
  background: rgba(240, 101, 67, 0.14);
}
.act.danger:hover {
  color: var(--danger);
  border-color: var(--danger-50);
}

.fm-input {
  flex: 1;
  min-width: 0;
  border: 1px solid var(--accent-40);
  border-radius: var(--r-sm);
  background: var(--k-25);
  color: var(--text-1);
  font-size: var(--fs-xs);
  font-family: inherit;
  padding: 5px 8px;
  outline: none;
  user-select: text;
}
.fm-rename {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
}
.fm-rename .fm-input {
  flex: 1;
}
.ok,
.cancel {
  border: 1px solid var(--border);
  border-radius: var(--r-xs);
  background: transparent;
  color: var(--text-2);
  font-size: var(--fs-2xs);
  font-family: inherit;
  padding: 4px 8px;
  cursor: pointer;
  flex-shrink: 0;
}
.ok {
  color: var(--accent);
  border-color: var(--accent-35);
}
.ok:hover {
  background: var(--accent-12);
}
.cancel:hover {
  background: var(--w-07);
}

/* ---- 编辑器 ---- */

/* ---- 确认弹窗 ---- */
</style>
