<script setup lang="ts">
/**
 * 拾木空间投影库（独立页面，不再是模态框）。
 *
 * - 帖子列表 / 详情都是**匿名接口**，未登录也能浏览（带封面图）；
 * - 下载附件（含付费下单）需要登录，未登录时点下载会拉起登录弹窗；
 * - 下载完成后跳到投影工坊并用 `?open=<本地路径>` 直接打开预览。
 *
 * 板块（分类）：发帖人把投影帖发在哪个板块并不固定（投影 / 红石 / 生电 / 建筑
 * 都可能是投影作品），而站点的 `categoryId` 只能传单个值，所以默认用
 * 「投影相关」= 这几个板块一起查（后端分板块取一页再合并），
 * 也可以点具体板块只看一个。
 */
import { computed, onMounted, onUnmounted, reactive, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { NButton, NInput, NSelect, useMessage } from "naive-ui";
import DOMPurify from "dompurify";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "vue-i18n";
import SimplePagination from "../components/SimplePagination.vue";
import ShimuLoginDialog from "../components/ShimuLoginDialog.vue";
import { api } from "../api";
import { shimuAbsoluteUrl, shimuErrorMessage, useShimuStore } from "../stores/shimu";
import type { ShimuCategory, ShimuPost, ShimuPostQuery } from "../types";

const { t } = useI18n();
const message = useMessage();
const router = useRouter();
const store = useShimuStore();

const PAGE_SIZE = 12;

const loginRef = ref<InstanceType<typeof ShimuLoginDialog> | null>(null);

const keyword = ref("");
const sort = ref<"new" | "hot" | "featured">("hot");
const page = ref(1);
const rows = ref<ShimuPost[]>([]);
const total = ref(0);
const loading = ref(false);

/**
 * 板块筛选。`related` = 投影相关（多个板块合并），`all` = 不限板块，
 * 其余为分类 ID 字符串。
 */
const board = ref<string>("related");
const categories = ref<ShimuCategory[]>([]);
/** 「投影相关」包含的板块名（站点分类名，匹配时忽略空格） */
const RELATED_BOARD_NAMES = ["投影", "红石", "生电", "建筑"];
/** 分类接口拿不到时的兜底 ID（拾木空间固定值） */
const RELATED_BOARD_IDS = [1, 3, 4, 5];

const relatedIds = computed(() => {
  const ids = categories.value
    .filter((c) => RELATED_BOARD_NAMES.some((n) => (c.name ?? "").replace(/\s+/g, "").includes(n)))
    .map((c) => c.categoryId);
  return ids.length ? ids : RELATED_BOARD_IDS;
});

const boardChips = computed(() => {
  const chips = [
    { key: "related", label: t("shimu.boardRelated") },
    { key: "all", label: t("shimu.boardAll") },
  ];
  for (const c of categories.value) {
    chips.push({ key: String(c.categoryId), label: c.name });
  }
  return chips;
});

const detail = ref<ShimuPost | null>(null);
const detailLoading = ref(false);
const downloading = ref("");

/** 图片放大查看（图集与正文图片共用；多图时可左右切换） */
const viewer = ref<{ list: string[]; index: number } | null>(null);
const viewerSrc = computed(() => {
  const v = viewer.value;
  return v ? (v.list[v.index] ?? "") : "";
});

function openViewer(list: string[], index = 0) {
  const items = list.filter(Boolean);
  if (!items.length) return;
  viewer.value = { list: items, index: Math.min(Math.max(index, 0), items.length - 1) };
}
function closeViewer() {
  viewer.value = null;
}
function stepViewer(delta: number) {
  const v = viewer.value;
  if (!v || v.list.length < 2) return;
  v.index = (v.index + delta + v.list.length) % v.list.length;
}

function onViewerKey(e: KeyboardEvent) {
  if (!viewer.value) return;
  if (e.key === "Escape") closeViewer();
  else if (e.key === "ArrowLeft") stepViewer(-1);
  else if (e.key === "ArrowRight") stepViewer(1);
}

/** 加载失败的图片地址：站点图挂了就退回占位块，别显示碎图 */
const brokenImgs = reactive(new Set<string>());
function markBroken(url: string) {
  if (url) brokenImgs.add(url);
}

interface Attachment {
  url: string;
  name: string;
  fileId?: number;
  /** 字节数（站点附件卡的 data-size） */
  size?: number;
  kind: AttachKind;
}
const attachments = ref<Attachment[]>([]);

type AttachKind = "schematic" | "image" | "archive" | "file";

/** 投影类附件：可「下载并预览」 */
const SCHEMATIC_EXT = /\.(litematic|schem|schematic|mcstructure|nbt)$/i;
const IMAGE_EXT = /\.(png|jpe?g|webp|gif|bmp)$/i;
const ARCHIVE_EXT = /\.(zip|rar|7z|tar|gz|mrpack)$/i;
/** URL 末尾是个文件名（判定「像附件」用） */
const FILE_EXT_IN_PATH =
  /[^/]+\.(?:litematic|schem|schematic|mcstructure|nbt|zip|rar|7z|tar|gz|png|jpe?g|webp|gif|bmp|pdf|mp4)$/i;
/** 正文里常见写法：「名字.ext + 大小 + 类型」，从文本里抠出文件名 */
const NAME_IN_TEXT =
  /[^\s/\\]+\.(?:litematic|schem|schematic|mcstructure|nbt|zip|rar|7z|tar|gz|png|jpe?g|webp|gif|bmp|pdf|mp4)/i;

const sortOptions = computed(() => [
  { label: t("shimu.sortNew"), value: "new" },
  { label: t("shimu.sortHot"), value: "hot" },
  { label: t("shimu.sortFeatured"), value: "featured" },
]);

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)));

