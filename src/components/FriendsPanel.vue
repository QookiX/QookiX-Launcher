<script setup lang="ts">
/**
 * 好友面板：搜索、好友列表、待处理申请、黑名单。
 *
 * 只做展示与调用，轮询由 Rust 后台推送事件（`qookix://friends`）。
 */
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { NInput, NModal, useDialog, useMessage } from "naive-ui";
import { api } from "../api";
import { qookixErrorMessage, useQookixStore } from "../stores/qookix";
import { useQookixPrefsStore } from "../stores/qookixPrefs";
import type { QookixActivity, QookixRelation, QookixSearchHit } from "../types";
import {
  IconCheck,
  IconEdit,
  IconPlay,
  IconPlus,
  IconRefresh,
  IconSearch,
  IconUser,
  IconUsers,
} from "../components/icons";

const emit = defineEmits<{ (e: "login"): void; (e: "joined"): void }>();

const { t } = useI18n();
const store = useQookixStore();
const prefs = useQookixPrefsStore();
const message = useMessage();
const dialog = useDialog();

const q = ref("");
const hits = ref<QookixSearchHit[]>([]);
const searching = ref(false);
const sending = ref<number | null>(null);
const showBlocks = ref(false);
let debounce: ReturnType<typeof setTimeout> | null = null;

const shareRoomId = computed(() => store.share?.roomId ?? "");

/**
 * 「正在玩什么」：版本 + 加载器 + 实例名 / 服务器。
 * 字段全可选，缺哪个就不显示哪块，不要拼出「· · 」这种空壳。
 */
function activityLabel(a: QookixActivity | undefined): string {
  if (!a) return "";
  const head = [a.mc_version, a.loader].filter(Boolean).join(" ");
  const tail = a.instance
    ? t("qookix.friends.playingInstance", { name: a.instance })
    : a.server
      ? t("qookix.friends.onServer", { addr: a.server })
      : "";
  return [head, tail].filter(Boolean).join(" · ");
}

function relationText(rel: QookixRelation): string {
  switch (rel) {
    case "friend":
      return t("qookix.friends.relationFriend");
    case "requested":
      return t("qookix.friends.requested");
    case "incoming":
      return t("qookix.friends.relationIncoming");
    case "blocked":
    case "blocked_by":
      return t("qookix.friends.relationBlocked");
    default:
      return "";
  }
}

async function runSearch() {
  const term = q.value.trim();
  if (!term) {
    hits.value = [];
    return;
  }
  searching.value = true;
  try {
    hits.value = await store.search(term);
  } catch (e) {
    message.error(qookixErrorMessage(e));
    hits.value = [];
  } finally {
    searching.value = false;
  }
}

watch(q, () => {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(() => void runSearch(), 300);
});

async function addFriend(hit: QookixSearchHit) {
  if (sending.value !== null) return;
  sending.value = hit.user.uid;
  try {
    await store.sendRequest(hit.user.uid);
    message.success(t("qookix.friends.requestSent"));
    hit.relation = "requested";
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    sending.value = null;
  }
}

async function respond(id: number, action: "accept" | "reject" | "cancel") {
  try {
    await store.respond(id, action);
    message.success(t("common.ok"));
  } catch (e) {
    message.error(qookixErrorMessage(e));
  }
}

async function invite(uid: number, name: string) {
  if (!shareRoomId.value) {
    message.warning(t("qookix.rooms.noCode"));
    return;
  }
  try {
    await store.invite(shareRoomId.value, uid);
    message.success(t("qookix.friends.invited") + ` · ${name}`);
  } catch (e) {
    message.error(qookixErrorMessage(e));
  }
}

function removeFriend(uid: number, name: string) {
  dialog.warning({
    title: t("qookix.friends.remove"),
    content: t("qookix.friends.removeConfirm", { name }),
    positiveText: t("common.confirm"),
    negativeText: t("common.cancel"),
    onPositiveClick: async () => {
      try {
        await store.removeFriend(uid);
      } catch (e) {
        message.error(qookixErrorMessage(e));
      }
    },
  });
}

