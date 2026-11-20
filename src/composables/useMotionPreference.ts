import { readonly, shallowRef } from "vue";

const media = typeof window === "undefined"
  ? undefined
  : window.matchMedia("(prefers-reduced-motion: reduce)");
const reducedMotion = shallowRef(media?.matches ?? false);
media?.addEventListener("change", (event) => { reducedMotion.value = event.matches; });

export const prefersReducedMotion = readonly(reducedMotion);

export function motionScrollBehavior(): ScrollBehavior {
  return reducedMotion.value ? "auto" : "smooth";
}
