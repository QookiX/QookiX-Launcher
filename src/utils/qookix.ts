/**
 * QookiX 房间的玩法类型：服务端是固定枚举（自由文本没法当筛选条件用）。
 * 客户端只做取值 → 文案的映射，取值必须与 backend rooms.go 的 roomPlayTypes 一致。
 */

export interface PlayTypeOption {
  value: string;
  labelKey: string;
}

export const PLAY_TYPES: PlayTypeOption[] = [
  { value: "survival", labelKey: "qookix.playType.survival" },
  { value: "creative", labelKey: "qookix.playType.creative" },
  { value: "minigame", labelKey: "qookix.playType.minigame" },
  { value: "modded", labelKey: "qookix.playType.modded" },
  { value: "building", labelKey: "qookix.playType.building" },
  { value: "other", labelKey: "qookix.playType.other" },
];

/**
 * 心跳距今多久（房间卡片用）。
 * 只给「多久之前」，不编延迟——陶瓦的 /state/ 里没有可用的 ping 数据。
 */
export function agoText(
  last: number,
  now: number,
  t: (key: string, params?: Record<string, unknown>) => string,
): string {
  const d = Math.max(0, Math.floor(now - last));
  if (d < 60) return t("qookix.time.justNow");
  if (d < 3600) return t("qookix.time.minutes", { n: Math.floor(d / 60) });
  if (d < 86400) return t("qookix.time.hours", { n: Math.floor(d / 3600) });
  return t("qookix.time.days", { n: Math.floor(d / 86400) });
}

/** 空值返回空串，界面据此决定要不要渲染这枚标签 */
export function playTypeLabel(
  value: string | undefined,
  t: (key: string) => string,
): string {
  if (!value) return "";
  const hit = PLAY_TYPES.find((p) => p.value === value);
  return hit ? t(hit.labelKey) : value;
}
