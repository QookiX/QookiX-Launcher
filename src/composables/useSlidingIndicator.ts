import { onBeforeUnmount, onMounted, onUpdated, ref, type Ref } from "vue";

interface Options {
  /** Direction the indicator travels along: vertical = top/height, horizontal = left/width */
  axis?: "vertical" | "horizontal";
}

/**
 * Sliding active-highlight indicator.
 *
 * Positions an absolutely positioned element over the currently active child
 * and smoothly slides/scales it to follow the active item when it changes,
 * producing the "highlight expands to the target button then settles" effect.
 *
 * Usage:
 *   <div ref="box" class="rel">
 *     <div class="indicator" :style="indicatorStyle"></div>
 *     <button v-for="..." ref="items">…</button>
 *   </div>
 *
 * @param containerRef ref of the relative-positioned container
 * @param getItems     () => the clickable items in DOM order (e.g. container.querySelectorAll)
 * @param getActive    () => index of the active item (or -1 when none)
 * @param opts         axis: "vertical" (default) | "horizontal"
 */
export function useSlidingIndicator(
  containerRef: Ref<HTMLElement | null>,
  getItems: () => HTMLElement[],
  getActive: () => number,
  opts: Options = {}
) {
  const axis = opts.axis ?? "vertical";
  const indicatorStyle = ref<Record<string, string>>({ opacity: "0" });

  /** Resolve a ref value to a DOM element (unwraps Vue component instances too). */
  function toElement(v: unknown): HTMLElement | null {
    if (v instanceof HTMLElement) return v;
    // <router-link> and other components expose their root element via $el
    if (v && typeof v === "object" && "$el" in v) {
      const el = (v as { $el?: unknown }).$el;
      return el instanceof HTMLElement ? el : null;
    }
    return null;
  }

  // 记录上一次的矩形，用于“从当前位置直接平滑滑动到目标”的连续过渡
  let last: { top: number; height: number; left: number; width: number } | null = null;

  function setRect(rect: { top: number; height: number; left: number; width: number }, dur: string) {
    const base: Record<string, string> =
    axis === "vertical"
      ? { top: `${rect.top}px`, height: `${rect.height}px`, left: `${rect.left}px`, width: `${rect.width}px` }
      : { left: `${rect.left}px`, width: `${rect.width}px` };
    indicatorStyle.value = {
      opacity: "1",
      ...base,
      transition: `top ${dur}, height ${dur}, left ${dur}, width ${dur}, opacity 0.2s`,
    };
  }

  /** 无激活项时淡出隐藏（保留位置样式，返回有激活项的页面时再滑入）。 */
  function hide(dur = "0.18s") {
    // 保留上次的几何信息，仅把透明度归零并只过渡 opacity
    const prev = indicatorStyle.value;
    indicatorStyle.value = { ...prev, opacity: "0", transition: `opacity ${dur}` };
  }

  function update() {
    const container = toElement(containerRef.value);
    if (!container) return;
    // Query items fresh each time so the index always matches the DOM order,
    // even when the list is filtered/reordered dynamically.
    const idx = getActive();
    const el = toElement(idx >= 0 ? getItems()[idx] : null);
    if (!el) {
      // 当前页面不在导航列表中（如设置页）：指示器淡出，而不是停留原地
      hide();
      return;
    }
    const r = el.getBoundingClientRect();
    const c = container.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) {
      // 元素存在但量不到尺寸（祖先被 v-show 藏起来 / 还没完成布局）：
      // 绝不能把 0×0 写进样式，否则指示器会被永久钉成「看不见」，
      // 只能等用户点一次选项触发 watch 才复活。这里保持未定位状态，
      // 由 ResizeObserver 或父组件的 onUpdated 补测。
      last = null;
      hide("0s");
      return;
    }
    const to = {
      top: r.top - c.top,
      height: r.height,
      left: r.left - c.left,
      width: r.width,
    };
    if (!last) {
      // 首次定位：直接到位，无动画
      setRect(to, "0s");
      last = to;
      return;
    }
    // 连续过渡：直接从当前位置滑动 + 缩放贴合到目标，一次到位，
    // 不再用「先包裹再收缩」的两段式（那种中间会有明显的停顿跳变）。
    setRect(to, "0.26s");
    last = to;
  }

  let raf = 0;
  /** Schedule a re-measure on the next animation frame (call after DOM updates). */
  function refresh() {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(update);
  }

  /** Synchronously re-measure the active item and apply the rect with zero
      transition. Besides being snap()'s implementation, callers can invoke it
      per-frame while the container itself is animating (e.g. sidebar expand),
      so the indicator hugs the item every frame instead of keeping stale
      geometry and jumping at the end. */
  function syncNow() {
    const container = toElement(containerRef.value);
    if (!container) return;
    const idx = getActive();
    const el = toElement(idx >= 0 ? getItems()[idx] : null);
    if (!el) {
      hide("0s");
      return;
    }
    const r = el.getBoundingClientRect();
    const c = container.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return; // 还没布局好，等下一帧再贴
    const to = {
      top: r.top - c.top,
      height: r.height,
      left: r.left - c.left,
      width: r.width,
    };
    setRect(to, "0s");
    last = to;
  }

  /** Instantly snap to the active item without animation (use after layout changes). */
  function snap() {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(syncNow);
  }

  // —— 自愈：元素真正出现后自动补测 ——
  // 这个 composable 注册在「页面组件」上，不是每套 seg 自己的子组件，onMounted 只跑一次。
  // 而 seg 常常在那一刻正被 v-show 藏起来（display:none → 量到 0×0），
  // 或在 v-if 分支里还没渲染（例如设置页的「画质」只在背景为动态场景时才存在）。
  // 没有兜底的话，指示器会一直停在 opacity:0 / 宽 0 的状态，只有用户点一次选项、
  // 触发 watch 里的 refresh 才会突然冒出来。
  let ro: ResizeObserver | null = null;
  let bound: { container: HTMLElement; item: HTMLElement | null } | null = null;

  function bindObserver() {
    const container = toElement(containerRef.value);
    if (!container) return;
    const idx = getActive();
    const item = toElement(idx >= 0 ? getItems()[idx] : null);
    if (bound && bound.container === container && bound.item === item) return;
    // 换了容器（v-show / v-if 重建出的是新元素）：按首次定位处理，不要从旧位置滑过去
    if (bound?.container !== container) last = null;
    bound = { container, item };
    if (!ro) ro = new ResizeObserver(() => refresh());
    ro.disconnect();
    ro.observe(container);
    if (item) ro.observe(item);
  }

  onMounted(() => {
    bindObserver();
    update();
    window.addEventListener("resize", refresh);
  });
  // 父组件更新意味着 DOM 可能刚被重建 / 从隐藏变可见：重新绑定并在未定位时补测
  onUpdated(() => {
    bindObserver();
    if (!last) refresh();
  });
  onBeforeUnmount(() => {
    cancelAnimationFrame(raf);
    ro?.disconnect();
    ro = null;
    bound = null;
    window.removeEventListener("resize", refresh);
  });

  return { indicatorStyle, update, refresh, snap, syncNow };
}
