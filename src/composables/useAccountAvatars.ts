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
 * 避免两处各拉一遍。localStorage 仍按原 key 持久化（qookix:avatar / qookix:yggavatar）。
 */

const offlineAvatarCache = reactive<Record<string, string>>({});
const onlineAvatarCache = reactive<Record<string, string>>({});
const yggAvatarCache = reactive<Record<string, string>>({});
/** 每个账号已失败的在线头像源序号，驱动三级回退链。 */
const avatarAttempt = reactive<Record<string, number>>({});

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
  const cached = localStorage.getItem(`qookix:avatar:${uuid}`);
  if (cached) onlineAvatarCache[uuid] = cached;
}

async function refreshOnlineAvatar(uuid: string, url: string) {
  try {
    const dataUrl = await api.fetchImageDataURL(url);
    // 与现有缓存相同则跳过，避免无谓的 <img src> 切换闪烁
    if (dataUrl && dataUrl !== onlineAvatarCache[uuid]) {
      onlineAvatarCache[uuid] = dataUrl;
      localStorage.setItem(`qookix:avatar:${uuid}`, dataUrl);
    }
  } catch {
    /* ignore */
  }
}

async function loadYggAvatar(acc: Extract<Account, { type: "yggdrasil" }>, force = false) {
  const key = `qookix:yggavatar:${acc.uuid}`;
  if (!force) {
    const cached = localStorage.getItem(key);
    if (cached) {
      yggAvatarCache[acc.uuid] = cached;
      return;
    }
  }
  try {
    // 皮肤站账号的 profile id 就是 uuid
    const tex = await api.yggdrasilTextures(acc.server, acc.uuid);
    const skin = tex?.skin || "";
    if (skin) {
      const avatar = await skinToAvatar(skin);
      if (avatar) {
        yggAvatarCache[acc.uuid] = avatar;
        localStorage.setItem(key, avatar);
      }
    }
  } catch {
    /* 皮肤站不可达时静默，显示默认头像 */
  }
}

/** watches 只挂一次（模块级单例）。 */
let initialized = false;
function ensureWatches() {
  if (initialized) return;
  initialized = true;
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
          const cached = localStorage.getItem(`qookix:yggavatar:${acc.uuid}`);
          if (cached) yggAvatarCache[acc.uuid] = cached;
        }
      }
      // 再后台异步拉取/刷新；加载完成后写入缓存，reactive 自动替换为最新头像。
      for (const acc of accounts.accounts) {
        if (acc.type === "offline" && !offlineAvatarCache[acc.uuid]) {
          const skin = await loadOfflineSkin(acc.uuid);
          if (skin) {
            const avatar = await skinToAvatar(skin.src);
            if (avatar) offlineAvatarCache[acc.uuid] = avatar;
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
          if (avatar) offlineAvatarCache[cur.uuid] = avatar;
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
}

export function useAccountAvatars() {
  ensureWatches();
  const accounts = useAccountsStore();

  function avatar(uuid: string): string {
    const offline = offlineAvatarCache[uuid];
    if (offline) return offline;
    const ygg = yggAvatarCache[uuid];
    if (ygg) return ygg;
    // 皮肤站账号不走正版头像源（crafatar 等只认正版 UUID）
    if (accounts.accounts.find((a) => a.uuid === uuid)?.type === "yggdrasil") return "";
    const cached = onlineAvatarCache[uuid];
    if (cached) return cached;
    const i = avatarAttempt[uuid] ?? 0;
    if (i < AVATAR_SOURCES.length) {
      const base = AVATAR_SOURCES[i](uuid);
      const sep = base.includes("?") ? "&" : "?";
      return `${base}${sep}v=${accounts.avatarVersion}`;
    }
    return "";
  }

  function onAvatarError(uuid: string) {
    const next = (avatarAttempt[uuid] ?? 0) + 1;
    if (next <= AVATAR_SOURCES.length) avatarAttempt[uuid] = next;
  }

  return { avatar, onAvatarError };
}
