/**
 * QookiX 相关的**本地**偏好：好友备注名、分享预设。
 *
 * 这些都只存在本机（localStorage），不往后端写：
 * - 备注名是自己认人用的，别人看到的不该变
 * - 分享预设（标题 / 玩法 / 版本）是「上次怎么填的」，换台机器不该同步
 */
import { defineStore } from "pinia";

const NOTES_KEY = "qookix:friend_notes";
const SHARE_KEY = "qookix:share_preset";

export interface SharePreset {
  title: string;
  playType: string;
  version: string;
}

const EMPTY_SHARE: SharePreset = { title: "", playType: "", version: "" };

function readJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    const v = JSON.parse(raw);
    return v && typeof v === "object" ? (v as T) : fallback;
  } catch {
    return fallback;
  }
}

function writeJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* localStorage 不可用：备注这次不持久化，不影响其它功能 */
  }
}

export const useQookixPrefsStore = defineStore("qookixPrefs", {
  state: () => ({
    /** uid → 备注名 */
    notes: readJson<Record<string, string>>(NOTES_KEY, {}),
    share: readJson<SharePreset>(SHARE_KEY, { ...EMPTY_SHARE }),
  }),
  actions: {
    noteFor(uid: number): string {
      return this.notes[String(uid)] ?? "";
    },
    /** 空字符串表示清除备注 */
    setNote(uid: number, note: string) {
      const key = String(uid);
      const v = (note ?? "").trim();
      if (v) this.notes[key] = v;
      else delete this.notes[key];
      writeJson(NOTES_KEY, this.notes);
    },
    /** 显示名优先级：备注 > 昵称 > 用户名 */
    displayName(user: { uid: number; username: string; nickname?: string }): string {
      return this.noteFor(user.uid) || user.nickname || user.username;
    },
    saveSharePreset(preset: SharePreset) {
      this.share = { ...preset };
      writeJson(SHARE_KEY, this.share);
    },
  },
});
