<script setup lang="ts">
/**
 * 房间目录：好友房间 / 公开大厅 / 联机邀请。
 *
 * 加入动作直接调本地陶瓦（`terracotta_join_room`），后端只提供房间码。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { NInput, NModal, NSelect, useMessage } from "naive-ui";
import { api } from "../api";
import { qookixErrorMessage, useQookixStore } from "../stores/qookix";
import { useInstancesStore } from "../stores/instances";
import { PLAY_TYPES, agoText, playTypeLabel } from "../utils/qookix";
import type { PackCompareOut, PackFile, QookixRoom } from "../types";
import { fmtBytes } from "../utils/format";
import { useSlidingIndicator } from "../composables/useSlidingIndicator";
import { IconCheck, IconCopy, IconExternal, IconRefresh, IconUsers } from "../components/icons";

const emit = defineEmits<{ (e: "login"): void; (e: "joined"): void }>();

const { t } = useI18n();
const store = useQookixStore();
const instances = useInstancesStore();
const message = useMessage();

const sub = ref<"friends" | "public">("friends");
// 好友房间 / 公开大厅切换的滑动高亮
const subSegRef = ref<HTMLElement | null>(null);
const { indicatorStyle: subSegStyle, refresh: refreshSubSeg } = useSlidingIndicator(
  subSegRef,
  () => Array.from(subSegRef.value?.querySelectorAll<HTMLElement>("button") ?? []),
  () => (sub.value === "friends" ? 0 : 1),
);
watch(sub, () => nextTick(() => refreshSubSeg()));
const publicRooms = ref<QookixRoom[]>([]);
const keyword = ref("");
const loading = ref(false);
const joiningId = ref<string | null>(null);
const now = ref(Date.now() / 1000);
let ticker: ReturnType<typeof setInterval> | null = null;

// 公开大厅筛选条件（好友房间不需要，那些是熟人房）
const filterPlayType = ref<string | null>(null);
const filterVersion = ref("");
const filterPlayers = ref(0);
const playTypeOptions = computed(() =>
  PLAY_TYPES.map((p) => ({ label: t(p.labelKey), value: p.value })),
);
const playerOptions = computed(() => [
  { label: t("qookix.rooms.playersAny"), value: 0 },
  { label: t("qookix.rooms.players2"), value: 2 },
  { label: t("qookix.rooms.players3"), value: 3 },
  { label: t("qookix.rooms.players5"), value: 5 },
]);
let filterTimer: ReturnType<typeof setTimeout> | null = null;

// 房间卡片上的房主皮肤头：`owner` 是 brief，不带 MC 字段，需要额外查 profiles
const brokenHeads = ref<Set<number>>(new Set());
function brokenHead(uid: number | undefined) {
  if (!uid) return;
  const next = new Set(brokenHeads.value);
  next.add(uid);
  brokenHeads.value = next;
}
function headFor(uid: number | undefined): string {
  if (!uid || brokenHeads.value.has(uid)) return "";
  const uuid = store.mcOf(uid)?.mc_uuid;
  return uuid ? `https://mc-heads.net/avatar/${uuid}/32` : "";
}
function verifiedFor(uid: number | undefined): boolean {
  return !!uid && !!store.mcOf(uid)?.mc_verified;
}

function reloadWithFilters(delay = 0) {
  if (filterTimer) clearTimeout(filterTimer);
  filterTimer = setTimeout(() => {
    if (sub.value === "public") void load();
  }, delay);
}

watch(filterPlayType, () => reloadWithFilters());
watch(filterPlayers, () => reloadWithFilters());
watch(filterVersion, () => reloadWithFilters(350));

const reportOpen = ref(false);
const reportRoomId = ref("");
const reportReason = ref("");
const reporting = ref(false);

const rooms = computed(() => (sub.value === "friends" ? store.rooms : publicRooms.value));

function statusLabel(r: QookixRoom): string {
  switch (r.status) {
    case "active":
      return t("qookix.rooms.active");
    case "expired":
      return t("qookix.rooms.expired");
    case "removed":
      return t("qookix.rooms.removed");
    default:
      return t("qookix.rooms.closed");
  }
}

function minutesLeft(expiresAt: number): number {
  return Math.max(0, Math.ceil((expiresAt - now.value) / 60));
}

async function load() {
  if (!store.loggedIn) return;
  loading.value = true;
  try {
    if (sub.value === "friends") {
      await store.loadRooms();
    } else {
      const r = await api.qookixPublicRooms(keyword.value.trim() || undefined, 1, 20, {
        version: filterVersion.value.trim() || undefined,
        playType: filterPlayType.value ?? undefined,
        minPlayers: filterPlayers.value || undefined,
      });
      publicRooms.value = r.rooms ?? [];
    }
    // 房主 brief 不带 MC 字段，补查一次；没绑定的人不会返回，静默跳过
    const shown = sub.value === "friends" ? store.rooms : publicRooms.value;
    void store.profilesFor(shown.map((r) => r.owner?.uid ?? 0));
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    loading.value = false;
  }
}

async function switchSub(next: "friends" | "public") {
  sub.value = next;
  await load();
}

// ---- 环境预检（Phase 2a）----
// 房主分享了模组清单时，加入前先和本地实例比一遍，
// 提前暴露「版本不同 / 少几个 mod」这类必然导致进房崩溃的问题。
const packCheck = ref<PackCompareOut | null>(null);
const packCheckRoom = ref<QookixRoom | null>(null);
const packCheckInstance = ref<string>("");
const packChecking = ref(false);
const packCheckOpen = computed(() => packCheck.value !== null);
const packLocalVersion = computed(
  () => instances.instances.find((i) => i.id === packCheckInstance.value)?.mc_version ?? "",
);
const packLocalLoader = computed(
  () => instances.instances.find((i) => i.id === packCheckInstance.value)?.loader ?? "",
);
const packInstanceName = computed(
  () => instances.instances.find((i) => i.id === packCheckInstance.value)?.name ?? "",
);

function defaultInstanceId(): string {
  const list = instances.instances.filter((i) => i.installed);
  if (!list.length) return "";
  const sorted = [...list].sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0));
  return sorted[0].id;
}

/** 点「加入」：有清单就先比对，有差异就让用户确认 */
async function onJoinClick(room: QookixRoom) {
  if (!room.code) {
    message.warning(t("qookix.rooms.noCode"));
    return;
  }
  if ((room.mods_count ?? 0) > 0) {
    const instId = defaultInstanceId();
    if (instId) {
      packChecking.value = true;
      try {
        const cmp = await api.qookixComparePack(room.id, instId);
        if (cmp.hasPack && !cmp.ok) {
          packCheck.value = cmp;
          packCheckRoom.value = room;
          packCheckInstance.value = instId;
          return;
        }
      } catch (e) {
        // 比对失败不阻塞加入：只是少一条提示，不能因此进不去
        message.warning(String(e));
      } finally {
        packChecking.value = false;
      }
    }
  }
  await join(room.code);
}