/**
 * 站点的「评论后查看」隐藏块标记（裸文本，形如 `[hidecontent type="reply" …]`）。
 * 作者确认客户端可以直接去掉。
 */
const HIDECONTENT_MARKER = /\[\/?hidecontent[^\]]*\]/gi;

/**
 * 去掉「评论后查看」的隐藏标记。
 *
 * 站点结构（抓自真实帖子）：
 *   `<div class="tinymce-hide"><p class="hide-before">[hidecontent …][/hidecontent]</p></div>`
 * 未评论时服务端**不会**下发被隐藏的内容，只留这串裸标记 —— 直接渲染就是截图里那种
 * 难看的方括号。这里删掉标记元素；万一块内确实带着正文（有权限时服务端会连同标记
 * 一起返回），则解包外层容器把内容保留下来，别连内容一起删。
 */
function stripHiddenMarkers(root: HTMLElement) {
  root.querySelectorAll(".hide-before").forEach((m) => m.remove());
  root.querySelectorAll<HTMLElement>(".tinymce-hide").forEach((box) => {
    if (!box.childNodes.length) return box.remove();
    box.replaceWith(...Array.from(box.childNodes));
  });
}

/**
 * 正文是富文本 HTML（编辑器上传的图片往往是 `/profile/xxx.png` 这种相对路径）。
 * 直接 v-html 会按启动器自身的 origin（tauri://localhost）解析 → 全部破图，
 * 所以先用 DOMParser 把相对 src/href 补成站点绝对地址，再交给 DOMPurify 净化。
 * （DOMParser 只解析不渲染，不会加载图片、也不会执行脚本。）
 */
