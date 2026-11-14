/**
 * 动态背景的画质档位。
 *
 * 背景是常驻渲染的，画质和 GPU 占用天然互相拉扯，所以交给用户自己选：
 * 笔记本电池供电时选「流畅」，台式机/独显选「高清」。
 *
 * 三个档位实际影响两件事：
 *   1. 画布像素比与目标帧率（`pixelRatio` / `fps`）；
 *   2. 着色器内部细节（`level`）：云海列车的云层数与颗粒噪声、海岛的水面细分。
 *
 * 改档位会重建场景组件（App.vue 把档位拼进 `:key`）—— `antialias` 和
 * `powerPreference` 只在 WebGL 上下文创建时生效，不重建就换不掉。
 */

export type BgQualityId = "low" | "medium" | "high";

export interface BgQualityProfile {
  id: BgQualityId;
  /** 设置页分段控件的 i18n key */
  labelKey: string;
  /** 画布像素比（相对 CSS 像素）。传设备的 devicePixelRatio，由它决定是否超采样 */
  pixelRatio: (devicePixelRatio: number) => number;
  /** 目标帧率上限 */
  fps: number;
  /** 传给着色器的细节档：0 低 / 1 中 / 2 高 */
  level: 0 | 1 | 2;
}

export const BG_QUALITIES: BgQualityProfile[] = [
  {
    id: "low",
    labelKey: "settings.appearance.bgQualityLow",
    // 1x 屏 0.75、2x 屏也是 0.75：明显低于原生，换省电
    pixelRatio: (dpr) => Math.min(dpr, 1) * 0.75,
    fps: 30,
    level: 0,
  },
  {
    id: "medium",
    labelKey: "settings.appearance.bgQualityMedium",
    // 1x 屏原生分辨率，2x 屏 1.5 倍（高 DPI 下依旧清晰）
    pixelRatio: (dpr) => Math.min(dpr, 1.5),
    fps: 60,
    level: 1,
  },
  {
    id: "high",
    labelKey: "settings.appearance.bgQualityHigh",
    // 1x 屏 1.4 倍超采样（边缘更顺），2x 屏封顶 2 倍
    pixelRatio: (dpr) => Math.min(dpr * 1.4, 2),
    fps: 60,
    level: 2,
  },
];

/** 后端 `background_quality` 为空串（老配置 / 从没设置过）时用它 */
export const DEFAULT_QUALITY_ID: BgQualityId = "medium";

/** 空值 / 脏值回落到默认档，避免脏配置让背景停在最低画质 */
export function resolveQuality(id: string | null | undefined): BgQualityProfile {
  return (
    BG_QUALITIES.find((q) => q.id === id) ??
    BG_QUALITIES.find((q) => q.id === DEFAULT_QUALITY_ID)!
  );
}
