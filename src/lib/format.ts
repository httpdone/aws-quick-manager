import type { Messages } from "../i18n";
import type { TimeUnit } from "../i18n/zh";

export const fmtDate = (secs: number) => (secs ? new Date(secs * 1000).toLocaleString() : "—");

export const fmtTime = (iso: string | null) => (iso ? new Date(iso).toLocaleString() : "—");

export const fmtHours = (h: number) => `${Math.round(h * 10) / 10} h`;
export const fmtMins = (m: number) => `${Math.round(m)} min`;
/** Quota amounts arrive in the window's own unit despite the *_hours field names. */
export const fmtQuota = (v: number, unit: string) => (unit === "min" ? fmtMins(v) : fmtHours(v));

/** Unix seconds of an ISO timestamp, 0 when missing. */
export const isoSecs = (iso: string | null) => (iso ? Math.floor(new Date(iso).getTime() / 1000) : 0);

/** Relative time ("3 minutes ago", "in 2 days") for unix seconds. */
export function fmtRelative(secs: number, t: Messages): string {
  if (!secs) return "—";
  const diff = secs - Date.now() / 1000;
  const abs = Math.abs(diff);
  const units: [number, TimeUnit][] = [
    [86400 * 365, "year"],
    [86400 * 30, "month"],
    [86400, "day"],
    [3600, "hour"],
    [60, "minute"],
  ];
  for (const [size, unit] of units) {
    if (abs >= size) {
      const n = Math.floor(abs / size);
      return diff < 0 ? t.time.ago(n, unit) : t.time.later(n, unit);
    }
  }
  return diff < 0 ? t.time.justNow : t.time.soon;
}

/** First character of a name, for avatars. */
export function initial(name: string): string {
  const chars = [...name.trim()];
  return chars.length ? chars[0].toUpperCase() : "?";
}

/** A stable hue for a string, for avatar colours. */
export function hue(seed: string): number {
  let h = 0;
  for (const ch of seed) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
  return h % 360;
}

/** Quota resets on the 1st of next month (the desktop's rule). */
export function nextResetLabel(t: Messages): string {
  const month = (new Date().getMonth() + 1) % 12;
  return t.time.resetsOn(month + 1);
}
