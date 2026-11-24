export type Theme = "system" | "light" | "dark";

const THEME_KEY = "aqm_theme";
const LAUNCH_KEY = "aqm_launch_after_switch";
const FORCE_KEY = "aqm_force_quit";

export function getTheme(): Theme {
  const v = localStorage.getItem(THEME_KEY);
  return v === "light" || v === "dark" ? v : "system";
}

export function applyTheme(theme: Theme = getTheme()) {
  const root = document.documentElement;
  if (theme === "system") delete root.dataset.theme;
  else root.dataset.theme = theme;
}

export function setTheme(theme: Theme) {
  localStorage.setItem(THEME_KEY, theme);
  applyTheme(theme);
}

/** Start Quick again after a switch (default on). */
export const getLaunchAfterSwitch = () => localStorage.getItem(LAUNCH_KEY) !== "0";

export const setLaunchAfterSwitch = (on: boolean) => localStorage.setItem(LAUNCH_KEY, on ? "1" : "0");

/** Force-quit Quick right away when switching, without asking it first (default off). */
export const getForceQuit = () => localStorage.getItem(FORCE_KEY) === "1";

export const setForceQuit = (on: boolean) => localStorage.setItem(FORCE_KEY, on ? "1" : "0");

export function getStored<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  const v = localStorage.getItem(key);
  return (allowed as readonly string[]).includes(v ?? "") ? (v as T) : fallback;
}
