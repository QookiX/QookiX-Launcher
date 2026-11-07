import { reactive, watch } from "vue";
import { useAccountsStore } from "../stores/accounts";
import { loadOfflineSkin } from "./useOfflineSkin";
import { api } from "../api";
import type { Account } from "../types";

/**
 * 账号头像的共享加载与缓存。
 *
 * 离线账号头像来自本地皮肤裁剪；正版账号走 mc-heads/minotar/crafatar 三级回退；
 * 皮肤站账号走 yggdrasil textures 接口裁剪。三套缓存均为模块级单例，
 * AccountChip（标题栏芯片 + 悬停快速切换）与 AccountDrawer（账号管理抽屉）共用，
 * 避免两处各拉一遍。localStorage 仍按原 key 持久化（qookix:avatar / qookix:yggavatar /
 * qookix:offlineavatar，后者只存裁好的 8×8 头像，不是整张皮肤）。
 *
 * **渲染约定：永远「本地缓存优先」**——`avatar()` 任何时刻都能同步命中磁盘缓存，
 * 网络/后端的新结果只是后台把缓存替换掉。否则账号列表是异步拉回来的，首帧渲染时
 * 内存缓存还是空的，头像会先掉成默认图标再「长」出来。
 */

const OFFLINE_AVATAR_PREFIX = "qookix:offlineavatar:";
const ONLINE_AVATAR_PREFIX = "qookix:avatar:";
const YGG_AVATAR_PREFIX = "qookix:yggavatar:";

const offlineAvatarCache = reactive<Record<string, string>>({});
const onlineAvatarCache = reactive<Record<string, string>>({});
const yggAvatarCache = reactive<Record<string, string>>({});
/** 每个账号已失败的在线头像源序号，驱动三级回退链。 */
const avatarAttempt = reactive<Record<string, number>>({});

/** 缓存里只认 data URL（历史版本可能留下空值/半截 JSON 之类，别让它们顶掉在线兜底） */
function isImageDataUrl(v: string): boolean {
  return v.startsWith("data:image/");
}

/**
 * 把 localStorage 里的头像缓存一次性灌进内存。
 *
 * 按 key 前缀扫，**不依赖账号列表**：账号本身要等 `listAccounts` 回来，
 * 而缓存 key 里就有 uuid，先恢复头像，首帧就能直接用缓存渲染。
 */
function restoreAvatarCaches() {
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (!key) continue;
      const value = localStorage.getItem(key);
      if (!value || !isImageDataUrl(value)) continue;
      if (key.startsWith(OFFLINE_AVATAR_PREFIX)) {
        const uuid = key.slice(OFFLINE_AVATAR_PREFIX.length);
        if (!offlineAvatarCache[uuid]) offlineAvatarCache[uuid] = value;
      } else if (key.startsWith(ONLINE_AVATAR_PREFIX)) {
        const uuid = key.slice(ONLINE_AVATAR_PREFIX.length);
        if (!onlineAvatarCache[uuid]) onlineAvatarCache[uuid] = value;
      } else if (key.startsWith(YGG_AVATAR_PREFIX)) {
        const uuid = key.slice(YGG_AVATAR_PREFIX.length);
        if (!yggAvatarCache[uuid]) yggAvatarCache[uuid] = value;
      }
    }
  } catch {
    /* localStorage 不可用（隐私模式等）时退化为纯内存缓存 */
  }
}

/**
 * 内存里没有就从 localStorage 现读一份（只读，不回写内存）。
 *
 * 兜底用：即便 `restoreAvatarCaches()` 没赶上（缓存是之后才写进去的），
 * 渲染时也能同步拿到头像，不会空一帧。
 * 刻意**不写回**内存缓存：这个函数会在渲染函数里被调用，渲染期改响应式状态
 * 会引发额外的重渲染（严重时是递归更新），缓存预热交给 watch/restore 做。
 */