function absolutizeRichHtml(html: string, host?: string): string {
  if (!html) return "";
  try {
    const doc = new DOMParser().parseFromString(`<div id="r">${html}</div>`, "text/html");
    const root = doc.getElementById("r");
    if (!root) return html;
    stripHiddenMarkers(root);
    root.querySelectorAll("[src], [href], [data-src]").forEach((el) => {
      for (const attr of ["src", "data-src", "href"]) {
        const v = el.getAttribute(attr);
        if (!v) continue;
        // 保留绝对地址 / data: / blob: / 锚点 / mailto 等
        if (/^([a-z][a-z0-9+.-]*:|\/\/|#)/i.test(v)) continue;
        el.setAttribute(attr, shimuAbsoluteUrl(v, host));
      }
      if (el.tagName === "IMG") {
        el.setAttribute("loading", "lazy");
      }
    });
    // 兜底：作者手写/粘贴在段落里的裸标记也一并清掉（DOM 路径覆盖不到的情况）
    return root.innerHTML.replace(HIDECONTENT_MARKER, "");
  } catch {
    return html;
  }
}

const bodyHtml = computed(() =>
  DOMPurify.sanitize(absolutizeRichHtml(detail.value?.content ?? "", store.config?.host))
);
const paidContent = computed(() => detail.value?.isPaid === "1" && detail.value?.payTarget === "0");
// 付费内容帖未购买时服务端直接返回空正文（且 purchased 字段在匿名访问下也会是 true），
// 所以以「正文为空」作为锁定信号更可靠。
const contentLocked = computed(() => paidContent.value && !(detail.value?.content ?? "").trim());

/** 标签：接口的 tagNames 常为 null，退一步显示分类名 */
function tagsOf(p?: ShimuPost | null): string {
  if (!p) return "";
  return p.tagNames?.trim() || p.categoryName?.trim() || "";
}

const accountMeta = computed(() => {
  const u = store.user;
  if (!u) return "";
  const parts: string[] = [];
  if (typeof u.score === "number") parts.push(t("shimu.score", { n: u.score }));
  if (typeof u.balance === "number") parts.push(t("shimu.balance", { n: u.balance.toFixed(2) }));
  const member = store.memberLabel;
  if (member) parts.push(member);
  return parts.join(" · ");
});

/** 封面：优先 `cover`，否则取 `images` 的第一张 */
function coverOf(p: ShimuPost): string {
  const raw = p.cover?.trim() || p.images?.split(",")[0]?.trim() || "";
  return raw ? shimuAbsoluteUrl(raw, store.config?.host) : "";
}

/** 详情页的图集（images 逗号分隔） */
const gallery = computed(() => {
  const raw = detail.value?.images ?? "";
  const host = store.config?.host;
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean)
    .map((p) => shimuAbsoluteUrl(p, host))
    .filter((u) => !brokenImgs.has(u))
    .slice(0, 8);
});

onMounted(() => {
  void store.init();
  void loadCategories();
  void search();
  window.addEventListener("keydown", onViewerKey);
});

onUnmounted(() => {
  window.removeEventListener("keydown", onViewerKey);
});

// 打开图片时锁定页面滚动（遮罩内部也不需要滚动）
watch(viewer, (v) => {
  const el = document.getElementById("app-content");
  if (el) el.style.overflow = v ? "hidden" : "";
});

// 登录 / 退出后刷新：purchased、liked 与正文解锁状态都会变
watch(
  () => store.loggedIn,
  () => {
    if (detail.value) void openPost(detail.value);
    else void search();
  }
);

watch(page, () => void search());

// 排序改成立刻生效（不用再点「刷新」）
watch(sort, () => requery());

function handleError(e: unknown) {
  message.error(shimuErrorMessage(e));
  if (String(e).includes("__shimu_expired__")) void store.refreshStatus();
}

async function search() {
  loading.value = true;
  try {
    const query: ShimuPostQuery = {
      keyword: keyword.value.trim() || undefined,
      sort: sort.value,
      pageNum: page.value,
      pageSize: PAGE_SIZE,
    };
    if (board.value === "related") query.categoryIds = relatedIds.value;
    else if (board.value !== "all") query.categoryId = Number(board.value);
    const res = await api.shimuListPosts(query);
    rows.value = res.rows ?? [];
    total.value = res.total ?? 0;
  } catch (e) {
    rows.value = [];
    total.value = 0;
    handleError(e);
  } finally {
    loading.value = false;
  }
}

/** 拉取站点板块列表（失败时降级为「投影相关」兜底 ID + 全部） */
async function loadCategories() {
  try {
    const list = await api.shimuCategories();
    categories.value = Array.isArray(list) ? list.filter((c) => c?.categoryId > 0) : [];
  } catch {
    categories.value = [];
  }
}

function submitSearch() {
  requery();
}

/** 条件变化后重新查询：先回到第一页（页码的 watch 会顺带触发搜索） */
function requery() {
  if (page.value !== 1) page.value = 1;
  else void search();
}

/** 切换板块：回到第一页重新查询 */
function selectBoard(key: string) {
  if (board.value === key) return;
  board.value = key;
  requery();
}

function openPost(post: ShimuPost) {
  detail.value = post;
  attachments.value = [];
  detailLoading.value = true;
  void (async () => {
    try {
      const raw = await api.shimuPostDetail(post.postId);
      const data = ((raw.data ?? raw) as ShimuPost) ?? post;
      detail.value = { ...post, ...data };
      attachments.value = extractAttachments(
        detail.value.content ?? "",
        store.config?.host ?? "https://www.mcimg.com"
      );
    } catch (e) {
      handleError(e);
    } finally {
      detailLoading.value = false;
    }
  })();
}

function closeDetail() {
  detail.value = null;
  attachments.value = [];
}

function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s).trim();
  } catch {
    return s.trim();
  }
}

