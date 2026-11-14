import type { InjectionKey } from "vue";
import type { HomeAction, HomeNode, HomeVars } from "./types";

/**
 * 渲染器上下文：由 HomeLayout 注入，HomeNode 递归消费。
 * 把「取变量 / 判条件 / 跑事件 / 解析图片地址」集中在一处，
 * 递归组件里就只剩布局本身，不需要层层透传 props。
 */
export interface HomeContext {
  /** 当前变量表（响应式），键值统一为字符串 */
  vars: HomeVars;
  /** 替换文本里的 {占位符} */
  interp: (text: string | undefined) => string;
  /** 节点是否满足 if / unless 条件 */
  visible: (node: HomeNode) => boolean;
  /** 执行一个或多个事件 */
  run: (action: HomeAction | HomeAction[] | undefined) => void;
  /** 图片地址：网址 / data URL 原样返回，本地路径转成 WebView 可加载的地址 */
  imgSrc: (src: string | undefined) => string;
}

export const HOME_CTX: InjectionKey<HomeContext> = Symbol("qookix-home-ctx");