function cachedOrLocal(prefix: string, uuid: string, cache: Record<string, string>): string {
  const hit = cache[uuid];
  if (hit) return hit;
  try {
    const stored = localStorage.getItem(`${prefix}${uuid}`) ?? "";
    return isImageDataUrl(stored) ? stored : "";
  } catch {
    return "";
  }
}

/** 清掉某个账号的全部头像缓存（内存 + 磁盘），用于坏条目自愈 */
function forgetAvatar(uuid: string) {
  delete offlineAvatarCache[uuid];
  delete onlineAvatarCache[uuid];
  delete yggAvatarCache[uuid];
  try {
    localStorage.removeItem(`${OFFLINE_AVATAR_PREFIX}${uuid}`);
    localStorage.removeItem(`${ONLINE_AVATAR_PREFIX}${uuid}`);
    localStorage.removeItem(`${YGG_AVATAR_PREFIX}${uuid}`);
  } catch {
    /* ignore */
  }
}

/** 离线头像也落盘：省掉「每次启动都要等一次后端 IPC 才有头像」的空窗 */
function rememberOfflineAvatar(uuid: string, dataUrl: string) {
  offlineAvatarCache[uuid] = dataUrl;
  try {
    localStorage.setItem(`${OFFLINE_AVATAR_PREFIX}${uuid}`, dataUrl);
  } catch {
    /* 配额满时只留内存，不影响功能 */
  }
}

const AVATAR_SOURCES: Array<(uuid: string) => string> = [
  (u) => `https://mc-heads.net/avatar/${u}/96`,
  (u) => `https://minotar.net/helm/${u}/96`,
  (u) => `https://crafatar.com/avatars/${u}?size=96&overlay`,
];

function skinToAvatar(skinDataUrl: string): Promise<string> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = 8;
      canvas.height = 8;
      const ctx = canvas.getContext("2d");
      if (!ctx) return resolve("");
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(img, 8, 8, 8, 8, 0, 0, 8, 8); // 头部基础层
      if (img.height >= 64) {
        ctx.drawImage(img, 40, 8, 8, 8, 0, 0, 8, 8); // 头部第二层（overlay）
      }
      resolve(canvas.toDataURL("image/png"));
    };
    img.onerror = () => resolve("");
    img.src = skinDataUrl;
  });
}

function loadOnlineAvatarCache(uuid: string) {
  const cached = localStorage.getItem(`${ONLINE_AVATAR_PREFIX}${uuid}`);
  if (cached) onlineAvatarCache[uuid] = cached;
}

async function refreshOnlineAvatar(uuid: string, url: string) {
  try {
    // 换肤后头像要立刻变：给源 URL 加时间戳，避免头像站/中间缓存把旧图返回来
    const sep = url.includes("?") ? "&" : "?";
    const dataUrl = await api.fetchImageDataURL(`${url}${sep}t=${Date.now()}`);
    // 与现有缓存相同则跳过，避免无谓的 <img src> 切换闪烁
    if (dataUrl && dataUrl !== onlineAvatarCache[uuid]) {
      onlineAvatarCache[uuid] = dataUrl;
      localStorage.setItem(`${ONLINE_AVATAR_PREFIX}${uuid}`, dataUrl);
    }
    // 拉到图说明链路通了：把「三级回退」的失败计数清掉，别再卡在兜底图标上
    if (dataUrl) delete avatarAttempt[uuid];
  } catch {
    /* ignore */
  }
}

/**
 * 皮肤站头像失败后的重试间隔（毫秒）。
 *
 * 皮肤站账号的头像**只有后端拉取纹理这一个来源**（crafatar 之类只认正版 UUID，
 * 见 `avatar()`），拿不到就只能显示默认人像图标。而启动器常在开机自启、
 * 系统网络/代理尚未就绪时就发出这次请求，一次失败就永久停在默认图标上，
 * 所以这里做有限重试，并把失败原因打到控制台便于排查。
 */
