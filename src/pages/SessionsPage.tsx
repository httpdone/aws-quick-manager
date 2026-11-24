import { useCallback, useEffect, useMemo, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  Account,
  CloudSession,
  SessionInfo,
  exportCloudSessions,
  exportSessions,
  importCloudSessions,
  listCloudSessions,
  listSessions,
} from "../lib/api";
import { useT } from "../i18n";
import { errorText } from "../lib/errors";
import { fmtRelative, fmtDate } from "../lib/format";
import { who } from "../lib/status";
import Icon from "../components/Icon";
import { Alert, Empty, Segmented, Spinner, cx, useUI } from "../components/ui";

interface Props {
  accounts: Account[];
}

type Source = "cloud" | "local";

interface Row {
  id: string;
  title: string;
  created_at: number;
  updated_at: number;
  message_count: number;
  archived: boolean;
  server_session_id?: string;
  cloud: boolean;
}

export default function SessionsPage({ accounts }: Props) {
  const t = useT();
  const { toast, confirm } = useUI();
  const [accountId, setAccountId] = useState("");
  const [targetId, setTargetId] = useState("");
  const [source, setSource] = useState<Source>("cloud");
  const [sessions, setSessions] = useState<Row[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [format, setFormat] = useState<"md" | "json">("md");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const account = useMemo(() => accounts.find((a) => a.id === accountId) ?? null, [accounts, accountId]);
  const targets = accounts.filter((a) => a.id !== accountId);

  useEffect(() => {
    if (accountId && accounts.some((a) => a.id === accountId)) return;
    const preferred = accounts.find((a) => a.active) ?? accounts[0];
    setAccountId(preferred?.id ?? "");
  }, [accounts, accountId]);

  useEffect(() => {
    if (targetId && targetId !== accountId && accounts.some((a) => a.id === targetId)) return;
    setTargetId(accounts.find((a) => a.id !== accountId)?.id ?? "");
  }, [accounts, accountId, targetId]);

  const load = useCallback(async () => {
    if (!accountId) {
      setSessions([]);
      return;
    }
    setLoading(true);
    setError("");
    setSelected(new Set());
    try {
      if (source === "cloud") {
        const list = await listCloudSessions(accountId);
        setSessions(list.map((s: CloudSession) => ({ ...s, cloud: true })));
      } else {
        const list = await listSessions(accountId);
        setSessions(list.map((s: SessionInfo) => ({ ...s, cloud: false })));
      }
    } catch (e) {
      setSessions([]);
      setError(errorText(t, e));
    } finally {
      setLoading(false);
    }
  }, [accountId, source]);

  useEffect(() => {
    load();
  }, [load]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = (sessions ?? []).filter((s) => !q || `${s.title} ${s.id}`.toLowerCase().includes(q));
    return rows.sort((a, b) => b.updated_at - a.updated_at);
  }, [sessions, query]);

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const allSelected = visible.length > 0 && visible.every((s) => selected.has(s.id));
  const toggleAll = () =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (allSelected) visible.forEach((s) => next.delete(s.id));
      else visible.forEach((s) => next.add(s.id));
      return next;
    });

  const runExport = async (ids: string[]) => {
    if (!account) return;
    setBusy(true);
    try {
      const result =
        source === "cloud" ? await exportCloudSessions(account.id, ids, format) : await exportSessions(account.id, ids, format);
      const count = "events" in result ? result.events : result.messages;
      toast(t.sessions.exported(result.sessions, count, source === "cloud"), "ok", {
        label: t.common.revealInFolder,
        run: () => revealItemInDir(result.path).catch((e) => toast(errorText(t, e), "bad")),
      });
    } catch (e) {
      toast(t.sessions.exportFailed(errorText(t, e)), "bad");
    } finally {
      setBusy(false);
    }
  };

  const runImport = async () => {
    const target = accounts.find((a) => a.id === targetId);
    if (!account || !target || selected.size === 0) return;
    const ok = await confirm({
      title: t.sessions.copyTitle(selected.size, who(target)),
      message: t.sessions.copyMessage,
      confirmLabel: t.common.copy,
    });
    if (!ok) return;
    setBusy(true);
    try {
      const result = await importCloudSessions(account.id, target.id, [...selected]);
      toast(t.sessions.copied(result.length, who(target)));
      setSelected(new Set());
    } catch (e) {
      toast(t.sessions.copyFailed(errorText(t, e)), "bad");
    } finally {
      setBusy(false);
    }
  };

  if (accounts.length === 0) {
    return (
      <div className="page">
        <header className="pageHead">
          <div>
            <h1>{t.sessions.title}</h1>
          </div>
        </header>
        <Empty icon="chat" title={t.common.noAccounts} />
      </div>
    );
  }

  return (
    <div className="page">
      <header className="pageHead">
        <div>
          <h1>{t.sessions.title}</h1>
          <p className="pageSub">{t.sessions.sub}</p>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading || busy}>
          <Icon name="refresh" className={loading ? "spin" : ""} />
          {t.common.refresh}
        </button>
      </header>

      <div className="toolbar">
        <select className="select wide" value={accountId} onChange={(e) => setAccountId(e.target.value)} disabled={busy} aria-label={t.sessions.account}>
          {accounts.map((a) => (
            <option key={a.id} value={a.id}>
              {who(a)}
              {a.active ? t.sessions.currentSuffix : ""}
            </option>
          ))}
        </select>
        <Segmented<Source>
          value={source}
          onChange={setSource}
          disabled={busy}
          options={[
            { value: "cloud", label: t.sessions.cloud, title: t.sessions.cloudTitle },
            { value: "local", label: t.sessions.local, title: t.sessions.localTitle },
          ]}
        />
        <div className="search">
          <Icon name="search" size={15} />
          <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t.sessions.searchPlaceholder} />
        </div>
      </div>

      {error && <Alert tone="bad">{error}</Alert>}

      {account && !account.on_desktop && source === "local" && (
        <Alert tone="info">{t.sessions.notOnDesktop}</Alert>
      )}

      {sessions && sessions.length > 0 && (
        <div className="actionBar">
          <label className="checkLabel">
            <input type="checkbox" checked={allSelected} onChange={toggleAll} />
            {selected.size > 0 ? t.sessions.selected(selected.size) : t.sessions.total(visible.length)}
          </label>
          <span className="spacer" />
          <Segmented<"md" | "json">
            value={format}
            onChange={setFormat}
            disabled={busy}
            options={[
              { value: "md", label: "Markdown" },
              { value: "json", label: "JSON" },
            ]}
          />
          <button className="btn sm" onClick={() => runExport([...selected])} disabled={busy || selected.size === 0}>
            <Icon name="download" size={14} />
            {t.sessions.exportSelected}
          </button>
          <button className="btn ghost sm" onClick={() => runExport([])} disabled={busy}>
            {t.sessions.exportAll}
          </button>
          {source === "cloud" && targets.length > 0 && (
            <>
              <span className="sep" />
              <span className="muted">{t.sessions.copyTo}</span>
              <select className="select sm" value={targetId} onChange={(e) => setTargetId(e.target.value)} disabled={busy} aria-label={t.sessions.targetAccount}>
                {targets.map((a) => (
                  <option key={a.id} value={a.id}>
                    {who(a)}
                  </option>
                ))}
              </select>
              <button className="btn primary sm" onClick={runImport} disabled={busy || selected.size === 0 || !targetId}>
                {busy ? <Spinner /> : <Icon name="copy" size={14} />}
                {t.common.copy}
              </button>
            </>
          )}
        </div>
      )}

      {loading && !sessions && (
        <ul className="rows">
          {[0, 1, 2, 3].map((i) => (
            <li key={i} className="row">
              <div className="skeleton" />
            </li>
          ))}
        </ul>
      )}

      {!loading && sessions?.length === 0 && !error && (
        <Empty icon="chat" title={source === "cloud" ? t.sessions.emptyCloud : t.sessions.emptyLocal} />
      )}

      {sessions && sessions.length > 0 && visible.length === 0 && <Empty icon="search" title={t.sessions.noMatch} />}

      {visible.length > 0 && (
        <ul className={cx("rows", loading && "isLoading")}>
          {visible.map((s) => (
            <li key={s.id} className={cx("row", selected.has(s.id) && "isSelected")} onClick={() => toggle(s.id)}>
              <input
                type="checkbox"
                checked={selected.has(s.id)}
                onChange={() => toggle(s.id)}
                onClick={(e) => e.stopPropagation()}
                aria-label={t.sessions.selectTitle(s.title)}
              />
              <div className="rowMain">
                <div className="rowTitle">
                  <span className="truncate">{s.title || t.sessions.untitled}</span>
                  {s.archived && <span className="tag">{t.sessions.archived}</span>}
                  {!s.cloud && (s.server_session_id ? <span className="tag ok">{t.sessions.synced}</span> : <span className="tag warn">{t.sessions.localOnly}</span>)}
                </div>
                <div className="rowSub mono truncate">{s.id}</div>
              </div>
              <div className="rowMeta">
                <span>{t.sessions.messages(s.message_count)}</span>
                <span title={fmtDate(s.updated_at)}>{t.sessions.updated(fmtRelative(s.updated_at, t))}</span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
