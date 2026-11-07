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
import { computed, onMounted, reactive, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { NButton, NInput, NSelect, NModal, useMessage } from "naive-ui";
import DOMPurify from "dompurify";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useI18n } from "vue-i18n";
import SimplePagination from "../components/SimplePagination.vue";
import ShimuLoginDialog from "../components/ShimuLoginDialog.vue";
import ShimuPostCard from "../components/ShimuPostCard.vue";
import ImageViewer from "../components/ImageViewer.vue";
import { api } from "../api";
import { shimuAbsoluteUrl, shimuErrorMessage, useShimuStore } from "../stores/shimu";
import { useInstancesStore } from "../stores/instances";
import type { ShimuCategory, ShimuPost, ShimuPostQuery } from "../types";

/**
 * `embedded` = 被「发现」页当作一个分类内嵌：此时搜索 / 排序 / 每页数量 / 视图 /
 * 目标实例全部由发现页的工具栏控制，本组件只负责列表、详情与安装。
 */
const props = withDefaults(
  defineProps<{
    embedded?: boolean;
    view?: "grid" | "list" | "compact";
    /** 发现页选的投影来源（`all` = 所有已接入来源合并） */
    source?: "all" | "shimu";
    keyword?: string;
    sort?: "new" | "hot" | "featured";
    pageSize?: number;
    /** 安装目标实例（发现页工具栏里选的那个） */
    targetInstanceId?: string | null;
  }>(),
  {
    embedded: false,
    view: "grid",
    source: "all",
    keyword: "",
    sort: "hot",
    pageSize: 12,
    targetInstanceId: null,
  }
);

const { t } = useI18n();
const message = useMessage();
const router = useRouter();
const store = useShimuStore();
const instancesStore = useInstancesStore();

const PAGE_SIZE = 12;

const loginRef = ref<InstanceType<typeof ShimuLoginDialog> | null>(null);
/** 登录态读完了吗：读完之前不显示未登录提示，避免闪 */
const statusReady = ref(false);

const keyword = ref(props.keyword);
const sort = ref<"new" | "hot" | "featured">(props.sort);
const pageSize = ref(props.pageSize);
const page = ref(1);
const rows = ref<ShimuPost[]>([]);
const total = ref(0);
const loading = ref(false);

// 发现页工具栏变化 → 同步到本组件并重新查询
watch(
  () => [props.keyword, props.sort, props.pageSize] as const,
  ([kw, st, ps]) => {
    if (!props.embedded) return;
    let dirty = false;
    if (keyword.value !== kw) { keyword.value = kw; dirty = true; }
    if (sort.value !== st) { sort.value = st; dirty = true; }
    if (pageSize.value !== (ps || PAGE_SIZE)) { pageSize.value = ps || PAGE_SIZE; dirty = true; }
    if (dirty) requery();
  }
);

// 发现页切换「来源」→ 回到第一页重查
watch(
  () => props.source,
  () => {
    if (props.embedded) requery();
  }
);

/**
 * 板块范围：站点自己的「投影」板块只有几十帖，投影作品实际大量发在「建筑」
 * 「红石」「生电」板块（这个站本身就是建筑投影社区）。所以查询固定聚合这几个
 * 板块，界面上不再摆板块筛选 —— 只留一个「投影」入口，不出现「投影相关 / 投影」
 * 两个语义重叠的按钮。
 */
const categories = ref<ShimuCategory[]>([]);
/** 一起纳入「投影」范围的板块名（站点分类名，匹配时忽略空格） */
const RELATED_BOARD_NAMES = ["投影", "红石", "生电", "建筑"];
/** 分类接口拿不到时的兜底 ID（拾木空间固定值） */
const RELATED_BOARD_IDS = [1, 3, 4, 5];

const relatedIds = computed(() => {
  const ids = categories.value
    .filter((c) => RELATED_BOARD_NAMES.some((n) => (c.name ?? "").replace(/\s+/g, "").includes(n)))
    .map((c) => c.categoryId);
  return ids.length ? ids : RELATED_BOARD_IDS;
});