function closePackCheck() {
  packCheck.value = null;
  packCheckRoom.value = null;
}

async function joinAnyway() {
  const code = packCheckRoom.value?.code;
  closePackCheck();
  if (code) await join(code);
}

// ---- 一键补齐（Phase 2b 快路径）----
// 只装「有来源」的 mod：Modrinth / CurseForge 且清单里带了 project_id + version_id。
// 手动拖进来的 jar 没有来源，客户端无从下载，只能如实告诉用户去要文件。
function canInstall(m: PackFile): boolean {
  const s = m.source;
  return !!s && (s.provider === "modrinth" || s.provider === "curseforge");
}
const installableMissing = computed(() =>
  (packCheck.value?.diff?.missing ?? []).filter(canInstall),
);
const manualMissing = computed(() => (packCheck.value?.diff?.missing ?? []).filter((m) => !canInstall(m)));
const installBusy = ref(false);
const installDone = ref(0);

// ---- 从房主 P2P 拉文件 ----
const pulling = ref(false);

async function pullFromHost() {
  const room = packCheckRoom.value;
  const instId = packCheckInstance.value;
  if (!room || !instId || pulling.value) return;
  pulling.value = true;
  try {
    const r = await api.qookixPackPull(room.id, instId);
    await instances.load();
    try {
      packCheck.value = await api.qookixComparePack(room.id, instId);
    } catch {
      /* 重比对失败不影响已下载的文件 */
    }
    if (r.failed > 0) {
      message.warning(t("qookix.pack.pullPartial", { ok: r.downloaded, fail: r.failed }));
    } else if (r.downloaded > 0) {
      message.success(t("qookix.pack.pullOk", { n: r.downloaded }));
    }
    if (packCheck.value?.ok) {
      closePackCheck();
      message.success(t("qookix.pack.same"));
    }
  } catch (e) {
    message.error(String(e));
  } finally {
    pulling.value = false;
  }
}

