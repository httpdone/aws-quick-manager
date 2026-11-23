import { Account, Usage } from "./api";

export type Tone = "ok" | "warn" | "bad";

export const who = (a: { display_name: string; email: string; id: string }) => a.display_name || a.email || a.id;

/** The account has a session somewhere, so its quota can be read. */
export const canQuota = (a: Account) => a.has_credentials || a.in_vault;

/** The account can be switched to. */
export const canSwitch = (a: Account) => !a.active && !a.error && canQuota(a);

export function accountStatus(a: Account): { label: string; tone: Tone } {
  if (a.error) return { label: "凭据读取失败", tone: "bad" };
  if (a.flag === "auth_invalid") return { label: "登录失效", tone: "bad" };
  if (a.flag === "forbidden") return { label: "无权限", tone: "bad" };
  if (!a.has_credentials && !a.in_vault) return { label: "未登录", tone: "bad" };
  if (a.refresh_failing_since) return { label: "续期失败", tone: "bad" };
  const now = Date.now() / 1000;
  if (a.refresh_expires_at && a.refresh_expires_at < now) return { label: "登录已过期", tone: "bad" };
  if (a.access_expires_at && a.access_expires_at < now) return { label: "待续期", tone: "warn" };
  return { label: "正常", tone: "ok" };
}

/** Hours left in the account's main pool, -1 when unknown. */
export const leftHours = (u?: Usage) => (u && u.windows.length ? u.windows[0].left_hours : -1);
