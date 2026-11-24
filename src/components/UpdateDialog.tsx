import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { relaunch } from "@tauri-apps/plugin-process";
import type { Update } from "@tauri-apps/plugin-updater";
import { updateChanges } from "../lib/api";
import { useT } from "../i18n";
import { errorText } from "../lib/errors";
import { REPO_URL, releaseNotes } from "../lib/updates";
import Icon from "./Icon";
import Modal from "./Modal";
import { Alert, Spinner, cx, useUI } from "./ui";

interface Props {
  update: Update;
  onClose: () => void;
}

type Phase = "ready" | "downloading" | "installing" | "error";

const fmtBytes = (n: number) => (n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`);
const isWindows = navigator.userAgent.includes("Windows");

export default function UpdateDialog({ update, onClose }: Props) {
  const t = useT();
  const { toast } = useUI();
  const [phase, setPhase] = useState<Phase>("ready");
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState("");
  const [commits, setCommits] = useState<string[]>([]);
  const notes = releaseNotes(update.body);

  // no notes on the release: list the commits since this version instead
  useEffect(() => {
    if (notes) return;
    let alive = true;
    updateChanges(update.currentVersion, update.version)
      .then((list) => alive && setCommits(list))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [notes, update.currentVersion, update.version]);

  const releaseUrl = `${REPO_URL}/releases/tag/v${update.version.replace(/^v/, "")}`;
  const open = (url: string) => openUrl(url).catch((e) => toast(errorText(t, e), "bad"));

  const install = async () => {
    setPhase("downloading");
    setDownloaded(0);
    setTotal(0);
    setError("");
    try {
      let got = 0;
      // downloads, checks the signature against the bundled public key, and
      // installs; on Windows the installer takes over and this app exits
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            setTotal(event.data.contentLength ?? 0);
            break;
          case "Progress":
            got += event.data.chunkLength;
            setDownloaded(got);
            break;
          case "Finished":
            setPhase("installing");
            break;
        }
      });
      setPhase("installing");
      await relaunch();
    } catch (e) {
      setError(errorText(t, e));
      setPhase("error");
    }
  };

  const busy = phase === "downloading" || phase === "installing";
  const pct = total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : null;
  const published = update.date ? update.date.slice(0, 10) : "";

  return (
    <Modal
      title={t.update.title}
      size="lg"
      onClose={onClose}
      locked={busy}
      footer={
        <>
          <button className="btn ghost" onClick={() => open(releaseUrl)}>
            <Icon name="github" size={14} />
            {t.update.viewOnGithub}
          </button>
          <span className="spacer" />
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t.common.cancel}
          </button>
          <button className="btn primary" onClick={install} disabled={busy} autoFocus>
            {busy ? <Spinner /> : <Icon name="download" size={14} />}
            {phase === "error" ? t.common.retry : t.update.install}
          </button>
        </>
      }
    >
      <div className="updateHead">
        <span className="updateIcon">
          <Icon name="download" size={18} />
        </span>
        <div>
          <b>{t.update.versions(update.currentVersion, update.version)}</b>
          {published && <small>{t.update.published(published)}</small>}
        </div>
      </div>

      <section className="updateNotes">
        <h3>{t.update.notes}</h3>
        {notes ? (
          <div className="notes">{notes}</div>
        ) : commits.length > 0 ? (
          <ul className="changeList">
            {commits.map((c, i) => (
              <li key={i}>{c}</li>
            ))}
          </ul>
        ) : (
          <p className="muted">{t.update.noNotes}</p>
        )}
      </section>

      {phase === "downloading" && (
        <div className="progress">
          <div className={cx("progressBar", pct === null && "indeterminate")}>
            <span style={pct === null ? undefined : { width: `${pct}%` }} />
          </div>
          <small className="muted">
            {pct === null ? t.update.downloadingUnknown : t.update.downloading(pct)}
            {total > 0 ? ` · ${fmtBytes(downloaded)} / ${fmtBytes(total)}` : ""}
          </small>
        </div>
      )}
      {phase === "installing" && <Alert tone="info">{isWindows ? t.update.installingWindows : t.update.installingMac}</Alert>}
      {phase === "error" && (
        <Alert tone="bad">
          <b>{t.update.failed}</b>
          <p>{error}</p>
        </Alert>
      )}
      {(phase === "ready" || phase === "error") && <p className="updateHint">{isWindows ? t.update.hintWindows : t.update.hintMac}</p>}
    </Modal>
  );
}
