export const fmtDate = (secs: number) => (secs ? new Date(secs * 1000).toLocaleString() : "—");

export const fmtTime = (iso: string | null) => (iso ? new Date(iso).toLocaleString() : "—");

export const fmtHours = (h: number) => `${Math.round(h * 10) / 10} h`;

/** Unix seconds of an ISO timestamp, 0 when missing. */
export const isoSecs = (iso: string | null) => (iso ? Math.floor(new Date(iso).getTime() / 1000) : 0);

/** "3 分钟前" / "2 天后" for unix seconds. */
export function fmtRelative(secs: number): string {
  if (!secs) return "—";
  const diff = secs - Date.now() / 1000;
  const abs = Math.abs(diff);
  const units: [number, string][] = [
    [86400 * 365, "年"],
    [86400 * 30, "个月"],
    [86400, "天"],
    [3600, "小时"],
    [60, "分钟"],
  ];
  for (const [size, name] of units) {
    if (abs >= size) {
      const n = Math.floor(abs / size);
      return diff < 0 ? `${n} ${name}前` : `${n} ${name}后`;
    }
  }
  return diff < 0 ? "刚刚" : "马上";
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
export function nextResetLabel(): string {
  const month = (new Date().getMonth() + 1) % 12;
  return `${month + 1} 月 1 日重置`;
}
