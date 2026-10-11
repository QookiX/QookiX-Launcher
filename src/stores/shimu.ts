/**
 * 拾木空间（www.mcimg.com）账号：OAuth 授权登录 + 用户资料。
 *
 * 仅用于「投影工坊」拉取站内投影帖，**不是启动器的启动账号**。
 * 流程见 `src-tauri/src/shimu.rs`：浏览器授权 → qookix://oauth/callback → 换 token。
 * access_token 3 天有效且没有 refresh_token，过期后必须重新授权。
 */
import { defineStore } from "pinia";
import i18n from "../i18n";
import { api } from "../api";
import type { ShimuConfig, ShimuStatus, ShimuUser } from "../types";

/** 后端约定的错误标记（见 src-tauri/src/shimu.rs） */
export const SHIMU_ERR_NOT_LOGGED_IN = "__shimu_not_logged_in__";
export const SHIMU_ERR_EXPIRED = "__shimu_expired__";

/** 服务端给的是相对路径（/profile/...），拼上站点地址才能直接当图片用 */
export function shimuAbsoluteUrl(path: string, host?: string): string {
  const p = (path ?? "").trim();
  if (!p) return "";
  // 已经是绝对地址（含 data:/blob:，例如内联封面）就直接用
  if (/^(https?:|data:|blob:)/i.test(p)) return p;
  // 协议相对地址（//cdn.example.com/x.png）补 https:
  if (p.startsWith("//")) return `https:${p}`;
  const base = (host || "https://www.mcimg.com").replace(/\/+$/, "");
  return p.startsWith("/") ? `${base}${p}` : `${base}/${p}`;
}

/** 后端错误 → 界面文案（登录态相关错误给专门提示） */
export function shimuErrorMessage(err: unknown): string {
  const raw = String(err instanceof Error ? err.message : err).replace(/^Error:\s*/, "");
  if (raw.includes(SHIMU_ERR_NOT_LOGGED_IN)) return i18n.global.t("shimu.errNotLoggedIn");
  if (raw.includes(SHIMU_ERR_EXPIRED)) return i18n.global.t("shimu.errExpired");
  return raw;
}

export const useShimuStore = defineStore("shimu", {
  state: () => ({
    config: null as ShimuConfig | null,
    status: {
      loggedIn: false,
      expired: false,
      expiresAt: 0,
      user: null,
    } as ShimuStatus,
    /** 正在等待浏览器回调的授权 state（空 = 没在等） */
    pendingState: "",
    /** 正在换 token */
    loading: false,
  }),
  getters: {
    user: (s): ShimuUser | null => s.status.user,
    loggedIn: (s): boolean => s.status.loggedIn,
    configured: (s): boolean => s.config?.configured ?? false,
    nickname: (s): string => s.status.user?.nickName || s.status.user?.userName || "",
    avatarUrl: (s): string =>
      s.status.user?.avatar ? shimuAbsoluteUrl(s.status.user.avatar, s.config?.host) : "",
    /** 会员展示名（0 非会员时不显示） */
    memberLabel: (s): string =>
      (s.status.user?.memberLevel ?? 0) > 0 ? s.status.user?.memberName || "" : "",
  },
  actions: {
    /** 应用启动或打开弹窗时调用：读配置 + 登录态 */
    async init() {
      await Promise.all([this.loadConfig(), this.refreshStatus()]);
    },
    async loadConfig() {
      try {
        this.config = await api.shimuConfig();
      } catch {
        /* 读不到配置不影响其它功能，登录时会再报错 */
      }
    },
    async refreshStatus() {
      try {
        this.status = await api.shimuStatus();
      } catch {
        /* 保持原状态 */
      }
    },
    /** 第一步：拿授权页地址（调用方负责用系统浏览器打开） */
    async startAuth(): Promise<string> {
      const info = await api.shimuOauthStart();
      this.pendingState = info.state;
      return info.url;
    },
    /** 第三步：授权码换 access_token（深链回调与手动粘贴共用） */
    async completeAuth(code: string, state: string) {
      this.loading = true;
      try {
        const res = await api.shimuOauthComplete(code, state);
        if (res.user) this.status = { ...this.status, loggedIn: true, expired: false, user: res.user };
        this.pendingState = "";
        await this.refreshStatus();
        return res;
      } finally {
        this.loading = false;
      }
    },
    /**
     * 处理回调地址（深链 `qookix://oauth/callback?code=..&state=..` 或用户手动粘贴）。
     * 返回 `denied` 表示用户在授权页点了拒绝。
     */
    async handleCallback(input: string): Promise<"ok" | "denied"> {
      const raw = input.trim();
      let url: URL;
      try {
        url = new URL(raw);
      } catch {
        throw new Error(i18n.global.t("shimu.errBadCallback"));
      }
      if (url.searchParams.get("error")) return "denied";
      const code = url.searchParams.get("code")?.trim() ?? "";
      const state = url.searchParams.get("state")?.trim() ?? "";
      if (!code) throw new Error(i18n.global.t("shimu.errNoCode"));
      await this.completeAuth(code, state);
      return "ok";
    },
    async setCredentials(clientId: string, clientSecret?: string) {
      this.config = await api.shimuSetCredentials(clientId, clientSecret);
    },
    async logout() {
      await api.shimuLogout();
      this.pendingState = "";
      await this.refreshStatus();
    },
  },
});
