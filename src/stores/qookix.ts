/**
 * QookiX 账号（可选）：好友 + 陶瓦联机房间目录。
 *
 * 只承载「好友」和「房间发现」两件事，与 Minecraft 启动账号完全无关。
 * 未登录时所有方法都会抛出 `not_logged_in`，界面据此显示登录引导而不是报错。
 *
 * 后台轮询在 Rust 侧（`src-tauri/src/qookix/poller.rs`），前端只消费事件，
 * 不自己起定时器——保证只有一个数据源。
 */
import { defineStore } from "pinia";
import { listen } from "@tauri-apps/api/event";
import i18n from "../i18n";
import { api } from "../api";
import { notifyError } from "../composables/notify";
import { notifyOs } from "../utils/osNotify";
import type {
  QookixApiError,
  QookixBlock,
  QookixFriend,
  QookixInvite,
  QookixNotifications,
  QookixRequest,
  QookixRoom,
  QookixShare,
  QookixStatus,
} from "../types";

/** 命令层错误码：未登录（不是错误，是引导登录的信号） */
export const QOOKIX_NOT_LOGGED_IN = "not_logged_in";

const EMPTY_STATUS: QookixStatus = { loggedIn: false, expiresAt: 0, user: null, share: null };

function asApiError(err: unknown): QookixApiError | null {
  if (err && typeof err === "object" && "code" in err) {
    const e = err as QookixApiError;
    return typeof e.code === "string" ? e : null;
  }
  return null;
}

/** 错误码 → 用户可读文案；未知错误原样返回 */
export function qookixErrorMessage(err: unknown): string {
  const t = i18n.global.t;
  const e = asApiError(err);
  if (!e) return String(err instanceof Error ? err.message : err);
  const known: Record<string, string> = {
    [QOOKIX_NOT_LOGGED_IN]: t("qookix.err.notLoggedIn"),
    invalid_credentials: t("qookix.err.invalidCredentials"),
    email_not_verified: t("qookix.err.emailNotVerified"),
    rate_limited: t("qookix.err.rateLimited"),
    network_error: t("qookix.err.network"),
    username_taken: t("qookix.err.usernameTaken"),
    email_taken: t("qookix.err.emailTaken"),
    already_friends: t("qookix.err.alreadyFriends"),
    self_request: t("qookix.err.selfRequest"),
    blocked: t("qookix.err.blocked"),
    blocked_by_target: t("qookix.err.blockedByTarget"),
    request_not_found: t("qookix.err.requestGone"),
    invite_not_found: t("qookix.err.inviteGone"),
    not_friend: t("qookix.err.notFriend"),
    not_room_owner: t("qookix.err.notOwner"),
    room_not_found: t("qookix.err.roomGone"),
    room_closed: t("qookix.err.roomClosed"),
    room_inactive: t("qookix.err.roomInactive"),
    room_removed: t("qookix.err.roomRemoved"),
    invalid_room_code: t("qookix.err.invalidRoomCode"),
    email_send_failed: t("qookix.err.emailSendFailed"),
    invalid_username: t("qookix.err.invalidUsername"),
    invalid_email: t("qookix.err.invalidEmail"),
    invalid_password: t("qookix.err.invalidPassword"),
    invalid_nickname: t("qookix.err.invalidNickname"),
    account_exists: t("qookix.err.accountExists"),
    // 正版绑定（MC_ACCOUNT_BIND v0.3）
    mc_token_malformed: t("qookix.err.mcTokenMalformed"),
    mc_token_invalid: t("qookix.err.mcTokenInvalid"),
    mc_no_entitlement: t("qookix.err.mcNoEntitlement"),
    mc_uuid_bound_to_other: t("qookix.err.mcUuidBoundToOther"),
    mc_account_already_bound: t("qookix.err.mcAccountAlreadyBound"),
    // 冷却固定 24h，服务端 Retry-After 也是这个值
    mc_unbind_cooldown: t("qookix.err.mcUnbindCooldown", { hours: 24 }),
    mc_not_bound: t("qookix.err.mcNotBound"),
    mc_verified_readonly: t("qookix.err.mcVerifiedReadonly"),
    mc_profile_unavailable: t("qookix.err.mcProfileUnavailable"),
    mc_offline_account: t("qookix.err.mcOfflineAccount"),
    mc_no_account: t("qookix.err.mcNoAccount"),
    mc_bind_rate_limited: t("qookix.err.rateLimited"),
    // 举报 / 改邮箱新增的服务端错误码
    already_reported: t("qookix.err.alreadyReported"),
    report_rate_limited: t("qookix.err.reportRateLimited"),
    same_email: t("qookix.err.sameEmail"),
    email_change_rate_limited: t("qookix.err.rateLimited"),
    // 大厅挂牌要求已验证：服务端后续会自己拦，这里给同一句提示
    mc_verified_required: t("qookix.rooms.needVerified"),
  };
  return known[e.code] ?? e.message;
}