async function installMissing() {
  const list = installableMissing.value;
  const instId = packCheckInstance.value;
  if (!list.length || !instId) return;
  installBusy.value = true;
  installDone.value = 0;
  let failed = 0;
  try {
    // 串行装：并发下载同一个实例的 mods 目录容易互相覆盖，慢一点但可控
    for (const m of list) {
      try {
        const src = m.source!;
        await api.installContent(instId, src.provider, src.project_id, src.version_id, "mod");
        installDone.value += 1;
      } catch (e) {
        failed += 1;
        console.warn("[pack] 安装失败", m.mod_id, e);
      }
    }
    await instances.load();
    // 装完重跑比对：让用户直接看到「还差什么」，而不是自己猜
    const room = packCheckRoom.value;
    if (room) {
      try {
        const cmp = await api.qookixComparePack(room.id, instId);
        if (cmp.ok) {
          closePackCheck();
          message.success(t("qookix.pack.same"));
        } else {
          packCheck.value = cmp;
        }
      } catch {
        /* 重新比对失败不影响已安装的结果 */
      }
    }
    if (failed > 0) {
      message.warning(t("qookix.pack.installPartial", { ok: installDone.value, fail: failed }));
    } else if (installDone.value > 0) {
      message.success(t("qookix.pack.installOk", { n: installDone.value }));
    }
  } finally {
    installBusy.value = false;
  }
}

async function join(code: string | undefined) {
  if (!code) {
    message.warning(t("qookix.rooms.noCode"));
    return;
  }
  // 防连点：连点会并发发多次加入请求，弹一串「陶瓦联机未运行」
  if (joiningId.value) return;
  joiningId.value = code;
  try {
    // 陶瓦没在运行就先拉起来（launch 会等到本地端口就绪），
    // 否则 join 只会拿到「陶瓦联机未运行」。
    const tc = await api.terracottaDetect();
    if (!tc.running) await api.terracottaLaunch();
    await api.terracottaJoinRoom(code);
    message.success(t("multiplayer.tc.joined"));
    emit("joined");
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    joiningId.value = null;
  }
}

async function acceptInvite(id: number) {
  try {
    const code = await store.acceptInvite(id);
    if (!code) {
      message.warning(t("qookix.rooms.noCode"));
      return;
    }
    await join(code);
  } catch (e) {
    message.error(qookixErrorMessage(e));
  }
}

async function declineInvite(id: number) {
  try {
    await store.declineInvite(id);
  } catch (e) {
    message.error(qookixErrorMessage(e));
  }
}

function openReport(roomId: string) {
  reportRoomId.value = roomId;
  reportReason.value = "";
  reportOpen.value = true;
}

async function submitReport() {
  const reason = reportReason.value.trim();
  if (reason.length < 3) return;
  reporting.value = true;
  try {
    await store.report(reportRoomId.value, reason);
    message.success(t("qookix.rooms.reportSent"));
    reportOpen.value = false;
  } catch (e) {
    message.error(qookixErrorMessage(e));
  } finally {
    reporting.value = false;
  }
}

async function copyCode(code: string) {
  try {
    await navigator.clipboard.writeText(code);
    message.success(t("multiplayer.tc.copied"));
  } catch {
    // 剪贴板不可用时不提示
  }
}

onMounted(() => {
  if (store.loggedIn) {
    void store.loadInvites();
    void load();
  }
  ticker = setInterval(() => (now.value = Date.now() / 1000), 30000);
});

// 在设置页登录后回到联机页，目录要自己补一次数据
watch(
  () => store.loggedIn,
  (v) => {
    if (v) {
      void store.loadInvites();
      void load();
    } else {
      publicRooms.value = [];
    }
  },
);

onUnmounted(() => {
  if (ticker) clearInterval(ticker);
});
</script>