async function block(uid: number, blocked: boolean) {
  try {
    await store.setBlock(uid, blocked);
  } catch (e) {
    message.error(qookixErrorMessage(e));
  }
}

// ---- 本地备注名：只影响自己的显示 ----
const remarkUid = ref<number | null>(null);
const remarkWho = ref("");
const remarkDraft = ref("");
const remarkOpen = computed({
  get: () => remarkUid.value !== null,
  set: (v: boolean) => {
    if (!v) remarkUid.value = null;
  },
});

function openRemark(uid: number, who: string) {
  remarkUid.value = uid;
  remarkWho.value = who;
  remarkDraft.value = prefs.noteFor(uid);
}

function saveRemark(clear = false) {
  if (remarkUid.value === null) return;
  prefs.setNote(remarkUid.value, clear ? "" : remarkDraft.value);
  remarkUid.value = null;
  message.success(clear ? t("qookix.friends.remarkCleared") : t("qookix.friends.remarkSaved"));
}

// ---- 好友正在玩的房间：直接在这行里加入 ----
function friendRoom(uid: number) {
  return store.rooms.find((r) => r.owner?.uid === uid && r.status === "active");
}

const joiningUid = ref<number | null>(null);

async function joinFriendRoom(uid: number) {
  const room = friendRoom(uid);
  if (!room?.code) {
    message.warning(t("qookix.rooms.noCode"));
    return;
  }
  if (joiningUid.value !== null) return;
  joiningUid.value = uid;
  try {
    // 与房间目录一致：陶瓦没跑就先拉起来，否则 join 只会报「未运行」
    const tc = await api.terracottaDetect();
    if (!tc.running) await api.terracottaLaunch();
    await api.terracottaJoinRoom(room.code);
    message.success(t("multiplayer.tc.joined"));
    emit("joined");
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    joiningUid.value = null;
  }
}

// ---- 皮肤头像：好友绑定了 Minecraft 账号就用皮肤头，否则用昵称首字母 ----
const brokenHeads = ref<Set<number>>(new Set());

function brokenHead(uid: number) {
  const next = new Set(brokenHeads.value);
  next.add(uid);
  brokenHeads.value = next;
}

function headUrl(uid: number): string {
  if (brokenHeads.value.has(uid)) return "";
  const uuid = store.mcOf(uid)?.mc_uuid;
  return uuid ? `https://mc-heads.net/avatar/${uuid}/64` : "";
}

onMounted(() => {
  if (store.loggedIn && !store.friends.length) void store.loadFriends();
  if (store.loggedIn) void store.loadProfiles();
});
</script>

