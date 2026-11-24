import type { ErrorParams, Messages } from "../i18n/zh";

/** What a failed backend command rejects with (see `src-tauri/src/error.rs`). */
export interface AppError {
  /** stable, e.g. `desktop.still_running`; decide on this, never on the text */
  code: string;
  params?: ErrorParams;
  /** the backend's own wording, used when the UI has no text for `code` */
  message: string;
}

export function isAppError(e: unknown): e is AppError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as AppError).code === "string" &&
    typeof (e as AppError).message === "string"
  );
}

/** The error's code, or null for errors that didn't come from the backend. */
export const errorCode = (e: unknown): string | null => (isAppError(e) ? e.code : null);

type Entry = string | ((p: ErrorParams) => string);

/** The error in the UI language. */
export function errorText(t: Messages, e: unknown): string {
  if (isAppError(e)) {
    const entry = (t.errors as Record<string, Entry | undefined>)[e.code];
    if (typeof entry === "function") return entry(e.params ?? {});
    if (typeof entry === "string") return entry;
    return e.message;
  }
  if (e instanceof Error) return e.message;
  return String(e);
}
