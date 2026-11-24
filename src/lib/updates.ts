import { useCallback, useEffect, useRef, useState } from "react";
import { Update, check as checkForUpdate } from "@tauri-apps/plugin-updater";

export const REPO_URL = "https://github.com/httpdone/aws-quick-manager";
export const ISSUES_URL = `${REPO_URL}/issues/new`;

/** First check a moment after start-up, then again when it's been a while. */
const FIRST_CHECK_MS = 3000;
const RECHECK_MS = 6 * 60 * 60 * 1000;
const TICK_MS = 30 * 60 * 1000;

export type UpdateStatus = "idle" | "checking" | "done" | "error";

export interface Updates {
  /** the newer release, or null when up to date / not checked yet */
  update: Update | null;
  status: UpdateStatus;
  error: unknown;
  /** check now; resolves with the newer release, or null */
  check: () => Promise<Update | null>;
}

/** Release notes worth showing: the workflow's stock text doesn't count. */
export function releaseNotes(body: string | undefined): string {
  const text = (body ?? "").trim();
  return text.startsWith("Automated build from") ? "" : text;
}

/**
 * Checks through Tauri's updater plugin: it reads `latest.json` from the
 * latest GitHub release and only offers a package whose signature matches
 * the public key in tauri.conf.json.
 */
export function useUpdates(): Updates {
  const [update, setUpdate] = useState<Update | null>(null);
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [error, setError] = useState<unknown>(null);
  const last = useRef(0);
  const busy = useRef(false);

  const check = useCallback(async () => {
    if (busy.current) return null;
    busy.current = true;
    setStatus("checking");
    setError(null);
    try {
      const next = await checkForUpdate({ timeout: 15000 });
      setUpdate(next);
      setStatus("done");
      return next;
    } catch (e) {
      setError(e);
      setStatus("error");
      return null;
    } finally {
      last.current = Date.now();
      busy.current = false;
    }
  }, []);

  useEffect(() => {
    const first = window.setTimeout(() => void check(), FIRST_CHECK_MS);
    const tick = () => {
      if (document.visibilityState === "visible" && last.current && Date.now() - last.current > RECHECK_MS) void check();
    };
    const timer = window.setInterval(tick, TICK_MS);
    window.addEventListener("focus", tick);
    return () => {
      window.clearTimeout(first);
      window.clearInterval(timer);
      window.removeEventListener("focus", tick);
    };
  }, [check]);

  return { update, status, error, check };
}