const YGG_RETRY_DELAYS = [2000, 5000, 10000];
/** 同一账号的并发去重：多处同时触发时只跑一条重试链 */
const yggInFlight = new Set<string>();

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function loadYggAvatar(acc: Extract<Account, { type: "yggdrasil" }>, force = false) {
  const key = `qookix:yggavatar:${acc.uuid}`;
  if (!force) {
    const cached = localStorage.getItem(key);
    if (cached) {
      yggAvatarCache[acc.uuid] = cached;
      return;
    }
  }
  if (yggInFlight.has(acc.uuid)) return;
  yggInFlight.add(acc.uuid);
  try {
    for (let attempt = 0; ; attempt++) {
      try {
        // 皮肤站账号的 profile id 就是 uuid；force 时带缓存击穿参数，换肤后能立刻拿到新图
        const tex = await api.yggdrasilTextures(acc.server, acc.uuid, force);
        const skin = tex?.skin || "";
        // 角色在皮肤站还没上传皮肤：重试也变不出头像，直接收手（保留默认图标）
        if (!skin) return;
        const avatar = await skinToAvatar(skin);
        if (avatar) {
          yggAvatarCache[acc.uuid] = avatar;
          localStorage.setItem(key, avatar);
        }
        return;
      } catch (e) {
        if (attempt >= YGG_RETRY_DELAYS.length) {
          console.warn(`[avatar] 皮肤站头像加载失败（${acc.server} / ${acc.uuid}）：`, e);
          return;
        }
        await sleep(YGG_RETRY_DELAYS[attempt]);
      }
    }
  } finally {
    yggInFlight.delete(acc.uuid);
  }
}

/** watches 只挂一次（模块级单例）。 */
let initialized = false;
function ensureWatches() {
  if (initialized) return;
  initialized = true;
  // 先同步恢复磁盘上的头像缓存（不依赖账号列表是否已经加载完）
  restoreAvatarCaches();
  const accounts = useAccountsStore();

  watch(
    () => accounts.accounts.map((a) => `${a.uuid}:${a.type}`).join(","),
    async () => {
      // 先同步预填本地缓存（localStorage），确保首帧渲染 avatar() 即可命中，
      // 避免在线源拉取期间头像先消失再出现。正版/皮肤站头像都走这套「本地优先」。
      // 必须在任意 await 之前完成，否则被离线账号的 loadOfflineSkin 挂起后会
      // 晚于组件首帧渲染，导致头像短暂落到在线直链/兜底图标。
      for (const acc of accounts.accounts) {
        if (acc.type === "microsoft" && !onlineAvatarCache[acc.uuid]) {
          loadOnlineAvatarCache(acc.uuid);
        }
        if (acc.type === "yggdrasil" && !yggAvatarCache[acc.uuid]) {
          const cached = localStorage.getItem(`${YGG_AVATAR_PREFIX}${acc.uuid}`);
          if (cached) yggAvatarCache[acc.uuid] = cached;
        }
      }
      // 再后台异步拉取/刷新；加载完成后写入缓存，reactive 自动替换为最新头像。
      for (const acc of accounts.accounts) {
        if (acc.type === "offline" && !offlineAvatarCache[acc.uuid]) {
          const skin = await loadOfflineSkin(acc.uuid);
          if (skin) {
            const avatar = await skinToAvatar(skin.src);
            if (avatar) rememberOfflineAvatar(acc.uuid, avatar);
          }
        }
        if (acc.type === "microsoft") {
          refreshOnlineAvatar(acc.uuid, AVATAR_SOURCES[0](acc.uuid));
        }
        if (acc.type === "yggdrasil" && !yggAvatarCache[acc.uuid]) {
          void loadYggAvatar(acc);
        }
      }
    },
    { immediate: true },
  );

  watch(
    () => accounts.avatarVersion,
    async () => {
      const cur = accounts.current;
      if (cur && cur.type === "offline") {
        const skin = await loadOfflineSkin(cur.uuid);
        if (skin) {
          const avatar = await skinToAvatar(skin.src);
          if (avatar) rememberOfflineAvatar(cur.uuid, avatar);
        }
      }
      if (cur && cur.type === "microsoft") {
        refreshOnlineAvatar(cur.uuid, AVATAR_SOURCES[0](cur.uuid));
      }
      if (cur && cur.type === "yggdrasil") {
        void loadYggAvatar(cur, true);
      }
    },
  );

  // 切到某个皮肤站账号时若本地还没有头像，补拉一次：
  // 首帧那次请求失败过，也应在这里自愈，而不是一直挂着默认图标。
  watch(
    () => accounts.current?.uuid,
    (uuid) => {
      if (!uuid) return;
      const acc = accounts.accounts.find((a) => a.uuid === uuid);
      if (acc?.type === "yggdrasil" && !yggAvatarCache[acc.uuid]) {
        void loadYggAvatar(acc);
      }
    },
  );

  // 网络恢复（开机自启时网卡还没就绪的典型场景）：把所有缺头像的皮肤站账号重试一遍
  if (typeof window !== "undefined") {
    window.addEventListener("online", () => {
      for (const acc of accounts.accounts) {
        if (acc.type === "yggdrasil" && !yggAvatarCache[acc.uuid]) void loadYggAvatar(acc);
      }
    });
  }
}