const detail = ref<ShimuPost | null>(null);
const detailLoading = ref(false);
const downloading = ref("");

/** 内嵌模式下卡片按发现页工具栏选的三视图渲染；独立页面保持网格 */
const cardView = computed(() => (props.embedded ? props.view : "grid"));

const emit = defineEmits<{ "detail-open": [boolean] }>();
// 内嵌时进详情要把发现页工具栏收起来，否则上下两套控件打架
watch(detail, (d) => {
  if (props.embedded) emit("detail-open", !!d);
});

/** 卡片逐个进场（与发现页内容卡片同一节奏） */
function cardDelay(i: number): string {
  const step = Math.min(0.045, 0.6 / Math.max(1, rows.value.length));
  return `${(0.06 + i * step).toFixed(3)}s`;
}

/** 投影下载完成后弹出的实例选择器：把投影复制到所选实例的 schematics 目录 */
const instancePicker = ref<{ sourcePath: string; filename: string } | null>(null);
const pickerInstanceId = ref<string | null>(null);
const pickerBusy = ref(false);
const instanceOptions = computed(() =>
  instancesStore.instances.map((i) => ({ label: i.name, value: i.id }))
);
const instancePickerVisible = computed({
  get: () => instancePicker.value !== null,
  set: (v) => { if (!v) instancePicker.value = null; },
});

/** 图片放大查看（图集与正文图片共用；走标准查看器，缩放 / 拖拽 / 左右切换） */
const lightboxOpen = ref(false);
const lightboxIndex = ref(0);
const lightboxImages = ref<string[]>([]);

