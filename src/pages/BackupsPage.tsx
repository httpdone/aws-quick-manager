import { useCallback, useEffect, useState } from "react";
import { BackupInfo, backupNow, deleteBackup, listBackups, restoreBackup } from "../lib/api";
import { fmtDate, fmtRelative } from "../lib/format";
import Icon from "../components/Icon";
import { Alert, Empty, Spinner, cx, useUI } from "../components/ui";

interface Props {
  desktopRunning: boolean;
  onRestored: () => void;
}

export default function BackupsPage({ desktopRunning, onRestored }: Props) {
  const { toast, confirm } = useUI();
  const [backups, setBackups] = useState<BackupInfo[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setBackups(await listBackups());
    } catch (e) {
      toast(String(e), "bad");
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
      toast(String(e), "bad");
    } finally {
      setBusy(null);
    }
  };

  const doBackup = () =>
    act("new", async () => {
      await backupNow();
      return "已备份当前账号清单";
    });

  const doRestore = async (b: BackupInfo) => {
    const ok = await confirm({
      title: "恢复这份清单？",
      message: `${fmtDate(b.modified)} 的备份，${b.accounts} 个账号，当前账号：${b.active || "无"}。\n\n会覆盖 Quick 现在的 profiles.json，下次打开 Quick 生效。各账号的登录凭据不受影响。`,
      confirmLabel: "恢复",
    });
    if (!ok) return;
    await act(b.name, async () => {
      await restoreBackup(b.name);
      onRestored();
      return "已恢复清单，下次打开 Quick 生效";
    });
  };

  const doDelete = async (b: BackupInfo) => {
    const ok = await confirm({ title: "删除这份备份？", message: `${fmtDate(b.modified)} 的备份，删除后不能恢复。`, confirmLabel: "删除", danger: true });
    if (!ok) return;
    await act(b.name, async () => {
      await deleteBackup(b.name);
      return "已删除备份";
    });
  };

  return (
    <div className="page">
      <header className="pageHead">
        <div>
          <h1>清单备份</h1>
          <p className="pageSub">每次切换前自动保存 Quick 的账号清单，最多保留 30 份</p>
        </div>
        <div className="actions">
          <button className="btn ghost" onClick={load} disabled={!!busy}>
            <Icon name="refresh" />
            刷新
          </button>
          <button className="btn primary" onClick={doBackup} disabled={!!busy}>
            {busy === "new" ? <Spinner /> : <Icon name="save" />}
            立即备份
          </button>
        </div>
      </header>

      {desktopRunning && <Alert tone="info">Quick 正在运行，退出后才能恢复清单；删除备份不受影响。</Alert>}

      {backups === null && <div className="skeleton tall" />}
      {backups?.length === 0 && (
        <Empty icon="archive" title="还没有备份">
          <p>切换、添加或删除账号时会自动备份，也可以现在手动备份一份。</p>
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
                      {fmtRelative(b.modified)}
                      {i === 0 && " · 最新"}
                    </span>
                  </div>
                  <div className="tlActions">
                    <button
                      className="btn sm"
                      onClick={() => doRestore(b)}
                      disabled={!!busy || desktopRunning || !!b.parse_error}
                      title={desktopRunning ? "先退出 Quick" : undefined}
                    >
                      {busy === b.name ? <Spinner /> : <Icon name="history" size={14} />}
                      恢复
                    </button>
                    <button className="iconBtn dangerHover" onClick={() => doDelete(b)} disabled={!!busy} title="删除">
                      <Icon name="trash" />
                    </button>
                  </div>
                </div>
                {b.parse_error ? (
                  <p className="badText">{b.parse_error}</p>
                ) : (
                  <>
                    <div className="tlChips">
                      {b.entries.length === 0 && <span className="muted">空清单</span>}
                      {b.entries.map((e) => (
                        <span key={e.id} className={cx("tag", e.active && "accent")} title={e.id}>
                          {e.active && <Icon name="check" size={11} />}
                          {e.display_name || e.id}
                        </span>
                      ))}
                    </div>
                    {(b.added.length > 0 || b.removed.length > 0 || b.active_changed) && (
                      <div className="tlDiff" title="和现在的清单相比">
                        {b.active_changed && <span>恢复后当前账号变为 {b.active || "无"}</span>}
                        {b.added.length > 0 && <span className="plus">多 {b.added.length} 个：{b.added.join("、")}</span>}
                        {b.removed.length > 0 && <span className="minus">少 {b.removed.length} 个：{b.removed.join("、")}</span>}
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