export function useAccountAvatars() {
  ensureWatches();
  const accounts = useAccountsStore();

  /**
   * 取账号头像。**永远本地缓存优先**：能命中就同步返回缓存，绝不等网络，
   * 这样刷新时只是把 `<img src>` 换成新图，不会先掉成默认图标。
   */
  function avatar(uuid: string): string {
    const offline = cachedOrLocal(OFFLINE_AVATAR_PREFIX, uuid, offlineAvatarCache);
    if (offline) return offline;
    const ygg = cachedOrLocal(YGG_AVATAR_PREFIX, uuid, yggAvatarCache);
    if (ygg) return ygg;
    // 皮肤站账号不走正版头像源（crafatar 等只认正版 UUID）
    if (accounts.accounts.find((a) => a.uuid === uuid)?.type === "yggdrasil") return "";
    const cached = cachedOrLocal(ONLINE_AVATAR_PREFIX, uuid, onlineAvatarCache);
    if (cached) return cached;
    const i = avatarAttempt[uuid] ?? 0;
    if (i < AVATAR_SOURCES.length) {
      const base = AVATAR_SOURCES[i](uuid);
      const sep = base.includes("?") ? "&" : "?";
      return `${base}${sep}v=${accounts.avatarVersion}`;
    }
    return "";
  }

  /**
   * 头像图加载失败。命中本地缓存时说明缓存条目坏了（写入被截断 / 旧格式 / 站点换了文件名），
   * 丢掉它并重新取一份——否则坏数据会把头像永久钉在空白上（缓存优先级高于在线兜底）。
   */
  function onAvatarError(uuid: string) {
    const hasStale = !!(yggAvatarCache[uuid] || offlineAvatarCache[uuid] || onlineAvatarCache[uuid]);
    if (hasStale) {
      forgetAvatar(uuid);
      delete avatarAttempt[uuid];
      const acc = accounts.accounts.find((a) => a.uuid === uuid);
      if (acc?.type === "yggdrasil") {
        void loadYggAvatar(acc, true);
      } else if (acc?.type === "offline") {
        void (async () => {
          const skin = await loadOfflineSkin(uuid);
          if (!skin) return;
          const avatar = await skinToAvatar(skin.src);
          if (avatar) rememberOfflineAvatar(uuid, avatar);
        })();
      } else if (acc?.type === "microsoft") {
        void refreshOnlineAvatar(uuid, AVATAR_SOURCES[0](uuid));
      }
      return;
    }
    const next = (avatarAttempt[uuid] ?? 0) + 1;
    if (next <= AVATAR_SOURCES.length) avatarAttempt[uuid] = next;
  }

  return { avatar, onAvatarError };
}