<template>
  <div class="rd-root">
    <div v-if="!store.loggedIn" class="rd-login glass">
      <p class="rd-login-text">{{ t("qookix.needLogin") }}</p>
      <button class="rd-btn primary" @click="emit('login')">{{ t("qookix.goLogin") }}</button>
    </div>

    <template v-else>
      <!-- 联机邀请 -->
      <section v-if="store.invites.length" class="rd-invites glass">
        <header class="rd-head">
          <span class="rd-title"><IconExternal /> {{ t("qookix.invites.title") }}</span>
          <span class="rd-count">{{ store.invites.length }}</span>
        </header>
        <div class="rd-invite-list">
          <div
            v-for="inv in store.invites"
            :key="inv.id"
            class="rd-invite"
            :class="{ dead: inv.room_status !== 'active' || minutesLeft(inv.expires_at) <= 0 }"
          >
            <div class="rd-invite-main">
              <div class="rd-invite-title">{{ inv.title || t("qookix.rooms.title") }}</div>
              <div class="rd-invite-meta">
                {{ t("qookix.invites.from", { name: inv.from?.nickname || inv.from?.username || "?" }) }}
                · {{ t("qookix.invites.left", { min: minutesLeft(inv.expires_at) }) }}
              </div>
            </div>
            <div class="rd-invite-ops">
              <button
                class="rd-btn primary small"
                :disabled="inv.room_status !== 'active' || minutesLeft(inv.expires_at) <= 0"
                @click="acceptInvite(inv.id)"
              >
                {{ t("qookix.invites.join") }}
              </button>
              <button class="rd-btn ghost small" @click="declineInvite(inv.id)">
                {{ t("qookix.invites.decline") }}
              </button>
            </div>
          </div>
        </div>
      </section>

      <!-- 目录 -->
      <section class="rd-dir glass">
        <header class="rd-head">
          <div ref="subSegRef" class="rd-subtabs qx-seg qx-seg--sm">
            <div class="qx-seg__indicator" :style="subSegStyle"></div>
            <button :class="{ active: sub === 'friends' }" @click="switchSub('friends')">
              {{ t("qookix.rooms.friends") }}
            </button>
            <button :class="{ active: sub === 'public' }" @click="switchSub('public')">
              {{ t("qookix.rooms.public") }}
            </button>
          </div>
          <div class="rd-head-ops">
            <n-input
              v-if="sub === 'public'"
              v-model:value="keyword"
              size="small"
              :placeholder="t('qookix.rooms.searchPlaceholder')"
              @keyup.enter="load"
            />
            <button class="rd-btn ghost small" :disabled="loading" @click="load">
              <IconRefresh /> {{ t("common.refresh") }}
            </button>
          </div>
        </header>

        <!-- 公开大厅筛选：玩法 / 版本 / 人数 -->
        <div v-if="sub === 'public'" class="rd-filters">
          <n-select
            v-model:value="filterPlayType"
            class="rd-filter"
            size="small"
            clearable
            :options="playTypeOptions"
            :placeholder="t('qookix.rooms.filterPlayType')"
          />
          <n-input
            v-model:value="filterVersion"
            class="rd-filter"
            size="small"
            :placeholder="t('qookix.rooms.versionPlaceholder')"
          />
          <n-select
            v-model:value="filterPlayers"
            class="rd-filter narrow"
            size="small"
            :options="playerOptions"
          />
        </div>

        <p class="rd-subhint">
          {{ sub === "friends" ? t("qookix.rooms.friendsHint") : t("qookix.rooms.publicHint") }}
        </p>

        <p v-if="!rooms.length && !loading" class="rd-empty">
          {{ sub === "friends" ? t("qookix.rooms.empty") : t("qookix.rooms.publicEmpty") }}
        </p>

        <div class="rd-grid">
          <article
            v-for="room in rooms"
            :key="room.id"
            class="rd-room"
            :class="{ dead: room.status !== 'active' }"
          >
            <div class="rd-room-top">
              <span class="rd-room-title">{{ room.title || t("qookix.rooms.title") }}</span>
              <span class="rd-status" :class="room.status">{{ statusLabel(room) }}</span>
            </div>
            <div class="rd-tags">
              <span v-if="(room.mods_count ?? 0) > 0" class="rd-tag">
                {{ t("qookix.pack.roomHas", { n: room.mods_count }) }}
              </span>
              <span v-if="room.play_type" class="rd-tag mode">
                {{ playTypeLabel(room.play_type, (k) => t(k)) }}
              </span>
              <span v-if="room.game_version" class="rd-tag">{{ room.game_version }}</span>
              <span v-if="room.modpack" class="rd-tag">{{ room.modpack }}</span>
              <span class="rd-tag"><IconUsers /> {{ t("qookix.rooms.members", { count: room.member_count }) }}</span>
              <!-- 活跃度：心跳距今多久。房间可能已经没人了但还没过期 -->
              <span class="rd-tag" :class="{ stale: room.status !== 'active' }">
                {{ t("qookix.rooms.activeAgo", { when: agoText(room.last_heartbeat, now, t) }) }}
              </span>
            </div>
            <div class="rd-room-foot">
              <span class="rd-owner">
                <img
                  v-if="headFor(room.owner?.uid)"
                  class="rd-owner-av"
                  :src="headFor(room.owner?.uid)"
                  alt=""
                  @error="brokenHead(room.owner?.uid)"
                />
                {{ room.owner?.nickname || room.owner?.username }}
                <span v-if="room.owner?.activity?.instance" class="rd-owner-inst">
                  · {{ room.owner.activity.instance }}
                </span>
                <span
                  v-if="verifiedFor(room.owner?.uid)"
                  class="rd-verified"
                  :title="t('qookix.mcVerified')"
                >
                  <IconCheck />
                </span>
              </span>
              <div class="rd-room-ops">
                <button
                  v-if="room.code"
                  class="rd-icon-btn"
                  :title="t('multiplayer.tc.copyCode')"
                  @click="copyCode(room.code)"
                >
                  <IconCopy />
                </button>
                <button class="rd-btn ghost small" @click="openReport(room.id)">
                  {{ t("qookix.rooms.report") }}
                </button>
                <button
                  class="rd-btn primary small"
                  :disabled="room.status !== 'active' || !room.code || joiningId !== null"
                  @click="onJoinClick(room)"
                >
                  {{ joiningId === room.code ? t("qookix.rooms.joining") : t("qookix.rooms.join") }}
                </button>
              </div>
            </div>
          </article>
        </div>
      </section>
    </template>

    <!-- 环境预检：房主带了模组清单，加入前先比对一遍 -->
    <n-modal
      v-model:show="packCheckOpen"
      preset="card"
      :title="t('qookix.pack.title')"
      style="width: 560px; max-width: 92vw"
      :bordered="false"
    >
      <div v-if="packCheck" class="pack-body">
        <p class="pack-head">
          {{ t("qookix.pack.comparedWith", { name: packInstanceName }) }}
          · {{ t("qookix.pack.roomHas", { n: packCheck.remoteMods }) }}
        </p>
        <ul class="pack-list">
          <li v-if="packCheck.diff?.mc_version_mismatch" class="bad">
            {{
              t("qookix.pack.mcMismatch", {
                remote: packCheck.remoteMcVersion,
                local: packLocalVersion,
              })
            }}
          </li>
          <li v-if="packCheck.diff?.loader_mismatch" class="bad">
            {{
              t("qookix.pack.loaderMismatch", {
                remote: packCheck.remoteLoader,
                local: packLocalLoader,
              })
            }}
          </li>
          <li v-if="(packCheck.diff?.missing.length ?? 0) > 0" class="bad">
            {{ t("qookix.pack.missing", { n: packCheck.diff?.missing.length ?? 0 }) }}
            <span class="pack-items">
              {{ packCheck.diff?.missing.map((m) => m.name || m.mod_id).join("、") }}
            </span>
            <!-- 有来源的那部分可以一键装，手动来源的只能找房主要文件 -->
            <span v-if="installableMissing.length" class="pack-fix">
              <button class="rd-btn primary small" :disabled="installBusy" @click="installMissing">
                {{
                  installBusy
                    ? t("qookix.pack.installing", { n: installDone })
                    : t("qookix.pack.installN", { n: installableMissing.length })
                }}
              </button>
            </span>
            <span v-if="manualMissing.length" class="pack-items manual">
              {{ t("qookix.pack.manualHint", { n: manualMissing.length }) }}
              {{ manualMissing.map((m) => m.name || m.mod_id).join("、") }}
            </span>
          </li>
          <li v-if="packCheck.pullBytes > 0" class="hint-line">
            {{ t("qookix.pack.pullHint", { size: fmtBytes(packCheck.pullBytes) }) }}
            <span class="pack-fix">
              <button class="rd-btn primary small" :disabled="pulling" @click="pullFromHost">
                {{ pulling ? t("qookix.pack.pulling") : t("qookix.pack.pullDo") }}
              </button>
            </span>
          </li>
          <li v-if="(packCheck.diff?.version_mismatch.length ?? 0) > 0" class="warn">
            {{ t("qookix.pack.versionMismatch", { n: packCheck.diff?.version_mismatch.length ?? 0 }) }}
            <span class="pack-items">
              {{
                packCheck.diff?.version_mismatch
                  .map((m) => `${m.name || m.mod_id} ${m.version}`)
                  .join("、")
              }}
            </span>
          </li>
          <li v-if="(packCheck.diff?.extra.length ?? 0) > 0" class="hint-line">
            {{ t("qookix.pack.extra", { n: packCheck.diff?.extra.length ?? 0 }) }}
          </li>
        </ul>
        <div class="rd-report-ops">
          <button class="rd-btn ghost" @click="closePackCheck">{{ t("qookix.pack.cancelJoin") }}</button>
          <button class="rd-btn primary" @click="joinAnyway">{{ t("qookix.pack.joinAnyway") }}</button>
        </div>
      </div>
    </n-modal>

    <n-modal
      v-model:show="reportOpen"
      preset="card"
      :title="t('qookix.rooms.report')"
      style="width: 420px; max-width: 92vw"
      :bordered="false"
    >
      <div class="rd-report">
        <n-input
          v-model:value="reportReason"
          type="textarea"
          :rows="3"
          :placeholder="t('qookix.rooms.reportReason')"
        />
        <div class="rd-report-ops">
          <button class="rd-btn ghost" @click="reportOpen = false">{{ t("common.cancel") }}</button>
          <button
            class="rd-btn primary"
            :disabled="reportReason.trim().length < 3 || reporting"
            @click="submitReport"
          >
            {{ t("common.confirm") }}
          </button>
        </div>
      </div>
    </n-modal>
  </div>