export const useQookixStore = defineStore("qookix", {
  state: () => ({
    status: { ...EMPTY_STATUS } as QookixStatus,
    friends: [] as QookixFriend[],
    incoming: [] as QookixRequest[],
    outgoing: [] as QookixRequest[],
    invites: [] as QookixInvite[],
    blocks: [] as QookixBlock[],
    /** 好友房间（好友 + 自己） */
    rooms: [] as QookixRoom[],
    /** uid → 绑定的 Minecraft 账号（好友/房间卡片画皮肤头与徽章用） */
    profiles: {} as Record<
      number,
      { mc_name: string; mc_uuid: string; mc_verified: boolean }
    >,
    /** 未读的好友申请 / 邀请数（事件累加，打开面板清零） */
    unread: 0,
    loading: false,
    initialized: false,
  }),
  getters: {
    loggedIn: (s): boolean => s.status.loggedIn,
    user: (s) => s.status.user,
    nickname: (s): string => s.status.user?.nickname || s.status.user?.username || "",
    share: (s): QookixShare | null => s.status.share ?? null,
    pendingCount: (s): number => s.incoming.length + s.invites.length,
  },
  actions: {
    /** 应用启动时调用一次：读登录态并注册后台事件 */
    async init() {
      if (this.initialized) return;
      this.initialized = true;
      await this.refreshStatus();
      await listen<QookixNotifications>("qookix://notifications", (e) => {
        this.mergeNotifications(e.payload);
      });
      await listen<{ friends: QookixFriend[] }>("qookix://friends", (e) => {
        this.friends = e.payload?.friends ?? [];
        void this.loadProfiles();
      });
      // 后台每 30 秒对账一次房间目录：被管理员下架 / 房主停止分享 / 心跳超时
      // 的房间要及时从列表里消失，不能等用户手动点刷新。
      await listen<{ rooms: QookixRoom[] }>("qookix://rooms", (e) => {
        this.rooms = e.payload?.rooms ?? [];
      });
      await listen<QookixStatus>("qookix://session", (e) => {
        this.status = e.payload ?? { ...EMPTY_STATUS };
      });
      await listen<{ sharing: boolean; reason?: string | null }>("qookix://share", (e) => {
        const reason = e.payload?.reason;
        if (reason) {
          // 房间被下架 / 关闭时用户没有点任何按钮，必须主动告知
          const key: Record<string, string> = {
            room_removed: "qookix.err.roomRemoved",
            room_closed: "qookix.err.roomClosed",
            room_not_found: "qookix.err.roomGone",
            not_room_owner: "qookix.err.notOwner",
          };
          notifyError(i18n.global.t(key[reason] ?? "qookix.err.roomGone"));
        }
        void this.refreshStatus();
      });
    },
    async refreshStatus() {
      try {
        this.status = await api.qookixStatus();
      } catch {
        this.status = { ...EMPTY_STATUS };
      }
    },
    async login(login: string, password: string) {
      this.loading = true;
      try {
        this.status = await api.qookixLogin(login, password);
        await this.loadAll();
      } finally {
        this.loading = false;
      }
    },
    /** 注册：开启邮箱验证时只是发信，不产生登录态 */
    async register(username: string, email: string, password: string, nickname?: string) {
      this.loading = true;
      try {
        const data = await api.qookixRegister(username, email, password, nickname);
        if (!data.email_verification_required) {
          // 服务端关掉了邮箱验证：等同于注册即登录
          this.status = await api.qookixStatus();
          await this.loadAll();
        }
        return data;
      } finally {
        this.loading = false;
      }
    },
    resendVerification(login: string) {
      return api.qookixResendVerification(login);
    },
    forgotPassword(login: string) {
      return api.qookixForgotPassword(login);
    },
    async logout() {
      this.status = await api.qookixLogout();
      this.friends = [];
      this.incoming = [];
      this.outgoing = [];
      this.invites = [];
      this.blocks = [];
      this.rooms = [];
      this.unread = 0;
    },
    /** 登录后一次性拉齐首屏需要的数据 */
    async loadAll() {
      await Promise.all([
        this.loadFriends(),
        this.loadRequests(),
        this.loadInvites(),
        this.loadRooms(),
      ]);
    },
    async loadFriends() {
      try {
        const r = await api.qookixFriends();
        this.friends = r.friends ?? [];
      } catch (e) {
        if (!asApiError(e)) throw e;
      }
    },
    async loadRequests() {
      try {
        const [inc, out] = await Promise.all([
          api.qookixRequests("incoming"),
          api.qookixRequests("outgoing"),
        ]);
        this.incoming = inc.requests ?? [];
        this.outgoing = out.requests ?? [];
      } catch (e) {
        if (!asApiError(e)) throw e;
      }
    },
    async loadInvites() {
      try {
        const r = await api.qookixInvites();
        this.invites = r.invites ?? [];
      } catch (e) {
        if (!asApiError(e)) throw e;
      }
    },
    async loadBlocks() {
      try {
        const r = await api.qookixBlocks();
        this.blocks = r.blocks ?? [];
      } catch (e) {
        if (!asApiError(e)) throw e;
      }
    },
    /** 批量拉好友绑定的 MC 账号；失败当作「都没绑定」，退回字母头像 */
    async loadProfiles() {
      const uids = this.friends.map((f) => f.user.uid);
      if (!uids.length) {
        this.profiles = {};
        return;
      }
      try {
        const r = await api.qookixUserProfiles(uids);
        const map: Record<number, { mc_name: string; mc_uuid: string; mc_verified: boolean }> = {};
        for (const p of r.profiles ?? []) {
          map[p.uid] = {
            mc_name: p.mc_name,
            mc_uuid: p.mc_uuid,
            mc_verified: p.mc_verified ?? false,
          };
        }
        this.profiles = map;
      } catch {
        /* 头像只是锦上添花，失败不影响好友列表 */
      }
    },
    /** 补查若干 uid 的绑定资料（房间卡片用：brief 里没有 MC 字段） */
    async profilesFor(uids: number[]) {
      const missing = [...new Set(uids)].filter((u) => u > 0 && !this.profiles[u]);
      if (!missing.length) return;
      try {
        const r = await api.qookixUserProfiles(missing);
        const next = { ...this.profiles };
        for (const p of r.profiles ?? []) {
          next[p.uid] = {
            mc_name: p.mc_name,
            mc_uuid: p.mc_uuid,
            mc_verified: p.mc_verified ?? false,
          };
        }
        this.profiles = next;
      } catch {
        /* 同上，失败就继续用字母头像 */
      }
    },
    /** 绑定正版 MC 账号；uuid 省略时用第一个正版登录的账号 */
    async bindMc(uuid?: string) {
      const r = await api.qookixBindMc(uuid);
      await this.refreshStatus();
      return r;
    },
    /** 发起更换邮箱 */
    changeEmail(newEmail: string) {
      return api.qookixChangeEmail(newEmail);
    },
    async unbindMc() {
      const r = await api.qookixUnbindMc();
      await this.refreshStatus();
      return r;
    },
    mcOf(uid: number): { mc_name: string; mc_uuid: string; mc_verified: boolean } | null {
      return this.profiles[uid] ?? null;
    },
    async loadRooms() {
      try {
        const r = await api.qookixRooms("friends");
        this.rooms = r.rooms ?? [];
      } catch (e) {
        if (!asApiError(e)) throw e;
      }
    },
    /** 后台推送的增量：按 id 去重后并入列表（游标回退 2 秒会带来重复） */
    mergeNotifications(n: QookixNotifications) {
      const freshReqs: QookixRequest[] = [];
      const seenReq = new Set(this.incoming.map((r) => r.id));
      for (const r of n.friend_requests ?? []) {
        if (!seenReq.has(r.id)) {
          this.incoming.push(r);
          this.unread += 1;
          freshReqs.push(r);
        }
      }
      const freshInvs: QookixInvite[] = [];
      const seenInv = new Set(this.invites.map((i) => i.id));
      for (const i of n.room_invites ?? []) {
        if (!seenInv.has(i.id)) {
          this.invites.push(i);
          this.unread += 1;
          freshInvs.push(i);
        }
      }
      // 窗口在后台时才弹系统通知（最小化 / 被别的程序挡住都能收到）
      const t = i18n.global.t;
      if (freshReqs.length === 1) {
        const name =
          freshReqs[0].from?.nickname || freshReqs[0].from?.username || "";
        void notifyOs(
          t("qookix.notify.requestTitle"),
          t("qookix.notify.requestOne", { name }),
        );
      } else if (freshReqs.length > 1) {
        void notifyOs(
          t("qookix.notify.requestTitle"),
          t("qookix.notify.requestMany", { count: freshReqs.length }),
        );
      }
      if (freshInvs.length === 1) {
        const inv = freshInvs[0];
        void notifyOs(
          t("qookix.notify.inviteTitle"),
          t("qookix.notify.inviteOne", {
            name: inv.from?.nickname || inv.from?.username || "",
            room: inv.title || t("qookix.rooms.title"),
          }),
        );
      } else if (freshInvs.length > 1) {
        void notifyOs(
          t("qookix.notify.inviteTitle"),
          t("qookix.notify.inviteMany", { count: freshInvs.length }),
        );
      }
    },
    async search(q: string) {
      const r = await api.qookixSearchUsers(q, 10);
      return r.users ?? [];
    },
    async sendRequest(uid: number, message?: string) {
      const r = await api.qookixSendFriendRequest(uid, message);
      await Promise.all([this.loadRequests(), this.loadFriends()]);
      return r;
    },
    async respond(requestId: number, action: "accept" | "reject" | "cancel") {
      await api.qookixRespondRequest(requestId, action);
      await Promise.all([this.loadRequests(), this.loadFriends()]);
    },
    async removeFriend(uid: number) {
      await api.qookixRemoveFriend(uid);
      await this.loadFriends();
    },
    async setBlock(uid: number, blocked: boolean) {
      await api.qookixSetBlock(uid, blocked);
      await Promise.all([this.loadFriends(), this.loadRequests(), this.loadBlocks()]);
    },
    async acceptInvite(inviteId: number): Promise<string> {
      const r = await api.qookixAcceptInvite(inviteId);
      this.invites = this.invites.filter((i) => i.id !== inviteId);
      return r.room?.code ?? "";
    },
    async declineInvite(inviteId: number) {
      await api.qookixDeclineInvite(inviteId);
      this.invites = this.invites.filter((i) => i.id !== inviteId);
    },
    async invite(roomId: string, toUid: number) {
      return api.qookixInvite(roomId, toUid);
    },
    async report(roomId: string, reason: string) {
      return api.qookixReportRoom(roomId, reason);
    },
    async shareRoom(
      code: string,
      visibility: "friends" | "public",
      title?: string,
      playType?: string,
      gameVersion?: string,
      pack?: string,
      modsCount?: number,
      packNet?: string,
    ) {
      const r = await api.qookixShareRoom(
        code,
        visibility,
        title,
        gameVersion,
        undefined,
        playType,
        pack,
        modsCount,
        packNet,
      );
      await this.refreshStatus();
      await this.loadRooms();
      return r;
    },
    async stopShare() {
      this.status = await api.qookixStopShare();
      await this.loadRooms();
    },
    markRead() {
      this.unread = 0;
    },
  },
});
