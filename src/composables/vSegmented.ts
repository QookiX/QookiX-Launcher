import type { ObjectDirective } from "vue";
import { prefersReducedMotion } from "./useMotionPreference";

interface SegmentedOptions {
  label: string;
  panel?: string;
  animatePanel?: boolean;
}

const activeTab = new WeakMap<HTMLElement, HTMLButtonElement>();
const panelMotion = new WeakMap<HTMLElement, Animation>();

function buttons(root: HTMLElement): HTMLButtonElement[] {
  return Array.from(root.querySelectorAll<HTMLButtonElement>(":scope > button"));
}

function enabled(button: HTMLButtonElement): boolean {
  return !button.disabled && !button.hidden && button.style.display !== "none";
}

function sync(root: HTMLElement, options: SegmentedOptions) {
  const items = buttons(root);
  const selected = items.find((button) => button.classList.contains("active"));
  const tabbable = selected && enabled(selected) ? selected : items.find(enabled);
  root.setAttribute("role", options.panel ? "tablist" : "radiogroup");
  root.setAttribute("aria-label", options.label);
  items.forEach((button, index) => {
    button.setAttribute("role", options.panel ? "tab" : "radio");
    button.setAttribute(options.panel ? "aria-selected" : "aria-checked", String(button === selected));
    button.tabIndex = button === tabbable ? 0 : -1;
    if (options.panel) {
      button.id = `${options.panel}-tab-${index}`;
      button.setAttribute("aria-controls", options.panel);
    }
  });
  if (options.panel && selected) {
    const panel = document.getElementById(options.panel);
    panel?.setAttribute("aria-labelledby", selected.id);
    const previous = activeTab.get(root);
    if (panel && previous && previous !== selected) {
      panelMotion.get(root)?.cancel();
      if (options.animatePanel !== false && !prefersReducedMotion.value) {
        panelMotion.set(root, panel.animate([{ opacity: 0.82 }, { opacity: 1 }], {
          duration: 180,
          easing: "ease-out",
        }));
      }
    }
    activeTab.set(root, selected);
  }
}

function navigate(event: KeyboardEvent) {
  const root = event.currentTarget as HTMLElement;
  const choices = buttons(root).filter(enabled);
  const index = choices.indexOf(event.target as HTMLButtonElement);
  if (index < 0) return;
  const rtl = getComputedStyle(root).direction === "rtl";
  let next: number;
  switch (event.key) {
    case "Home": next = 0; break;
    case "End": next = choices.length - 1; break;
    case "ArrowRight": next = index + (rtl ? -1 : 1); break;
    case "ArrowLeft": next = index + (rtl ? 1 : -1); break;
    default: return;
  }
  event.preventDefault();
  event.stopPropagation();
  const target = choices[(next + choices.length) % choices.length];
  target.focus();
  target.click();
}

export const vSegmented: ObjectDirective<HTMLElement, SegmentedOptions> = {
  mounted(root, { value }) {
    sync(root, value);
    root.addEventListener("keydown", navigate);
  },
  updated(root, { value }) { sync(root, value); },
  beforeUnmount(root) {
    root.removeEventListener("keydown", navigate);
    panelMotion.get(root)?.cancel();
  },
};