<template>
  <div class="fp-root">
    <!-- 未登录：登录引导 -->
    <div v-if="!store.loggedIn" class="fp-empty glass">
      <div class="fp-empty-icon"><IconUsers /></div>
      <p class="fp-empty-title">{{ t("qookix.needLogin") }}</p>
      <p class="fp-empty-desc">{{ t("qookix.intro") }}</p>
      <button class="fp-btn primary" @click="emit('login')">{{ t("qookix.goLogin") }}</button>
    </div>

    <template v-else>
      <!-- 搜索条 -->
      <div class="fp-toolbar glass">
        <IconSearch class="fp-toolbar-icon" />
        <n-input
          v-model:value="q"
          :placeholder="t('qookix.friends.searchPlaceholder')"
          :loading="searching"
          clearable
        />
        <button class="fp-btn ghost" :disabled="searching" @click="runSearch">
          <IconRefresh /> {{ t("common.refresh") }}
        </button>
      </div>

      <div v-if="hits.length" class="fp-hits glass">
        <div v-for="h in hits" :key="h.user.uid" class="fp-hit">
          <span class="fp-avatar">{{ (h.user.nickname || h.user.username).slice(0, 1) }}</span>
          <div class="fp-hit-info">
            <div class="fp-hit-name">{{ h.user.nickname || h.user.username }}</div>
            <div class="fp-hit-meta">@{{ h.user.username }}</div>
          </div>
          <span v-if="relationText(h.relation)" class="fp-tag">{{ relationText(h.relation) }}</span>
          <button
            v-if="
              h.relation === 'none' ||
              h.relation === 'rejected' ||
              h.relation === 'cancelled' ||
              h.relation === 'accepted'
            "
            class="fp-btn primary small"
            :disabled="sending === h.user.uid"
            @click="addFriend(h)"
          >
            <IconPlus /> {{ t("qookix.friends.add") }}
          </button>
        </div>
      </div>
      <p v-else-if="q.trim() && !searching" class="fp-hint">{{ t("qookix.friends.searchEmpty") }}</p>

      <div class="fp-grid">
        <!-- 好友列表 -->
        <section class="fp-card glass">
          <header class="fp-card-head">
            <span class="fp-card-title"><IconUsers /> {{ t("qookix.title") }}</span>
            <span class="fp-count">{{ store.friends.length }}</span>
          </header>
          <p v-if="!store.friends.length" class="fp-empty-line">{{ t("qookix.friends.empty") }}</p>
          <div v-else class="fp-list">
            <div v-for="f in store.friends" :key="f.user.uid" class="fp-row">
              <span class="fp-avatar">
                <img
                  v-if="headUrl(f.user.uid)"
                  class="fp-avatar-img"
                  :src="headUrl(f.user.uid)"
                  alt=""
                  @error="brokenHead(f.user.uid)"
                />
                <template v-else>{{ prefs.displayName(f.user).slice(0, 1) }}</template>
                <i class="fp-dot" :class="{ on: f.user.online }"></i>
              </span>
              <div class="fp-row-info">
                <div class="fp-row-name">
                  {{ prefs.displayName(f.user) }}
                  <span
                    v-if="store.mcOf(f.user.uid)?.mc_verified"
                    class="fp-verified"
                    :title="t('qookix.mcVerified')"
                  >
                    <IconCheck />
                  </span>
                </div>
                <div class="fp-row-meta">@{{ f.user.username }}</div>
                <!-- 正在玩什么（对方心跳上报，可能没有） -->
                <div v-if="f.user.online && activityLabel(f.user.activity)" class="fp-row-activity">
                  {{ activityLabel(f.user.activity) }}
                </div>
                <!-- 好友正在开房：直接在这行点加入，不用先切到房间页签 -->
                <button
                  v-if="friendRoom(f.user.uid)"
                  class="fp-playing"
                  :disabled="joiningUid !== null"
                  @click="joinFriendRoom(f.user.uid)"
                >
                  <IconPlay />
                  <span class="fp-playing-text">
                    {{ t("qookix.friends.playing") }} ·
                    {{ friendRoom(f.user.uid)?.title || t("qookix.rooms.title") }}
                  </span>
                  <b>{{ t("qookix.friends.joinNow") }}</b>
                </button>
              </div>
              <div class="fp-row-ops">
                <button
                  class="fp-mini icon"
                  :title="t('qookix.friends.remark')"
                  @click="openRemark(f.user.uid, prefs.displayName(f.user))"
                >
                  <IconEdit />
                </button>
                <button class="fp-mini" @click="invite(f.user.uid, prefs.displayName(f.user))">
                  {{ t("qookix.friends.invite") }}
                </button>
                <button class="fp-mini" @click="block(f.user.uid, true)">
                  {{ t("qookix.friends.block") }}
                </button>
                <button class="fp-mini danger" @click="removeFriend(f.user.uid, prefs.displayName(f.user))">
                  {{ t("qookix.friends.remove") }}
                </button>
              </div>
            </div>
          </div>
        </section>

        <!-- 待处理申请 -->
        <section class="fp-card glass">
          <header class="fp-card-head">
            <span class="fp-card-title"><IconCheck /> {{ t("qookix.friends.pendingTitle") }}</span>
            <span class="fp-count">{{ store.incoming.length }}</span>
          </header>
          <p v-if="!store.incoming.length" class="fp-empty-line">
            {{ t("qookix.friends.pendingEmpty") }}
          </p>
          <div v-else class="fp-list">
            <div v-for="r in store.incoming" :key="r.id" class="fp-row">
              <span class="fp-avatar">{{ (r.from?.nickname || r.from?.username || "?").slice(0, 1) }}</span>
              <div class="fp-row-info">
                <div class="fp-row-name">{{ r.from?.nickname || r.from?.username }}</div>
                <div class="fp-row-meta">{{ r.message || t("qookix.friends.incoming") }}</div>
              </div>
              <div class="fp-row-ops">
                <button class="fp-mini ok" @click="respond(r.id, 'accept')">
                  {{ t("qookix.friends.accept") }}
                </button>
                <button class="fp-mini" @click="respond(r.id, 'reject')">
                  {{ t("qookix.friends.reject") }}
                </button>
              </div>
            </div>
          </div>

          <template v-if="store.outgoing.length">
            <div class="fp-divider"></div>
            <div v-for="r in store.outgoing" :key="r.id" class="fp-row">
              <span class="fp-avatar">{{ (r.to?.nickname || r.to?.username || "?").slice(0, 1) }}</span>
              <div class="fp-row-info">
                <div class="fp-row-name">{{ r.to?.nickname || r.to?.username }}</div>
                <div class="fp-row-meta">{{ t("qookix.friends.outgoing") }}</div>
              </div>
              <div class="fp-row-ops">
                <button class="fp-mini" @click="respond(r.id, 'cancel')">
                  {{ t("qookix.friends.cancel") }}
                </button>
              </div>
            </div>
          </template>
        </section>
      </div>

      <!-- 黑名单 -->
      <section class="fp-card glass">
        <button class="fp-collapse" @click="showBlocks = !showBlocks">
          <IconUser />
          <span>{{ t("qookix.friends.blockTitle") }}</span>
          <span class="fp-count">{{ store.blocks.length }}</span>
        </button>
        <template v-if="showBlocks">
          <p v-if="!store.blocks.length" class="fp-empty-line">
            {{ t("qookix.friends.blockEmpty") }}
          </p>
          <div v-for="b in store.blocks" :key="b.user.uid" class="fp-row">
            <span class="fp-avatar">{{ (b.user.nickname || b.user.username).slice(0, 1) }}</span>
            <div class="fp-row-info">
              <div class="fp-row-name">{{ b.user.nickname || b.user.username }}</div>
              <div class="fp-row-meta">@{{ b.user.username }}</div>
            </div>
            <button class="fp-mini" @click="block(b.user.uid, false)">
              {{ t("qookix.friends.unblock") }}
            </button>
          </div>
        </template>
      </section>
    </template>

    <!-- 好友备注名（只存在本机） -->
    <n-modal
      v-model:show="remarkOpen"
      preset="card"
      :title="t('qookix.friends.remarkTitle')"
      style="width: 420px; max-width: 92vw"
      :bordered="false"
    >
      <div class="fp-remark">
        <p class="fp-remark-hint">{{ t("qookix.friends.remarkHint", { name: remarkWho }) }}</p>
        <n-input
          v-model:value="remarkDraft"
          :placeholder="t('qookix.friends.remarkPlaceholder')"
          maxlength="24"
          @keyup.enter="saveRemark(false)"
        />
        <div class="fp-remark-ops">
          <button class="fp-mini" @click="saveRemark(true)">
            {{ t("qookix.friends.remarkClear") }}
          </button>
          <span class="fp-remark-spacer"></span>
          <button class="fp-mini" @click="remarkOpen = false">{{ t("common.cancel") }}</button>
          <button class="fp-btn primary small" @click="saveRemark(false)">
            {{ t("common.confirm") }}
          </button>
        </div>
      </div>
    </n-modal>
  </div>