function openViewer(list: string[], index = 0) {
  const items = list.filter(Boolean);
  if (!items.length) return;
  lightboxImages.value = items;
  lightboxIndex.value = Math.min(Math.max(index, 0), items.length - 1);
  lightboxOpen.value = true;
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

const pageCount = computed(() => Math.max(1, Math.ceil(total.value / (pageSize.value || PAGE_SIZE))));

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
/** 付费帖未购买时，服务端连附件地址都不下发（附件列表为空）—— 别误报成「需要评论」 */
const paidFileLocked = computed(
  () => detail.value?.isPaid === "1" && !attachments.value.length
);
/** 回复可见帖未评论时，服务端只下发 [hidecontent] 裸标记 + .hide-before 提示，不含真正内容。
 *  靠检测原始 content 里的隐藏标记判断，不能靠 bodyHtml 是否为空（帖子可能有公开引言 + 隐藏块）。 */
const needComment = computed(() => {
  if (detailLoading.value || contentLocked.value) return false;
  return replyLocked(detail.value);
});

/**
 * 附件拿不到的真正原因：付费内容 / 评论后可见 / 确实没传投影。
 * 详情页与「卡片直接安装」共用同一套判断，避免给出「没有投影文件」这种误导性提示。
 */
function replyLocked(p: ShimuPost | null | undefined): boolean {
  const c = p?.content ?? "";
  const hasHide = /\[hidecontent/i.test(c) || c.includes("tinymce-hide") || c.includes("hide-before");
  return hasHide && p?.commented !== true;
}

function attachLockReason(p: ShimuPost): "paid" | "reply" | null {
  const isPaidContent = p.isPaid === "1" && p.payTarget === "0";
  if (isPaidContent && !(p.content ?? "").trim()) return "paid";
  if (replyLocked(p)) return "reply";
  return null;
}

/**
 * 正文里是否存在投影附件卡。
 *
 * 付费帖未购买时，服务端只下发 `data-name`（甚至 data-url 为空串），
 * `extractAttachments` 会因为拿不到下载地址把整张卡跳过 —— 于是「明明有投影文件」
 * 却被报成没有附件。判断提示文案时得以这张卡为准。
 */
const SCHEMATIC_CARD_RE = /data-name\s*=\s*"[^"]*\.(?:litematic|schem|schematic|mcstructure)\b/i;

function hasSchematicCard(p: ShimuPost): boolean {
  return SCHEMATIC_CARD_RE.test(p.content ?? "");
}

/** 付费金额：`1 积分` / `¥3`（没有金额时给空串） */
function priceLabel(p: ShimuPost): string {
  const amount = p.payAmount ?? 0;
  if (amount <= 0) return "";
  return p.payType === "1" ? t("shimu.points", { n: amount }) : `¥${amount}`;
}

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
  // 登录态读到之前不显示「未登录」提示，避免刚进页面闪一下
  void store.init().finally(() => { statusReady.value = true; });
  void loadCategories();
  void search();
  // 内嵌时工具栏已经选了实例，这里补一次实例列表好显示实例名
  if (props.embedded) void instancesStore.load();
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
// 独立页面里排序下拉改成立刻生效；内嵌时排序由发现页工具栏驱动（见上面的 props watch）
watch(sort, () => {
  if (!props.embedded) requery();
});

function handleError(e: unknown) {
  message.error(shimuErrorMessage(e));
  if (String(e).includes("__shimu_expired__")) void store.refreshStatus();
}

async function search() {
  loading.value = true;
  try {
    // 来源：目前接进来的只有拾木空间，`all`（全部来源）与 `shimu` 取数完全相同；
    // 以后接入第二个投影站时，这里按 props.source 分流（或并行请求后合并分页）。
    const query: ShimuPostQuery = {
      keyword: keyword.value.trim() || undefined,
      sort: sort.value,
      // 只取带投影文件的帖子（站点侧过滤，每页能凑满，不用客户端再筛）
      hasProjection: 1,
      pageNum: page.value,
      pageSize: pageSize.value || PAGE_SIZE,
    };
    // 固定按「投影」范围查询：投影 + 建筑 + 红石 + 生电 合并取一页
    query.categoryIds = relatedIds.value;
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

/** 拉取站点板块列表（失败时 relatedIds 走兜底 ID） */
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
    if (att.kind === "schematic") {
      if (preview) {
        await router.push({ path: "/toolbox/schematic", query: { open: res.path } });
      } else {
        void instancesStore.load();
        pickerInstanceId.value = null;
        instancePicker.value = { sourcePath: res.path, filename: res.name };
      }
    }
  } catch (e) {
    handleError(e);
  } finally {
    downloading.value = "";
  }
}

/** 把刚下载的投影复制到所选实例的 schematics 目录 */
async function confirmAddToInstance() {
  const picker = instancePicker.value;
  const id = pickerInstanceId.value;
  if (!picker || !id) return;
  pickerBusy.value = true;
  try {
    await api.copyFileToInstanceSchematics(picker.sourcePath, id, picker.filename);
    const inst = instancesStore.get(id);
    message.success(t("shimu.addedToInstance", { name: inst?.name ?? id }));
    instancePicker.value = null;
  } catch (e) {
    handleError(e);
  } finally {
    pickerBusy.value = false;
  }
}

/* —— 卡片上的「安装」：一步装进工具栏里选中的实例 —— */
const installingPost = ref<number | null>(null);
/** 本次会话里已装进当前目标实例的帖子（换实例后清空） */
const installedPosts = reactive(new Set<number>());

const targetInstance = computed(() =>
  props.targetInstanceId
    ? instancesStore.instances.find((i) => i.id === props.targetInstanceId) ?? null
    : null
);
const targetInstanceName = computed(() => targetInstance.value?.name ?? "");

watch(
  () => props.targetInstanceId,
  () => installedPosts.clear()
);

/**
 * 拉详情并取出该帖的投影附件。
 *
 * 列表接口不带附件，必须再取一次详情；拿不到下载地址时按真实原因给提示
 * （付费未购买 / 需回复 / 只有附件卡没链接 / 根本没投影）。
 */
async function resolveSchematic(p: ShimuPost): Promise<Attachment | null> {
  const raw = await api.shimuPostDetail(p.postId);
  const data = ((raw.data ?? raw) as ShimuPost) ?? p;
  const atts = extractAttachments(
    data.content ?? "",
    store.config?.host ?? "https://www.mcimg.com"
  );
  const file = atts.find((a) => a.kind === "schematic");
  if (file) return file;
  // 附件卡在正文里、但没拿到下载地址：多半是付费帖（未购买时服务端不下发 data-url），
  // 这时不能说"没有投影文件"，要按真实原因给提示。
  const card = hasSchematicCard(data);
  const reason = attachLockReason(data);
  const price = priceLabel(data);
  if (reason === "paid") {
    message.warning(t("shimu.installNeedPurchase"));
  } else if (reason === "reply") {
    message.warning(t("shimu.installNeedComment"));
  } else if (data.isPaid === "1") {
    // 付费帖（内容或附件）拿不到附件地址：未购买时服务端本来就不下发 data-url
    message.warning(t("shimu.installNeedPurchaseFile", { price: price || t("shimu.paidFileShort") }));
  } else if (card) {
    message.warning(t("shimu.installLinkMissing"));
  } else {
    message.warning(t("shimu.noSchematic"));
  }
  return null;
}

/**
 * 卡片直达安装：拉详情 → 取第一个投影附件 → 下载 → 复制到目标实例的 schematics。
 * 没登录先拉登录框；没选实例就提示（选择器在发现页工具栏上）。
 */
async function installToInstance(p: ShimuPost) {
  if (!store.loggedIn) {
    message.warning(t("shimu.needLoginToDownload"));
    loginRef.value?.open();
    return;
  }
  const instId = props.targetInstanceId;
  // 只有内嵌模式（发现页工具栏）才有实例选择；独立页面走「下载完再选实例」
  if (props.embedded && !instId) {
    message.warning(t("shimu.pickInstanceFirst"));
    return;
  }
  installingPost.value = p.postId;
  try {
    const file = await resolveSchematic(p);
    if (!file) return;
    if (!instId) {
      await download(file);
      return;
    }
    const res = await api.shimuDownloadFile(file.url, file.name, file.fileId);
    await api.copyFileToInstanceSchematics(res.path, instId, res.name);
    installedPosts.add(p.postId);
    message.success(t("shimu.installedToInstance", { name: targetInstanceName.value || instId }));
  } catch (e) {
    handleError(e);
  } finally {
    installingPost.value = null;
  }
}

/* —— 卡片上的「预览」：下载投影后直接打开投影工坊，不用先选实例 —— */
const previewingPost = ref<number | null>(null);

async function previewFromCard(p: ShimuPost) {
  if (!store.loggedIn) {
    message.warning(t("shimu.needLoginToDownload"));
    loginRef.value?.open();
    return;
  }
  previewingPost.value = p.postId;
  try {
    const file = await resolveSchematic(p);
    if (!file) return;
    // download() 带 preview=true：下完直接跳到 /toolbox/schematic?open=<path>
    await download(file, true);
  } catch (e) {
    handleError(e);
  } finally {
    previewingPost.value = null;
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
  <div class="shimu-view" data-tauri-drag-region>
    <!-- 参数栏 + 账号区（内嵌到发现页时由发现页工具栏接管，这里整块不渲染） -->
    <div v-if="!embedded" class="glass param-bar">
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
    </div>

    <!-- 详情 -->
    <div v-if="detail" class="glass detail">
      <div class="detail-head">
        <button class="link-btn" @click="closeDetail">← {{ t("shimu.back") }}</button>
        <button class="mini-btn primary" @click="openOnSite">{{ t("shimu.openPost") }}</button>
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
        <div v-else-if="bodyHtml" class="detail-body" v-html="bodyHtml" @click="onBodyClick"></div>
        <p v-else-if="!contentLocked && !needComment" class="muted">{{ t("shimu.emptyContent") }}</p>

        <div v-if="detailLoading || attachments.length || contentLocked || needComment" class="files">
          <div class="files-head">{{ t("shimu.attachments") }}</div>
          <p v-if="detailLoading" class="muted">{{ t("common.loading") }}</p>
          <template v-else-if="attachments.length">
            <div v-for="att in attachments" :key="att.url" class="file">
              <span class="file-kind">{{ kindLabel(att.kind) }}</span>
              <span class="file-name" :title="att.url">{{ att.name }}</span>
              <span v-if="att.size" class="file-size">{{ formatSize(att.size) }}</span>
              <button
                class="mini-btn"
                :disabled="downloading === att.url"
                @click="download(att, false)"
              >
                {{ downloading === att.url ? t("common.loading") : t("shimu.download") }}
              </button>
              <button
                v-if="att.kind === 'schematic'"
                class="mini-btn primary"
                :disabled="downloading === att.url"
                @click="download(att, true)"
              >
                {{ downloading === att.url ? t("common.loading") : t("shimu.downloadPreview") }}
              </button>
            </div>
          </template>
          <p v-else-if="contentLocked" class="locked">
            {{ t("shimu.attachmentsLockedPay") }}
            <button class="locked-link" @click="openOnSite">{{ t("shimu.openPost") }} →</button>
          </p>
          <p v-else-if="paidFileLocked" class="locked">
            {{ t("shimu.attachmentsLockedPaidFile", { price: priceLabel(detail!) || t("shimu.paidFileShort") }) }}
            <button class="locked-link" @click="openOnSite">{{ t("shimu.openPost") }} →</button>
          </p>
          <p v-else class="locked">
            {{ t("shimu.attachmentsLockedReply") }}
            <button class="locked-link" @click="openOnSite">{{ t("shimu.openPost") }} →</button>
          </p>
        </div>
      </div>
    </div>

    <!-- 列表（滚动交给 App 的 .content，页面自己不造滚动容器） -->
    <template v-else>
      <!-- 第三方账号未授权：浏览不受影响，但下载 / 购买要授权，这里给一条直达登录的提示 -->
      <div v-if="embedded && statusReady && !store.loggedIn" class="login-notice">
        <span class="ln-text">
          {{ store.status.expired ? t("shimu.loginNoticeExpired") : t("shimu.loginNotice") }}
        </span>
        <button class="locked-link" @click="loginRef?.open()">
          {{ store.status.expired ? t("shimu.reauth") : t("shimu.login") }} →
        </button>
      </div>

      <!-- 加载态用骨架卡，和发现页其它分类同一套 shimmer（形状跟着投影卡片走） -->
      <div v-if="loading && !rows.length" class="grid" :class="`view-${cardView}`">
        <div v-for="i in 8" :key="i" class="skel-card glass" :class="`view-${cardView}`">
          <div class="skel-cover"></div>
          <div class="skel-body">
            <div class="skel-line w-80"></div>
            <div class="skel-line w-45"></div>
          </div>
          <div class="skel-foot">
            <div class="skel-line w-30"></div>
            <div class="skel-btn"></div>
          </div>
        </div>
      </div>
      <div v-else-if="!rows.length" class="glass empty">{{ t("shimu.emptyList") }}</div>
      <div v-else class="grid" :class="`view-${cardView}`">
        <ShimuPostCard
          v-for="(p, i) in rows"
          :key="p.postId"
          :post="p"
          :view="cardView"
          :installing="installingPost === p.postId"
          :previewing="previewingPost === p.postId"
          :installed="installedPosts.has(p.postId)"
          :instance-name="targetInstanceName"
          :style="{ animationDelay: cardDelay(i) }"
          @open="openPost"
          @install="installToInstance"
          @preview="previewFromCard"
        />
      </div>

      <div v-if="rows.length" class="pager">
        <SimplePagination v-model:page="page" :page-count="pageCount" />
      </div>
    </template>

    <ShimuLoginDialog ref="loginRef" />

    <!-- 投影下载完成后：选择要复制到的实例 -->
    <NModal
      v-model:show="instancePickerVisible"
      preset="card"
      :title="t('shimu.selectInstance')"
      style="max-width: 420px"
      :bordered="false"
      size="small"
    >
      <div v-if="!instanceOptions.length" class="muted">{{ t("shimu.noInstance") }}</div>
      <div v-else class="picker-row">
        <n-select
          v-model:value="pickerInstanceId"
          :options="instanceOptions"
          :placeholder="t('shimu.selectInstance')"
          filterable
        />
        <n-button
          type="primary"
          :loading="pickerBusy"
          :disabled="!pickerInstanceId"
          @click="confirmAddToInstance"
        >
          {{ t("shimu.addToInstance") }}
        </n-button>
      </div>
    </NModal>

    <!-- 图片放大查看：标准查看器（缩放 / 拖拽 / 左右切换） -->
    <ImageViewer
      v-model:open="lightboxOpen"
      v-model:index="lightboxIndex"
      :images="lightboxImages"
    />
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
  /* 投影卡片以封面为主，列宽比内容分类的 220px 略宽，图才看得清 */
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: var(--gap-grid, 16px);
  align-content: start;
}
.grid.view-list,
.grid.view-compact {
  grid-template-columns: 1fr;
}
.grid.view-compact {
  gap: 8px;
}
/* 卡片逐个进场（与发现页内容卡片同一节奏） */
.grid :deep(.p-card) {
  animation: shimu-card-enter 0.34s cubic-bezier(0.22, 0.61, 0.36, 1) backwards;
}
@keyframes shimu-card-enter {
  from { transform: translateY(28px); opacity: 0; }
  to { transform: none; opacity: 1; }
}
@media (prefers-reduced-motion: reduce) {
  .grid :deep(.p-card) { animation: none; }
}
.empty {
  min-height: 260px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  color: var(--text-3);
}
/* —— 骨架屏布局：颜色变量 / shimmer / 条块尺寸来自 styles.css 公共原子 —— */
.skel-card {
  padding: 0;
  gap: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.skel-cover {
  width: 100%;
  aspect-ratio: 16 / 10;
}
.skel-body {
  padding: 10px 12px 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.skel-foot {
  padding: 10px 12px 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.skel-card.view-list,
.skel-card.view-compact {
  flex-direction: row;
  align-items: center;
  padding: 14px;
  gap: 14px;
}
.skel-card.view-list .skel-cover,
.skel-card.view-compact .skel-cover {
  width: 46px;
  height: 46px;
  aspect-ratio: auto;
  border-radius: 10px;
  flex: none;
}
.skel-card.view-compact {
  padding: 5px 12px;
  gap: 10px;
}
.skel-card.view-compact .skel-cover {
  width: 30px;
  height: 30px;
  border-radius: 7px;
}
.skel-card.view-list .skel-body,
.skel-card.view-compact .skel-body {
  flex: 1;
  min-width: 0;
  padding: 0;
}
.skel-card.view-list .skel-foot,
.skel-card.view-compact .skel-foot {
  padding: 0;
  flex: none;
  gap: 8px;
}
.skel-card.view-compact .skel-btn {
  width: 64px;
  height: 24px;
}
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
.detail-body { font-size: 13px; line-height: 1.75; color: var(--text-1); overflow-wrap: anywhere; user-select: text; cursor: text; }
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
.locked {
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--accent-soft);
  border-left: 3px solid var(--accent);
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-1);
}
.locked-link {
  margin-left: 6px;
  padding: 0;
  border: none;
  background: none;
  color: var(--accent);
  font: inherit;
  text-decoration: underline;
  cursor: pointer;
}
.locked-link:hover { color: var(--accent-hover); }

/* 未登录第三方账号的提示条：贴列表顶部，右侧给登录入口 */
.login-notice {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
  padding: 9px 12px;
  border-radius: 8px;
  background: var(--accent-soft);
  border-left: 3px solid var(--accent);
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-1);
}
.ln-text {
  flex: 1;
  min-width: 160px;
}
.files { display: flex; flex-direction: column; gap: 8px; }
.files-head { font-size: 12px; color: var(--text-3); }
.file {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-radius: 8px;
  background: var(--panel);
}
.file-kind {
  flex: none;
  padding: 2px 8px;
  border-radius: 5px;
  font-size: 11px;
  color: var(--text-3);
  background: var(--bg-2);
}
.file-size { flex: none; font-size: 11px; color: var(--text-3); }
.file .mini-btn { padding: 4px 10px; font-size: 11px; }

/* —— 图片放大查看 —— */
.file-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: var(--text-1);
}
.picker-row { display: flex; gap: 8px; align-items: center; }
.picker-row .n-select { flex: 1; min-width: 200px; }
</style>
