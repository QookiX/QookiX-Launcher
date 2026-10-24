import { api } from "../api";

export interface OfflineSkin {
  /** base64 PNG data URL */
  src: string;
  /** `null` when no variant was recorded → caller should auto-detect */
  variant: "classic" | "slim" | null;
}

/**
 * Load a saved offline skin for a uuid from the backend disk file
 * `skins/offline/<uuid>.png`（权威源）。不再镜像到 localStorage：后端已持久化
 * 皮肤与模型，前端多存一份 base64 PNG 只会挤占共享的 ~5MB 配额，换来的仅是
 * 省掉一次 IPC，不划算。
 */
export async function loadOfflineSkin(uuid: string): Promise<OfflineSkin | null> {
  try {
    const res = await api.getOfflineSkin(uuid);
    return res ? { src: res.src, variant: res.variant } : null;
  } catch {
    /* backend unavailable — caller decides what to fall back to */
    return null;
  }
}