</template>

<style scoped>
.fp-root {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding-bottom: 24px;
}

/* ---- 未登录 ---- */
.fp-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 40px 24px;
  text-align: center;
}
.fp-empty-icon {
  width: 40px;
  height: 40px;
  opacity: 0.55;
}
.fp-empty-title {
  margin: 0;
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-1);
}
.fp-empty-desc {
  margin: 0;
  max-width: 420px;
  font-size: var(--fs-xs);
  line-height: 1.7;
  color: var(--text-3);
}

/* ---- 搜索条 ---- */
.fp-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
}
.fp-toolbar-icon {
  width: 16px;
  height: 16px;
  flex: none;
  color: var(--text-3);
}
/* n-input 默认 width:100%，在 flex 行里必须让它可收缩，否则会把按钮挤出去 */
.fp-toolbar :deep(.n-input) {
  flex: 1;
  min-width: 0;
}
.fp-hits {
  display: flex;
  flex-direction: column;
  gap: 2px;
  max-height: 280px;
  overflow: auto;
  padding: 8px;
}
.fp-hit {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: var(--r-md);
  transition: background var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.fp-hit:hover {
  background: var(--w-04, rgba(255, 255, 255, 0.04));
}
.fp-hit-info {
  min-width: 0;
  flex: 1;
}
.fp-hit-name {
  font-size: var(--fs-sm);
  color: var(--text-1);
}
.fp-hit-meta {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* ---- 两张主卡片 ---- */
.fp-grid {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  gap: 16px;
}
@media (max-width: 1000px) {
  .fp-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
.fp-card {
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 18px 20px;
  min-width: 0;
}
.fp-card-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.fp-card-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}
.fp-card-title svg {
  width: 15px;
  height: 15px;
  opacity: 0.75;
}
.fp-count {
  margin-left: auto;
  min-width: 18px;
  padding: 1px 7px;
  border-radius: var(--r-pill);
  background: var(--w-06, rgba(255, 255, 255, 0.06));
  font-size: var(--fs-2xs);
  line-height: 16px;
  text-align: center;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
.fp-empty-line {
  margin: 0;
  padding: 16px 0;
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.fp-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

/* ---- 行 ---- */
.fp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 8px;
  border-radius: var(--r-md);
  transition: background var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.fp-row:hover {
  background: var(--w-04, rgba(255, 255, 255, 0.04));
}
.fp-avatar {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 30px;
  flex: none;
  border-radius: 50%;
  background: var(--accent-08, rgba(232, 154, 75, 0.12));
  color: var(--accent);
  font-size: var(--fs-xs);
  font-weight: 600;
}
.fp-avatar-img {
  width: 100%;
  height: 100%;
  border-radius: 50%;
  object-fit: cover;
}

/* 正版已验证：名字后面一枚小对勾 */
.fp-verified {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 13px;
  height: 13px;
  margin-left: 4px;
  vertical-align: -1px;
  border-radius: 50%;
  background: rgba(78, 201, 160, 0.16);
  color: var(--success, #4ec9a0);
}
.fp-verified svg {
  width: 9px;
  height: 9px;
}
.fp-dot {
  position: absolute;
  right: -1px;
  bottom: -1px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  border: 2px solid var(--bg-1, #0b0d12);
  background: var(--text-3);
}
.fp-dot.on {
  background: var(--success, #4ec9a0);
}
.fp-row-info {
  min-width: 0;
  flex: 1;
}
.fp-row-name {
  font-size: var(--fs-sm);
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.fp-row-meta {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* 「1.20.1 fabric · 在玩 xx」：比 @用户名 再弱一档，别抢主信息 */
.fp-row-activity {
  margin-top: 1px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  opacity: 0.9;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.fp-row-ops {
  display: flex;
  gap: 6px;
  flex: none;
}

/* ---- 小按钮 ---- */
.fp-tag {
  flex: none;
  font-size: var(--fs-2xs);
  padding: 2px 8px;
  border-radius: var(--r-pill);
  border: 1px solid var(--border);
  color: var(--text-3);
  white-space: nowrap;
}
.fp-mini {
  border: 1px solid var(--border);
  background: transparent;
  border-radius: var(--r-sm);
  padding: 4px 9px;
  font-size: var(--fs-2xs);
  color: var(--text-2);
  white-space: nowrap;
  cursor: pointer;
  transition: border-color var(--dur-sm, 0.15s) var(--ease-out, ease),
    color var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.fp-mini:hover {
  border-color: var(--accent-35, rgba(232, 154, 75, 0.35));
  color: var(--text-1);
}
.fp-mini.ok {
  color: var(--success, #4ec9a0);
  border-color: rgba(78, 201, 160, 0.35);
}
.fp-mini.danger:hover {
  color: var(--danger, #e5534b);
  border-color: rgba(229, 83, 75, 0.4);
}
.fp-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  height: 32px;
  padding: 0 14px;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: transparent;
  font-size: var(--fs-xs);
  color: var(--text-1);
  white-space: nowrap;
  flex: none;
  cursor: pointer;
  transition: transform var(--dur-sm, 0.15s) var(--ease-out, ease),
    filter var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.fp-btn svg {
  width: 14px;
  height: 14px;
}
.fp-btn.primary {
  border: none;
  background: var(--accent);
  /* 跟随全局主题对比色：写死深色墨字在红色/紫色主题下会变成黑字 */
  color: var(--accent-contrast, #fff);
  font-weight: 600;
}
.fp-btn.small {
  height: 26px;
  padding: 0 10px;
  font-size: var(--fs-2xs);
}
.fp-btn:hover:not(:disabled) {
  filter: brightness(1.05);
}
.fp-btn:active:not(:disabled) {
  transform: scale(0.97);
}
.fp-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.fp-hint {
  margin: 0;
  padding: 0 4px;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.fp-divider {
  height: 1px;
  margin: 4px 0;
  background: var(--border);
  opacity: 0.6;
}
.fp-collapse {
  display: flex;
  align-items: center;
  gap: 8px;
  border: none;
  background: none;
  padding: 0;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
  cursor: pointer;
}
.fp-collapse svg {
  width: 15px;
  height: 15px;
  opacity: 0.75;
}

/* ---- 好友正在开房：行内加入 ---- */
.fp-playing {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  max-width: 100%;
  margin-top: 4px;
  padding: 3px 8px;
  border: 1px solid rgba(78, 201, 160, 0.35);
  border-radius: var(--r-sm);
  background: rgba(78, 201, 160, 0.1);
  color: var(--success, #4ec9a0);
  font-size: var(--fs-2xs);
  font-family: inherit;
  cursor: pointer;
  transition: background var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.fp-playing:hover:not(:disabled) {
  background: rgba(78, 201, 160, 0.18);
}
.fp-playing:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}
.fp-playing svg {
  width: 11px;
  height: 11px;
  flex: none;
}
.fp-playing-text {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.fp-playing b {
  flex: none;
  font-weight: 600;
}

.fp-mini.icon {
  display: inline-flex;
  align-items: center;
  padding: 4px 7px;
}
.fp-mini.icon svg {
  width: 12px;
  height: 12px;
}

/* ---- 备注弹窗 ---- */
.fp-remark {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.fp-remark-hint {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.fp-remark-ops {
  display: flex;
  align-items: center;
  gap: 6px;
}
.fp-remark-spacer {
  flex: 1;
}
</style>