function kindOf(url: string, name: string): AttachKind {
  const target = name || url.split(/[?#]/)[0];
  if (SCHEMATIC_EXT.test(target)) return "schematic";
  if (IMAGE_EXT.test(target)) return "image";
  if (ARCHIVE_EXT.test(target)) return "archive";
  return "file";
}

/**
 * 从正文富文本里抽出附件。
 *
 * 站点编辑器的附件是**卡片节点、没有 `<a>`**，地址放在 data-url：
 *   <div class="re-file" data-name="豪宅.litematic" data-size="4422"
 *        data-url="/profile/upload/2026/10/09/豪宅_20261009211016A011.litematic">
 *     <span class="re-file-name">豪宅.litematic</span> …
 *   </div>
 * 所以主路径读 `data-url`（+ data-name / data-size）；另外兼容手写 `a[href]` 的帖子。
 */
function extractAttachments(content: string, host: string): Attachment[] {
  if (!content) return [];
  const doc = new DOMParser().parseFromString(`<div id="r">${content}</div>`, "text/html");
  const root = doc.getElementById("r");
  if (!root) return [];
  const found = new Map<string, Attachment>();
  const push = (att: Attachment | null) => {
    if (att && !found.has(att.url)) found.set(att.url, att);
  };

  root.querySelectorAll<HTMLElement>("[data-url], [data-href], [data-download]").forEach((el) => {
    const raw =
      el.getAttribute("data-url") ?? el.getAttribute("data-href") ?? el.getAttribute("data-download") ?? "";
    if (!raw.trim()) return;
    push(attachmentFromUrl(raw, host, el.textContent ?? "", cardMeta(el)));
  });

  root.querySelectorAll("a[href]").forEach((a) => {
    push(attachmentFromUrl(a.getAttribute("href") ?? "", host, (a.textContent ?? "").replace(/\s+/g, " ")));
  });

  return [...found.values()];
}

/** 附件卡上的展示信息（名字 / 字节数） */
function cardMeta(el: HTMLElement): { name?: string; size?: number } {
  const sizeRaw = (el.getAttribute("data-size") ?? "").trim();
  const name =
    (el.getAttribute("data-name") ?? "").trim() ||
    el.querySelector(".re-file-name")?.textContent?.trim() ||
    "";
  return { name, size: /^\d+$/.test(sizeRaw) ? Number(sizeRaw) : undefined };
}

/** URL（+ 显示名 / 大小）→ 附件对象；不像附件时返回 null */
function attachmentFromUrl(
  rawAttr: string,
  host: string,
  text = "",
  extra: { name?: string; size?: number } = {}
): Attachment | null {
  const raw = rawAttr.trim().replace(/&amp;/g, "&");
  if (!raw || /^(javascript:|data:|mailto:|#)/i.test(raw)) return null;
  // 只收站点自身或绝对 http(s) 地址（正文里的外站链接不当附件）
  if (!/^(https?:|\/\/|\/)/i.test(raw)) return null;
  const url = shimuAbsoluteUrl(raw, host);
  const named = (extra.name ?? "").trim() || text.match(NAME_IN_TEXT)?.[0] || "";
  const path = url.split(/[?#]/)[0];
  const byExt = !!named || FILE_EXT_IN_PATH.test(path);
  const fileIdMatch = url.match(/(?:\/download\/|\/file\/|\/attach(?:ment)?\/|fileId=)(\d+)/i);
  const fileId = fileIdMatch ? Number(fileIdMatch[1]) : undefined;
  if (!byExt && !fileId) return null;
  const name = named || safeDecode(path.split("/").pop() ?? "") || t("shimu.attachment");
  return { url, name, fileId, size: extra.size, kind: kindOf(url, name) };
}

/** 字节数 → 人类可读（与站点附件卡同一口径） */
function formatSize(bytes?: number): string {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${i === 0 || v >= 10 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

function kindLabel(kind: AttachKind): string {
  switch (kind) {
    case "schematic":
      return t("shimu.kindSchematic");
    case "image":
      return t("shimu.kindImage");
    case "archive":
      return t("shimu.kindArchive");
    default:
      return t("shimu.kindFile");
  }
}

/**
 * 下载附件（含付费下单，必须带用户身份）。
 * `preview=true` 且是投影时，下载完回投影工坊直接打开预览。
 */
async function download(att: Attachment, preview = false) {
  if (!store.loggedIn) {
    message.warning(t("shimu.needLoginToDownload"));
    loginRef.value?.open();
    return;
  }
  downloading.value = att.url;
  try {
    const res = await api.shimuDownloadFile(att.url, att.name, att.fileId);
    message.success(t("shimu.downloaded", { name: res.name }));
    if (preview && att.kind === "schematic") {
      await router.push({ path: "/toolbox/schematic", query: { open: res.path } });
    }
  } catch (e) {
    handleError(e);
  } finally {
    downloading.value = "";
  }
}

/**
 * 正文里的链接点击交给我们处理：
 * - 附件（站内下载链接 / 投影、图片、压缩包）→ 走启动器下载（才能带上 token）；
 * - 其它链接 → 用系统浏览器打开，避免内部 webview 被导航走。
 */
function onBodyClick(e: MouseEvent) {
  const target = e.target as HTMLElement | null;
  if (!target) return;
  const host = store.config?.host ?? "https://www.mcimg.com";

  // 正文里的图片：点开就看大图（优先于链接，附件卡里的图除外）
  const img = target.closest("img");
  if (img && !img.closest("[data-url]")) {
    const src = img.getAttribute("src") ?? "";
    if (src) {
      e.preventDefault();
      const bodyImgs = [...document.querySelectorAll<HTMLImageElement>(".detail-body img")]
        .map((i) => i.getAttribute("src") ?? "")
        .filter(Boolean);
      const idx = bodyImgs.indexOf(src);
      openViewer(bodyImgs.length ? bodyImgs : [src], idx < 0 ? 0 : idx);
      return;
    }
  }

  // 站点编辑器的附件卡（地址在 data-url，本身不是链接）→ 直接下载
  const card = target.closest("[data-url], [data-href], [data-download]") as HTMLElement | null;
  if (card) {
    const raw =
      card.getAttribute("data-url") ?? card.getAttribute("data-href") ?? card.getAttribute("data-download") ?? "";
    const att = attachmentFromUrl(raw, host, card.textContent ?? "", cardMeta(card));
    if (att) {
      e.preventDefault();
      void download(att, false);
      return;
    }
  }

  const anchor = target.closest("a");
  if (!anchor) return;
  const href = anchor.getAttribute("href") ?? "";
  if (!href) return;
  e.preventDefault();
  const url = shimuAbsoluteUrl(href, host);
  const known = attachments.value.find((a) => a.url === url);
  const att = known ?? attachmentFromUrl(href, host, anchor.textContent ?? "");
  if (att) void download(att, att.kind === "schematic");
  else openUrl(url).catch(() => {});
}

async function logout() {
  try {
    await store.logout();
    message.success(t("shimu.loggedOut"));
  } catch (e) {
    handleError(e);
  }
}

/**
 * 帖子在前台的地址。路由已从站点前端 bundle 确认为 `post/:postId`，
 * 所以 `{HOST}/post/{postId}`；若接口直接返回了链接字段则优先用它。
 */
function postUrl(post: ShimuPost): string {
  const host = (store.config?.host ?? "https://www.mcimg.com").replace(/\/+$/, "");
  const raw = post as unknown as Record<string, unknown>;
  for (const key of ["url", "link", "shareUrl", "htmlUrl", "detailUrl", "postUrl"]) {
    const v = raw[key];
    if (typeof v === "string" && v.trim()) return shimuAbsoluteUrl(v, host);
  }
  return `${host}/post/${post.postId}`;
}

/** 打开该帖子在拾木空间的页面（列表页没有帖子时回退到站点首页） */
function openOnSite() {
  if (detail.value) {
    openUrl(postUrl(detail.value)).catch((e: unknown) => message.error(String(e)));
    return;
  }
  const host = store.config?.host ?? "https://www.mcimg.com";
  openUrl(host).catch((e: unknown) => message.error(String(e)));
}

function paidLabel(p: ShimuPost): string {
  if (p.isPaid !== "1") return "";
  return p.payTarget === "1" ? t("shimu.paidFileShort") : t("shimu.paidContentShort");
}

/** 付费标的 + 金额（payType：0 余额 / 1 积分） */
function paidDetailLabel(p: ShimuPost): string {
  const kind = paidLabel(p);
  if (!kind) return "";
  const amount = p.payAmount ?? 0;
  if (amount <= 0) return kind;
  const money = p.payType === "1" ? t("shimu.points", { n: amount }) : `¥${amount}`;
  return `${kind} · ${money}`;
}
</script>

<template>
  <div class="shimu-view">
    <!-- 参数栏 + 账号区 + 板块筛选（同一张卡，板块行只在列表态显示） -->
    <div class="glass param-bar">
      <div class="param-row">
        <n-input
          v-model:value="keyword"
          class="search"
          size="small"
          clearable
          :placeholder="t('shimu.searchPlaceholder')"
          @keyup.enter="submitSearch"
        />
        <n-select v-model:value="sort" class="sort" size="small" :options="sortOptions" />
        <button class="mini-btn" :disabled="loading" @click="submitSearch">
          {{ t("common.refresh") }}
        </button>
        <span v-if="!detail" class="count">{{ t("shimu.postCount", { n: total }) }}</span>

        <span class="flex-1" />

        <template v-if="store.loggedIn && store.user">
          <img
            v-if="store.avatarUrl && !brokenImgs.has(store.avatarUrl)"
            class="account-avatar"
            :src="store.avatarUrl"
            alt=""
            @error="markBroken(store.avatarUrl)"
          />
          <div class="account-info">
            <span class="account-name">{{ store.nickname }}</span>
            <span v-if="accountMeta" class="account-meta">{{ accountMeta }}</span>
          </div>
          <button class="mini-btn" @click="loginRef?.open()">{{ t("shimu.reauth") }}</button>
          <button class="mini-btn" @click="logout">{{ t("shimu.logout") }}</button>
        </template>
        <template v-else>
          <span class="guest-badge">{{ t("shimu.guestBadge") }}</span>
          <button class="mini-btn primary" @click="loginRef?.open()">{{ t("shimu.login") }}</button>
        </template>
      </div>

      <!-- 板块筛选：发帖人把投影帖发在哪个板块不固定，默认合并「投影相关」 -->
      <div v-if="!detail" class="board-row">
        <button
          v-for="c in boardChips"
          :key="c.key"
          class="board-chip"
          :class="{ active: board === c.key }"
          :disabled="loading"
          @click="selectBoard(c.key)"
        >
          {{ c.label }}
        </button>
      </div>
    </div>

    <!-- 详情 -->
    <div v-if="detail" class="glass detail">
      <div class="detail-head">
        <button class="link-btn" @click="closeDetail">← {{ t("shimu.back") }}</button>
        <n-button size="tiny" type="primary" quaternary @click="openOnSite">
          {{ t("shimu.openPost") }}
        </n-button>
      </div>

      <h2 class="detail-title">{{ detail.title }}</h2>
      <div class="detail-meta">
        <span v-if="detail.authorName">{{ detail.authorName }}</span>
        <span v-if="detail.createTime">{{ detail.createTime }}</span>
        <span>{{ t("shimu.stats", { views: detail.viewCount ?? 0, likes: detail.likeCount ?? 0 }) }}</span>
        <span v-if="paidDetailLabel(detail)" class="paid">{{ paidDetailLabel(detail) }}</span>
        <span v-if="tagsOf(detail)" class="tags">{{ tagsOf(detail) }}</span>
      </div>

      <div class="detail-scroll">
        <div v-if="gallery.length" class="gallery">
          <img
            v-for="(g, i) in gallery"
            :key="i"
            :src="g"
            alt=""
            loading="lazy"
            @click="openViewer(gallery.slice(), i)"
            @error="markBroken(g)"
          />
        </div>

        <div v-if="detailLoading" class="muted">{{ t("common.loading") }}</div>
        <div v-else-if="contentLocked" class="locked">{{ t("shimu.contentLocked") }}</div>
        <div v-else-if="bodyHtml" class="detail-body" v-html="bodyHtml" @click="onBodyClick"></div>
        <p v-else class="muted">{{ t("shimu.emptyContent") }}</p>

        <div class="files">
          <div class="files-head">{{ t("shimu.attachments") }}</div>
          <p v-if="detailLoading" class="muted">{{ t("common.loading") }}</p>
          <p v-else-if="!attachments.length" class="muted">{{ t("shimu.noAttachments") }}</p>
          <div v-for="att in attachments" :key="att.url" class="file">
            <span class="file-kind">{{ kindLabel(att.kind) }}</span>
            <span class="file-name" :title="att.url">{{ att.name }}</span>
            <span v-if="att.size" class="file-size">{{ formatSize(att.size) }}</span>
            <n-button
              size="tiny"
              :loading="downloading === att.url"
              @click="download(att, false)"
            >
              {{ t("shimu.download") }}
            </n-button>
            <n-button
              v-if="att.kind === 'schematic'"
              size="tiny"
              type="primary"
              :loading="downloading === att.url"
              @click="download(att, true)"
            >
              {{ t("shimu.downloadPreview") }}
            </n-button>
          </div>
        </div>
      </div>
    </div>

    <!-- 列表（滚动交给 App 的 .content，页面自己不造滚动容器） -->
    <template v-else>
      <div v-if="loading && !rows.length" class="glass empty">{{ t("common.loading") }}</div>
      <div v-else-if="!rows.length" class="glass empty">{{ t("shimu.emptyList") }}</div>
      <div v-else class="grid">
        <button v-for="p in rows" :key="p.postId" class="card glass clickable" @click="openPost(p)">
          <div class="cover">
            <img
              v-if="coverOf(p) && !brokenImgs.has(coverOf(p))"
              :src="coverOf(p)"
              alt=""
              loading="lazy"
              @error="markBroken(coverOf(p))"
            />
            <!-- 没有封面图就直接把标题写在占位块上（比只放首字更好认） -->
            <div v-else class="cover-ph">
              <span>{{ p.title?.trim() || t("shimu.untitled") }}</span>
            </div>
            <span v-if="paidLabel(p)" class="paid-badge">{{ paidLabel(p) }}</span>
          </div>
          <div class="card-body">
            <h3 class="card-title">{{ p.title }}</h3>
            <div class="card-meta">
              <span v-if="p.authorName" class="ellipsis">{{ p.authorName }}</span>
              <span v-if="p.createTime" class="nowrap">{{ p.createTime }}</span>
            </div>
            <div class="card-meta">
              <span class="nowrap">👁 {{ p.viewCount ?? 0 }}</span>
              <span class="nowrap">♥ {{ p.likeCount ?? 0 }}</span>
              <span v-if="tagsOf(p)" class="ellipsis">{{ tagsOf(p) }}</span>
            </div>
          </div>
        </button>
      </div>

      <div v-if="rows.length" class="pager">
        <SimplePagination v-model:page="page" :page-count="pageCount" />
      </div>
    </template>

    <ShimuLoginDialog ref="loginRef" />

    <!-- 图片放大查看（teleport 到 body，避免被内容区的 overflow 裁剪） -->
    <Teleport to="body">
      <div v-if="viewer" class="img-viewer" @click.self="closeViewer">
        <img class="img-viewer-img" :src="viewerSrc" alt="" @click.stop />
        <div class="img-viewer-bar">
          <span v-if="viewer.list.length > 1" class="iv-count">
            {{ viewer.index + 1 }} / {{ viewer.list.length }}
          </span>
          <button v-if="viewer.list.length > 1" class="iv-btn" @click="stepViewer(-1)">
            {{ t("shimu.viewerPrev") }}
          </button>
          <button v-if="viewer.list.length > 1" class="iv-btn" @click="stepViewer(1)">
            {{ t("shimu.viewerNext") }}
          </button>
          <button class="iv-btn" @click="closeViewer">{{ t("shimu.viewerClose") }}</button>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.shimu-view {
  display: flex;
  flex-direction: column;
  gap: var(--gap-page, 18px);
}
.param-bar {
  padding: 10px 14px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: none;
}
.param-row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
.search { flex: 1; min-width: 160px; max-width: 320px; }
.sort { width: 104px; flex: none; }
.flex-1 { flex: 1; }
.count { font-size: 11px; color: var(--text-3); white-space: nowrap; }
.muted { margin: 0; font-size: 12px; color: var(--text-3); }

/* —— 板块筛选（与参数栏同一张卡，用细分隔线分开） —— */
.board-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding-top: 8px;
  border-top: 1px solid var(--border);
}
.board-chip {
  padding: 4px 10px;
  border-radius: 999px;
  border: 1px solid var(--border);
  background: transparent;
  color: var(--text-3);
  font-family: inherit;
  font-size: 12px;
  line-height: 1.6;
  white-space: nowrap;
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s, color 0.15s;
}
.board-chip:hover { background: var(--panel-hover); color: var(--text-1); }
.board-chip.active {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-soft);
}
.board-chip:disabled { opacity: 0.5; cursor: default; }

/* —— 账号区 —— */
.account-avatar { width: 26px; height: 26px; border-radius: 8px; object-fit: cover; flex: none; }
.account-info { display: flex; flex-direction: column; line-height: 1.25; min-width: 0; }
.account-name { font-size: 12px; color: var(--text-1); }
.account-meta { font-size: 10px; color: var(--text-3); }
.guest-badge {
  padding: 3px 8px;
  border-radius: 6px;
  font-size: 11px;
  color: var(--text-3);
  background: var(--panel);
  border: 1px solid var(--border);
}

/* —— 通用按钮（与投影工坊一致） —— */
.mini-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--panel);
  color: var(--text-2);
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s, color 0.15s;
}
.mini-btn:hover { background: var(--panel-hover); color: var(--text-1); }
.mini-btn:disabled { opacity: 0.5; cursor: default; }
.mini-btn.primary { border-color: var(--accent); color: var(--accent); }
.mini-btn.primary:hover { background: var(--accent-soft); }
.link-btn {
  padding: 0;
  border: none;
  background: none;
  color: var(--text-3);
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
}
.link-btn:hover { color: var(--accent); }

/* —— 列表 —— */
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: var(--gap-grid, 16px);
  align-content: start;
}
.empty {
  min-height: 260px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  color: var(--text-3);
}
/* 卡片样式交给 .glass（--panel/--border），这里只加结构；
   不要覆写 background/border，否则浅色主题下会没有底色 */
.card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 0;
  border-radius: 12px;
  text-align: left;
  cursor: pointer;
  font-family: inherit;
}
.cover {
  position: relative;
  aspect-ratio: 16 / 10;
  background: var(--bg-2);
  overflow: hidden;
}
.cover img { width: 100%; height: 100%; object-fit: cover; display: block; }
/* 无封面时的占位块：直接显示标题（最多 3 行、超出省略，竖向居中） */
.cover-ph {
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 12px 14px;
  background: linear-gradient(135deg, #6b5b95, #352f44);
}
.cover-ph span {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  overflow: hidden;
  font-size: 13px;
  font-weight: 600;
  line-height: 1.5;
  text-align: center;
  overflow-wrap: anywhere;
  color: rgba(255, 255, 255, 0.94);
}
.paid-badge {
  position: absolute;
  top: 8px;
  right: 8px;
  padding: 2px 7px;
  border-radius: 6px;
  font-size: 10px;
  color: var(--accent);
  background: color-mix(in srgb, var(--feat-solid) 78%, transparent);
  border: 1px solid var(--accent-35);
  backdrop-filter: blur(6px);
}
.card-body { padding: 9px 11px 11px; display: flex; flex-direction: column; gap: 5px; }
.card-title {
  margin: 0;
  font-size: 13px;
  line-height: 1.45;
  color: var(--text-1);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.card-meta { display: flex; gap: 8px; font-size: 11px; color: var(--text-3); min-width: 0; }
.ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.nowrap { white-space: nowrap; }
.pager { display: flex; justify-content: center; flex: none; }

/* —— 详情（整页滚动，不自己造滚动区） —— */
.detail { display: flex; flex-direction: column; gap: 8px; padding: var(--pad-panel, 20px 24px); }
.detail-head { display: flex; align-items: center; justify-content: space-between; flex: none; }
.detail-title { margin: 0; font-size: 17px; line-height: 1.45; color: var(--text-1); }
.detail-meta { display: flex; flex-wrap: wrap; gap: 10px; font-size: 11px; color: var(--text-3); }
.detail-meta .paid { color: var(--accent); }
.detail-meta .tags { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 46%; }
.detail-scroll { display: flex; flex-direction: column; gap: 12px; padding-top: 4px; }
.gallery { display: flex; gap: 8px; overflow-x: auto; padding-bottom: 2px; }
.gallery img {
  height: 150px;
  border-radius: 10px;
  flex: none;
  object-fit: cover;
  border: 1px solid var(--border);
  cursor: zoom-in;
  transition: border-color 0.15s;
}
.gallery img:hover { border-color: var(--accent); }
.detail-body { font-size: 13px; line-height: 1.75; color: var(--text-1); overflow-wrap: anywhere; }
/* —— 富文本排版（帖子正文由站内编辑器生成） —— */
.detail-body :deep(img) { max-width: 100%; height: auto; border-radius: 8px; display: block; margin: 8px 0; cursor: zoom-in; }
.detail-body :deep(a) { color: var(--accent); text-decoration: none; }
.detail-body :deep(a:hover) { text-decoration: underline; }
.detail-body :deep(p) { margin: 0 0 10px; }
.detail-body :deep(h1),
.detail-body :deep(h2),
.detail-body :deep(h3),
.detail-body :deep(h4) { margin: 14px 0 8px; line-height: 1.4; color: var(--text-1); }
.detail-body :deep(h1) { font-size: 18px; }
.detail-body :deep(h2) { font-size: 16px; }
.detail-body :deep(h3) { font-size: 15px; }
.detail-body :deep(h4) { font-size: 14px; }
.detail-body :deep(ul),
.detail-body :deep(ol) { margin: 0 0 10px; padding-left: 22px; }
.detail-body :deep(li) { margin: 3px 0; }
.detail-body :deep(blockquote) {
  margin: 10px 0;
  padding: 6px 12px;
  border-left: 3px solid var(--accent-35);
  background: var(--panel);
  border-radius: 0 8px 8px 0;
  color: var(--text-2);
}
.detail-body :deep(pre) {
  margin: 10px 0;
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--bg-2);
  overflow-x: auto;
  font-size: 12px;
  line-height: 1.6;
}
.detail-body :deep(code) {
  font-family: "Cascadia Code", Consolas, monospace;
  font-size: 12px;
  background: var(--panel);
  padding: 1px 5px;
  border-radius: 5px;
}
.detail-body :deep(pre code) { background: none; padding: 0; }
.detail-body :deep(table) {
  width: 100%;
  margin: 10px 0;
  border-collapse: collapse;
  font-size: 12px;
  display: block;
  overflow-x: auto;
}
.detail-body :deep(th),
.detail-body :deep(td) { border: 1px solid var(--border); padding: 6px 10px; text-align: left; }
.detail-body :deep(th) { background: var(--panel); color: var(--text-1); }
.detail-body :deep(hr) { border: none; border-top: 1px solid var(--border); margin: 14px 0; }
.detail-body :deep(video),
.detail-body :deep(iframe) { max-width: 100%; border-radius: 8px; border: none; }
/* 正文里的附件卡（.re-file[data-url]，站点编辑器产物）：整块可点，点了就下载 */
.detail-body :deep(.re-file) {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin: 6px 0;
  padding: 6px 10px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--panel);
  font-size: 12px;
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s;
}
.detail-body :deep(.re-file:hover) { border-color: var(--accent); background: var(--panel-hover); }
.detail-body :deep(.re-file-name) { color: var(--accent); }
.detail-body :deep(.re-file-size),
.detail-body :deep(.re-file-tag) { color: var(--text-3); font-size: 11px; }
.detail-body :deep(:first-child) { margin-top: 0; }
.detail-body :deep(:last-child) { margin-bottom: 0; }
.locked { font-size: 12px; line-height: 1.7; color: var(--accent); }
.files { display: flex; flex-direction: column; gap: 6px; }
.files-head { font-size: 12px; color: var(--text-3); }
.file {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
  border-radius: 8px;
  border: 1px solid var(--border);
  background: var(--panel);
}
.file-kind {
  flex: none;
  padding: 1px 7px;
  border-radius: 5px;
  font-size: 10px;
  color: var(--text-3);
  background: var(--bg-2);
  border: 1px solid var(--border);
}
.file-size { flex: none; font-size: 11px; color: var(--text-3); }

/* —— 图片放大查看 —— */
.img-viewer {
  position: fixed;
  inset: 0;
  z-index: 3000;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.82);
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
}
.img-viewer-img {
  max-width: 92vw;
  max-height: 84vh;
  object-fit: contain;
  border-radius: 8px;
  box-shadow: 0 18px 50px var(--k-50);
  cursor: default;
}
.img-viewer-bar {
  position: absolute;
  bottom: 22px;
  left: 50%;
  transform: translateX(-50%);
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  border-radius: 10px;
  border: 1px solid rgba(255, 255, 255, 0.16);
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
}
.iv-btn {
  padding: 4px 10px;
  border-radius: 7px;
  border: 1px solid rgba(255, 255, 255, 0.16);
  background: rgba(255, 255, 255, 0.06);
  color: #f2f3f7;
  font-family: inherit;
  font-size: 12px;
  cursor: pointer;
}
.iv-btn:hover { background: rgba(255, 255, 255, 0.14); }
.iv-count { font-size: 12px; color: rgba(255, 255, 255, 0.75); font-variant-numeric: tabular-nums; }
.file-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: var(--text-1);
}
</style>
