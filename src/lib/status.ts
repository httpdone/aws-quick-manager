import type { Messages } from "../i18n";
import { Account, Usage } from "./api";

export type Tone = "ok" | "warn" | "bad";

export const who = (a: { display_name: string; email: string; id: string }) => a.display_name || a.email || a.id;

/** The account has a session somewhere, so its quota can be read. */
export const canQuota = (a: Account) => a.has_credentials || a.in_vault;

/** The account can be switched to. */
export const canSwitch = (a: Account) => !a.active && !a.error && canQuota(a);

export function accountStatus(a: Account, t: Messages): { label: string; tone: Tone } {
  const s = t.status;
  if (a.error) return { label: s.credError, tone: "bad" };
  if (a.flag === "auth_invalid") return { label: s.authInvalid, tone: "bad" };
  if (a.flag === "forbidden") return { label: s.forbidden, tone: "bad" };
  if (!a.has_credentials && !a.in_vault) return { label: s.signedOut, tone: "bad" };
  if (a.refresh_failing_since) return { label: s.refreshFailing, tone: "bad" };
  // an expired access token alone is normal: it lasts about an hour and is
  // renewed automatically the next time the account is used
  const now = Date.now() / 1000;
  if (a.refresh_expires_at && a.refresh_expires_at < now) return { label: s.expired, tone: "bad" };
  return { label: s.ok, tone: "ok" };
}

/** The account can't be used until the user signs in to it again. */
export const needsSignIn = (a: Account, t: Messages) => accountStatus(a, t).tone === "bad";

/** Hours left in the account's main pool, -1 when unknown. */
export const leftHours = (u?: Usage) => (u && u.windows.length ? u.windows[0].left_hours : -1);
