import { defineStore } from "pinia";
import { openUrl } from "@tauri-apps/plugin-opener";
import i18n from "../i18n";
import { api } from "../api";
import { useSettingsStore } from "./settings";
import type { Account, YggOauthConfig } from "../types";

/** LittleSkin OAuth2 回调换 token 后的角色列表（多角色时交给前端选择） */
interface YggOauthResult {
  profiles: { id: string; name: string }[];
  server: string;
  serverName: string;
  links: { homepage: string; register: string };
}

export const useAccountsStore = defineStore("accounts", {
  state: () => ({
    accounts: [] as Account[],
    msFlow: null as { userCode: string; verificationUri: string; expiresIn: number } | null,
    msPolling: false,
    msError: "",
    msSuccess: "",
    msFailed: "",
    /** Set by the sidebar chip / home button to open the account manager popover */
    showManager: false,
    /** Bumped after skin apply to force avatar refresh */
    avatarVersion: 0,
    /** 最近一次成功拉取的时间戳，用于短 TTL 内跳过重复请求 */
    lastLoadedAt: 0,
    // ---- LittleSkin OAuth2（Yggdrasil Connect） ----
    /** 接入配置（configured 决定是否露出 OAuth2 登录入口） */
    yggOauthConfig: null as YggOauthConfig | null,
    /** 正在等待浏览器回调（对话框据此展示「请在浏览器中完成授权」） */
    yggOauthWaiting: false,
    /** 回调拿到多角色时的选择列表（单角色已自动添加，不会落到这里） */
    yggOauthResult: null as YggOauthResult | null,
  }),
  getters: {
    /** The currently playing account: settings.selected_account, else first */
    current: (s): Account | null => {
      const settings = useSettingsStore();
      const sel = settings.settings?.selected_account;
      if (sel) {
        const found = s.accounts.find((a) => a.uuid === sel);
        if (found) return found;
      }
      return s.accounts[0] ?? null;
    },
  },
  actions: {
    /** 后台静默刷新账号列表，失败保留旧数据 */
    async refresh() {
      try {
        this.accounts = await api.listAccounts();
        this.lastLoadedAt = Date.now();
      } catch {
        /* 后台刷新失败保留旧数据 */
      }
    },
    /**
     * 拉取账号列表。
     * - 已有数据时采用 stale-while-revalidate：立即返回旧数据，后台静默刷新替换。
     * - `force=true` 时始终前台拉取（用于登录/登出后强制刷新）。
     */
    async load(force = false) {
      const now = Date.now();
      if (!force && this.lastLoadedAt && now - this.lastLoadedAt < 3000) return;
      const hasData = this.accounts.length > 0;
      if (hasData && !force) {
        void this.refresh();
        return;
      }
      this.accounts = await api.listAccounts();
      this.lastLoadedAt = Date.now();
    },
    /** Select the current playing account (persisted globally). */
    async select(uuid: string) {
      const settings = useSettingsStore();
      await settings.patch({ selected_account: uuid });
    },
    async addOffline(username: string, uuid?: string) {
      const acc = await api.loginOffline(username, uuid);
      await this.load();
      // auto-select the new account
      await this.select(acc.uuid);
      return acc;
    },
    async startMs() {
      this.msError = "";
      const info = await api.loginMsStart();
      this.msFlow = { userCode: info.userCode, verificationUri: info.verificationUri, expiresIn: info.expiresIn };
      this.pollMs();
    },
    async pollMs() {
      if (this.msPolling) return;
      this.msPolling = true;
      try {
        const maxAttempts = this.msFlow ? Math.ceil(this.msFlow.expiresIn / 5) + 12 : 120;
        for (let i = 0; i < maxAttempts; i++) {
          if (!this.msFlow) return;
          try {
            const acc = await api.loginMsPoll();
            this.msFlow = null;
            this.msError = "";
            await this.load(true);
            await this.select(acc.uuid);
            this.msSuccess = `${acc.username} 登录成功`;
            return;
          } catch (e: unknown) {
            if (!this.msFlow) return;
            const msg = String(e);
            if (msg === "__auth_pending__") {
              await new Promise((r) => setTimeout(r, 5000));
              continue;
            }
            this.msError = msg.replace(/^Error:\s*/, "");
            return;
          }
        }
        this.msError = "等待超时，请重新尝试";
      } finally {
        this.msPolling = false;
      }
    },
    /** Manually trigger an immediate poll (skips the 5s wait). */
    async manualCheck() {
      if (!this.msFlow || this.msError) return;
      try {
        const acc = await api.loginMsPoll();
        this.msFlow = null;
        this.msError = "";
        this.msPolling = false;
        await this.load(true);
        await this.select(acc.uuid);
        this.msSuccess = `${acc.username} 登录成功`;
      } catch (e: unknown) {
        const msg = String(e);
        if (msg !== "__auth_pending__" && this.msFlow) {
          this.msError = msg.replace(/^Error:\s*/, "");
          this.msPolling = false;
        }
      }
    },
    async remove(uuid: string) {
      await api.logoutAccount(uuid);
      const settings = useSettingsStore();
      if (settings.settings?.selected_account === uuid) {
        await settings.patch({ selected_account: null });
      }
      await this.load();
    },
    bumpAvatar() {
      this.avatarVersion++;
    },
    // ---- LittleSkin OAuth2（Yggdrasil Connect） ----
    /** 拉取接入配置（打开添加账号弹窗时调用，决定是否露出 OAuth2 入口） */
    async loadYggOauthConfig() {
      try {
        this.yggOauthConfig = await api.yggdrasilOauthConfig();
      } catch {
        this.yggOauthConfig = null;
      }
    },
    /** 开发者工具页：写入 LittleSkin 接入凭证（仅开发构建可用） */
    async setYggOauthCredentials(clientId: string, clientSecret?: string) {
      this.yggOauthConfig = await api.yggdrasilSetCredentials(clientId, clientSecret);
    },
    /** 第一步：拿授权页地址并用系统浏览器打开 */
    async startYggOauth() {
      const info = await api.yggdrasilOauthStart();
      this.yggOauthWaiting = true;
      this.yggOauthResult = null;
      try {
        await openUrl(info.url);
      } catch (e) {
        this.yggOauthWaiting = false;
        throw e;
      }
    },
    /**
     * 处理深链回调（`qookix://oauth/littleskin/callback?code=..&state=..`）。
     * 返回 `denied` 表示用户在授权页点了拒绝；单角色直接添加，多角色交给前端选择。
     */
    async handleYggOauthCallback(url: URL): Promise<"ok" | "denied"> {
      if (url.searchParams.get("error")) {
        this.yggOauthWaiting = false;
        return "denied";
      }
      const code = url.searchParams.get("code")?.trim() ?? "";
      const state = url.searchParams.get("state")?.trim() ?? "";
      if (!code) throw new Error(i18n.global.t("yggLogin.oauthErrNoCode"));
      try {
        const res = await api.yggdrasilOauthComplete(code, state);
        const profiles = res.profiles;
        // 单角色：直接添加；多角色且对话框在等：交给前端选择；
        // 多角色但对话框已关：自动选第一个（用户可在账号管理里切换）
        if (profiles.length === 1 || !this.yggOauthWaiting) {
          const p = profiles[0];
          const acc = await this.addYggOauthAccount(p.id);
          // 用 msSuccess 复用 AccountDrawer 的「提示 + 收起弹窗」逻辑
          this.msSuccess =
            profiles.length > 1
              ? i18n.global.t("yggLogin.oauthAutoProfile", { name: p.name })
              : i18n.global.t("accountChip.yggAccountAdded", { username: acc.username });
          return "ok";
        }
        this.yggOauthResult = res;
        return "ok";
      } catch (e) {
        this.yggOauthWaiting = false;
        throw e;
      }
    },
    /** 第四步：为选定角色签发 Minecraft 令牌并登记为皮肤站账号 */
    async addYggOauthAccount(profileId: string): Promise<Account> {
      const acc = await api.yggdrasilOauthAddAccount(profileId);
      await this.load(true);
      await this.select(acc.uuid);
      this.yggOauthResult = null;
      this.yggOauthWaiting = false;
      return acc;
    },
  },
});