</template>

<style scoped>
.rd-root {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.rd-login {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 16px;
}
.rd-login-text {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.rd-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}
.rd-title {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-1);
}
.rd-title svg {
  width: 15px;
  height: 15px;
}
.rd-count {
  margin-left: 6px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.rd-invites,
.rd-dir {
  padding: 18px 20px;
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}
/* n-input 默认 width:100%，不固定宽度会把后面的刷新按钮挤出去 */
.rd-head-ops :deep(.n-input) {
  width: 200px;
  flex: none;
}
.rd-invite-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.rd-invite {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 12px;
  border-radius: var(--r-md);
  border-left: 3px solid var(--accent);
  background: var(--panel);
  transition: opacity var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.rd-invite.dead {
  opacity: 0.5;
}
.rd-invite-title {
  font-size: var(--fs-sm);
  color: var(--text-1);
}
.rd-invite-meta {
  font-size: var(--fs-2xs);
  color: var(--text-3);
}
.rd-invite-ops {
  display: flex;
  gap: 6px;
}
/* 好友房间 / 公开大厅切换：外观与滑块走全局 .qx-seg--sm */
.rd-head-ops {
  display: flex;
  align-items: center;
  gap: 8px;
}
.rd-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 10px;
}
.rd-room {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px;
  border-radius: var(--r-md);
  border: 1px solid var(--border);
  background: var(--panel);
  transition: transform var(--dur-sm, 0.15s) var(--ease-out, ease),
    border-color var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.rd-room:hover {
  transform: translateY(-2px);
  border-color: var(--accent-35, rgba(232, 154, 75, 0.35));
}
.rd-room.dead {
  opacity: 0.55;
}
.rd-room-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.rd-room-title {
  font-size: var(--fs-sm);
  color: var(--text-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.rd-status {
  flex: none;
  font-size: var(--fs-2xs);
  padding: 2px 7px;
  border-radius: var(--r-pill);
  border: 1px solid var(--border);
  color: var(--text-3);
}
.rd-status.active {
  color: var(--success, #4ec9a0);
  border-color: rgba(78, 201, 160, 0.4);
}
.rd-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
}
.rd-tag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  font-size: var(--fs-2xs);
  padding: 2px 7px;
  border-radius: var(--r-xs);
  background: var(--bg-2, rgba(255, 255, 255, 0.04));
  color: var(--text-3);
}
.rd-tag svg {
  width: 11px;
  height: 11px;
}
.rd-room-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.rd-owner {
  font-size: var(--fs-2xs);
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* 房主：小头像 + 名字 + 已验证对勾 */
.rd-owner {
  display: inline-flex;
  align-items: center;
  gap: 5px;
}
.rd-owner-av {
  width: 14px;
  height: 14px;
  flex: none;
  border-radius: var(--r-xs);
  image-rendering: pixelated;
}
.rd-verified {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 12px;
  height: 12px;
  flex: none;
  border-radius: 50%;
  background: rgba(78, 201, 160, 0.16);
  color: var(--success, #4ec9a0);
}
.rd-verified svg {
  width: 8px;
  height: 8px;
}

.rd-room-ops {
  display: flex;
  align-items: center;
  gap: 5px;
  flex: none;
}
.rd-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  height: 30px;
  padding: 0 12px;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: transparent;
  font-size: var(--fs-xs);
  color: var(--text-1);
  white-space: nowrap;
  flex: none;
  cursor: pointer;
  transition: transform var(--dur-sm, 0.15s) var(--ease-out, ease);
}
.rd-btn svg {
  width: 13px;
  height: 13px;
}
.rd-btn.primary {
  border: none;
  background: var(--accent);
  /* 跟随全局主题对比色：写死深色墨字在红色/紫色主题下会变成黑字 */
  color: var(--accent-contrast, #fff);
  font-weight: 600;
}
.rd-btn.small {
  height: 26px;
  padding: 0 10px;
  font-size: var(--fs-2xs);
}
.rd-btn:hover:not(:disabled) {
  filter: brightness(1.05);
}
.rd-btn:active:not(:disabled) {
  transform: scale(0.97);
}
.rd-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.rd-icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: var(--r-sm);
  border: 1px solid var(--border);
  background: transparent;
  color: var(--text-3);
  cursor: pointer;
}
.rd-icon-btn svg {
  width: 13px;
  height: 13px;
}
.rd-icon-btn:hover {
  color: var(--accent);
  border-color: var(--accent-35, rgba(232, 154, 75, 0.35));
}
.rd-filters {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.rd-filter {
  width: 150px;
}
.rd-filter.narrow {
  width: 120px;
}
.rd-tag.mode {
  color: var(--accent);
  background: var(--accent-08, rgba(232, 154, 75, 0.12));
}
/* 非活跃房间的「多久前活跃」压一档，避免看起来还在线 */
.rd-tag.stale {
  opacity: 0.6;
}
.rd-owner-inst {
  color: var(--text-3);
  opacity: 0.8;
}
/* 环境预检弹窗 */
.pack-body {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.pack-head {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.pack-list {
  margin: 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: var(--fs-xs);
  line-height: 1.6;
}
.pack-list .bad {
  color: var(--danger, #e5534b);
}
.pack-list .warn {
  color: var(--accent);
}
.pack-list .hint-line {
  color: var(--text-3);
}
.pack-items {
  display: block;
  margin-top: 2px;
  font-size: var(--fs-2xs);
  color: var(--text-3);
  word-break: break-all;
}
/* 手动来源的 mod：没有在线来源，颜色压一档，别和可安装的混在一起 */
.pack-items.manual {
  color: var(--text-3);
  opacity: 0.85;
}
.pack-fix {
  display: block;
  margin-top: 6px;
}
.rd-subhint {
  margin: -4px 0 0;
  font-size: var(--fs-2xs);
  line-height: 1.6;
  color: var(--text-3);
}
.rd-empty {
  margin: 0;
  padding: 20px 0;
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-3);
}
.rd-report {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.rd-report-ops {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
