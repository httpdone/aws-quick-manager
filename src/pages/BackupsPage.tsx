import { useCallback, useEffect, useState } from "react";
import { BackupInfo, backupNow, deleteBackup, listBackups, restoreBackup } from "../lib/api";
import { useT } from "../i18n";
import { errorText } from "../lib/errors";
import { fmtDate, fmtRelative } from "../lib/format";
import Icon from "../components/Icon";
import { Alert, Empty, Spinner, cx, useUI } from "../components/ui";

interface Props {
  desktopRunning: boolean;
  onRestored: () => void;
}

function backupActiveName(backup: BackupInfo, none: string) {
  return backup.entries.find((entry) => entry.active)?.display_name || backup.active || none;
}

export default function BackupsPage({ desktopRunning, onRestored }: Props) {
  const t = useT();
  const { toast, confirm } = useUI();
  const [backups, setBackups] = useState<BackupInfo[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setBackups(await listBackups());
    } catch (e) {
      toast(errorText(t, e), "bad");
      setBackups((prev) => prev ?? []);
    }
  }, [toast]);

  useEffect(() => {
    load();
  }, [load]);

  const act = async (key: string, action: () => Promise<string>) => {
    setBusy(key);
    try {
      toast(await action());
      await load();
    } catch (e) {
      toast(errorText(t, e), "bad");
    } finally {
      setBusy(null);
    }
  };

  const doBackup = () =>
    act("new", async () => {
      await backupNow();
      return t.backups.backedUp;
    });

  const doRestore = async (b: BackupInfo) => {
    const ok = await confirm({
      title: t.backups.restoreTitle,
      message: t.backups.restoreMessage(fmtDate(b.modified), b.accounts, backupActiveName(b, t.common.none)),
      confirmLabel: t.common.restore,
    });
    if (!ok) return;
    await act(b.name, async () => {
      await restoreBackup(b.name);
      onRestored();
      return t.backups.restored;
    });
  };

  const doDelete = async (b: BackupInfo) => {
    const ok = await confirm({ title: t.backups.deleteTitle, message: t.backups.deleteMessage(fmtDate(b.modified)), confirmLabel: t.common.delete, danger: true });
    if (!ok) return;
    await act(b.name, async () => {
      await deleteBackup(b.name);
      return t.backups.deleted;
    });
  };

  return (
    <div className="page">
      <header className="pageHead">
        <div>
          <h1>{t.backups.title}</h1>
          <p className="pageSub">{t.backups.sub}</p>
        </div>
        <div className="actions">
          <button className="btn ghost" onClick={load} disabled={!!busy}>
            <Icon name="refresh" />
            {t.common.refresh}
          </button>
          <button className="btn primary" onClick={doBackup} disabled={!!busy}>
            {busy === "new" ? <Spinner /> : <Icon name="save" />}
            {t.backups.backupNow}
          </button>
        </div>
      </header>

      {desktopRunning && <Alert tone="info">{t.backups.desktopRunning}</Alert>}

      {backups === null && <div className="skeleton tall" />}
      {backups?.length === 0 && (
        <Empty icon="archive" title={t.backups.empty}>
          <p>{t.backups.emptyHint}</p>
        </Empty>
      )}

      {backups && backups.length > 0 && (
        <ol className="timeline">
          {backups.map((b, i) => (
            <li key={b.name} className={cx("tlItem", b.parse_error && "isBad")}>
              <span className="tlDot" />
              <div className="tlCard">
                <div className="tlHead">
                  <div>
                    <b>{fmtDate(b.modified)}</b>
                    <span className="muted">
                      {fmtRelative(b.modified, t)}
                      {i === 0 && t.backups.latest}
                    </span>
                  </div>
                  <div className="tlActions">
                    <button
                      className="btn sm"
                      onClick={() => doRestore(b)}
                      disabled={!!busy || desktopRunning || !!b.parse_error}
                      title={desktopRunning ? t.backups.quitFirst : undefined}
                    >
                      {busy === b.name ? <Spinner /> : <Icon name="history" size={14} />}
                      {t.common.restore}
                    </button>
                    <button className="iconBtn dangerHover" onClick={() => doDelete(b)} disabled={!!busy} title={t.common.delete}>
                      <Icon name="trash" />
                    </button>
                  </div>
                </div>
                {b.parse_error ? (
                  <p className="badText">{t.backups.parseError}</p>
                ) : (
                  <>
                    <div className="tlChips">
                      {b.entries.length === 0 && <span className="muted">{t.backups.emptyList}</span>}
                      {b.entries.map((e) => (
                        <span key={e.id} className={cx("tag", e.active && "accent")} title={e.id}>
                          {e.active && <Icon name="check" size={11} />}
                          {e.display_name || e.id}
                        </span>
                      ))}
                    </div>
                    {(b.added.length > 0 || b.removed.length > 0 || b.active_changed) && (
                      <div className="tlDiff" title={t.backups.diffTitle}>
                        {b.active_changed && <span>{t.backups.activeChanges(backupActiveName(b, t.common.none))}</span>}
                        {b.added.length > 0 && <span className="plus">{t.backups.more(b.added.length, b.added.join(t.common.listSep))}</span>}
                        {b.removed.length > 0 && <span className="minus">{t.backups.fewer(b.removed.length, b.removed.join(t.common.listSep))}</span>}
                      </div>
                    )}
                  </>
                )}
              </div>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
