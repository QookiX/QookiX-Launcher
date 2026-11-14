import { defineAsyncComponent, type Component } from "vue";

/**
 * 动态背景场景注册表。
 *
 * 新增一个场景要做三件事：
 *   1. 在 `src/components/background/` 下写一个组件，props 固定为 `{ dark, accent }`，
 *      根元素自带类名，并把这个类名加进 `src/styles/base.css` 的层级选择器
 *      （否则 `.app-bg > *` 会把画布抬到内容之上）；
 *   2. 在这里登记（id / 两处 i18n key / 缩略图样式）；
 *   3. 在设置页的缩略图网格里补一张 CSS 缩略图。
 *
 * 组件一律走 `defineAsyncComponent` + 动态 import：不用动态背景的用户，
 * 不会为 three.js 或任何场景代码付出首屏成本。
 */
export interface BackgroundScene {
  id: string;
  /** 设置页卡片标题的 i18n key */
  labelKey: string;
  /** 设置页卡片说明的 i18n key */
  descKey: string;
  /** 缩略图区域额外挂的修饰类：`scene-thumb--<thumb>` */
  thumb: string;
  component: Component;
}

export const BACKGROUND_SCENES: BackgroundScene[] = [
  {
    id: "island",
    labelKey: "settings.appearance.sceneIsland",
    descKey: "settings.appearance.sceneIslandDesc",
    thumb: "island",
    component: defineAsyncComponent(() => import("./IslandScene.vue")),
  },
  {
    id: "train",
    labelKey: "settings.appearance.sceneTrain",
    descKey: "settings.appearance.sceneTrainDesc",
    thumb: "train",
    component: defineAsyncComponent(() => import("./CloudTrainScene.vue")),
  },
];

/** 老配置没有 `background_scene` 字段时回落到这里 */
export const DEFAULT_SCENE_ID = "island";

/** 空值 / 脏值一律回落到默认场景，避免前端背景整片空白 */
export function resolveScene(id: string | null | undefined): BackgroundScene {
  return BACKGROUND_SCENES.find((s) => s.id === id) ?? BACKGROUND_SCENES[0];
}
