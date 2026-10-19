// Navigation between views: six sections per controller, plus two global pages.

import { $ } from "./ui-common";

export type View =
  | "overview"
  | "controls"
  | "rules"
  | "triggers"
  | "lighting"
  | "motion"
  | "profiles"
  | "settings"
  | "diagnostic";

/** The views that concern the selected controller; the others are global. */
const CONTROLLER_VIEWS: View[] = ["overview", "controls", "rules", "triggers", "lighting", "motion"];
const ALL_VIEWS: View[] = [...CONTROLLER_VIEWS, "profiles", "settings", "diagnostic"];
const STORAGE_KEY = "ds5forge.view";

export const isControllerView = (v: View) => CONTROLLER_VIEWS.includes(v);

function load(): View {
  try {
    const saved = localStorage.getItem(STORAGE_KEY) as View | null;
    if (saved && ALL_VIEWS.includes(saved)) return saved;
  } catch {
    // Storage unavailable: start from the overview.
  }
  return "overview";
}

let current: View = load();
/** The last controller view visited: the one shown again when coming back from a global page. */
let lastControllerView: View = isControllerView(current) ? current : "overview";
let hasControllers = false;
const listeners = new Set<(view: View) => void>();

export const getView = () => current;
export const onViewChange = (cb: (view: View) => void) => listeners.add(cb);

function apply() {
  document.querySelectorAll<HTMLElement>(".view").forEach((el) => {
    el.hidden = el.dataset.view !== current;
  });
  document.querySelectorAll<HTMLElement>("[data-nav]").forEach((el) => {
    const active = el.dataset.nav === current;
    el.classList.toggle("active", active);
    el.setAttribute("aria-current", active ? "page" : "false");
  });

  const onController = isControllerView(current);
  $("controller-view").hidden = !(onController && hasControllers);
  $("empty").hidden = !(onController && !hasControllers);
}

export function setView(view: View) {
  if (view === current) return;
  current = view;
  if (isControllerView(view)) lastControllerView = view;
  try {
    localStorage.setItem(STORAGE_KEY, view);
  } catch {
    // Unimportant: the view is simply not remembered.
  }
  apply();
  listeners.forEach((cb) => cb(view));
}

/** Choosing a controller goes back to its settings when a global page is showing. */
export function showControllerView() {
  if (!isControllerView(current)) setView(lastControllerView);
}

/** To be called again when the list of controllers changes (empty or not). */
export function refreshViews(controllersPresent: boolean) {
  hasControllers = controllersPresent;
  apply();
}

export function initViews() {
  document.querySelectorAll<HTMLElement>("[data-nav]").forEach((el) => {
    el.addEventListener("click", () => setView(el.dataset.nav as View));
  });
  apply();
}
