import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  Account,
  CloudSession,
  SessionInfo,
  deleteCloudSessions,
  exportCloudSessions,
  exportSessions,
  importCloudSessions,
  listCloudSessionsPage,
  listSessions,
} from "../lib/api";
import { useI18n } from "../i18n";
import { errorText } from "../lib/errors";
import { fmtRelative, fmtDate, hue } from "../lib/format";
import { getStored } from "../lib/prefs";
import { who } from "../lib/status";
import Icon from "../components/Icon";
import SessionPreview, { clearSessionCache } from "../components/SessionPreview";
import { Alert, Avatar, Empty, Segmented, Spinner, cx, useUI } from "../components/ui";

interface Props {
  accounts: Account[];
}

type Source = "cloud" | "local";
const SOURCES = ["cloud", "local"] as const;
const LIST_CACHE_MS = 60_000;
/** cloud sessions fetched per page */
const PAGE_SIZE = 10;
/** rows shown per step while scrolling (local sessions are read at once) */
const RENDER_STEP = 10;

interface ListState {
  rows: Row[];
  next: string | null;
  at: number;
}

/** Placeholder rows shaped like the real ones, while the list loads. */
const SKELETON_WIDTHS = [62, 48, 71, 39, 55];

function ListSkeleton({ text }: { text: string }) {
  return (
    <div className="rows listSkeleton" aria-busy="true">
      <div className="listSkeletonStatus" role="status">
        <Spinner />
        <span>{text}</span>
      </div>
      <ul aria-hidden="true">
        {SKELETON_WIDTHS.map((w, i) => (
          <li key={i} className="row skeletonRow" style={{ animationDelay: `${i * 70}ms` }}>
            <span className="skeleton box" />
            <div className="rowMain">
              <span className="skeleton" style={{ width: `${w}%` }} />
              <span className="skeleton thin" style={{ width: `${Math.max(28, w - 18)}%` }} />
            </div>
            <div className="rowMeta">
              <span className="skeleton thin" style={{ width: 54 }} />
              <span className="skeleton thin" style={{ width: 76 }} />
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

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
  const { t, lang } = useI18n();
  const { toast, confirm } = useUI();
  const [source, setSource] = useState<Source>(() => getStored("aqm_sessions_tab", SOURCES, "cloud"));
  const [accountId, setAccountId] = useState("");
  const [targetId, setTargetId] = useState("");
  const [sessions, setSessions] = useState<Row[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [format, setFormat] = useState<"md" | "json">("md");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<"" | "export" | "import" | "delete">("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [preview, setPreview] = useState<Row | null>(null);
  const listCache = useRef(new Map<string, ListState>());
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [moreError, setMoreError] = useState("");
  const [shown, setShown] = useState(RENDER_STEP);
  /** bumped per load, so a late answer for another account is dropped */
  const loadSeq = useRef(0);
  const sentinel = useRef<HTMLLIElement | null>(null);

  useEffect(() => localStorage.setItem("aqm_sessions_tab", source), [source]);

  const account = useMemo(() => accounts.find((a) => a.id === accountId) ?? null, [accounts, accountId]);
  const target = useMemo(() => accounts.find((a) => a.id === targetId) ?? null, [accounts, targetId]);
  const targets = accounts.filter((a) => a.id !== accountId);
  const cloud = source === "cloud";

  useEffect(() => {
    if (accountId && accounts.some((a) => a.id === accountId)) return;
    const preferred = accounts.find((a) => a.active) ?? accounts[0];
    setAccountId(preferred?.id ?? "");
  }, [accounts, accountId]);

  useEffect(() => {
    if (targetId && targetId !== accountId && accounts.some((a) => a.id === targetId)) return;
    setTargetId(accounts.find((a) => a.id !== accountId)?.id ?? "");
  }, [accounts, accountId, targetId]);

  const load = useCallback(async (force = false) => {
    const seq = ++loadSeq.current;
    setShown(RENDER_STEP);
    setMoreError("");
    setLoadingMore(false);
    if (!accountId) {
      setSessions([]);
      setNextCursor(null);
      return;
    }
    setSelected(new Set());
    const key = `${source}:${accountId}`;
    const hit = listCache.current.get(key);
    if (!force && hit && Date.now() - hit.at < LIST_CACHE_MS) {
      setSessions(hit.rows);
      setNextCursor(hit.next);
      setError("");
      setLoading(false);
      return;
    }
    setLoading(true);
    setError("");
    try {
      let rows: Row[];
      let next: string | null = null;
      if (source === "cloud") {
        const page = await listCloudSessionsPage(accountId, null, PAGE_SIZE);
        rows = page.sessions.map((s: CloudSession) => ({ ...s, cloud: true }));
        next = page.next_cursor;
      } else {
        rows = (await listSessions(accountId)).map((s: SessionInfo) => ({ ...s, cloud: false }));
      }
      if (seq !== loadSeq.current) return;
      listCache.current.set(key, { rows, next, at: Date.now() });
      setSessions(rows);
      setNextCursor(next);
    } catch (e) {
      if (seq !== loadSeq.current) return;
      setSessions([]);
      setNextCursor(null);
      setError(errorText(t, e));
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [accountId, source]);

  /** The next page of cloud sessions, appended. */
  const loadMore = useCallback(async () => {
    if (source !== "cloud" || !accountId || !nextCursor || loadingMore) return;
    const seq = loadSeq.current;
    const key = `cloud:${accountId}`;
    setLoadingMore(true);
    setMoreError("");
    try {
      const page = await listCloudSessionsPage(accountId, nextCursor, PAGE_SIZE);
      if (seq !== loadSeq.current) return;
      const next = page.next_cursor && page.next_cursor !== nextCursor ? page.next_cursor : null;
      setSessions((prev) => {
        const have = new Set((prev ?? []).map((s) => s.id));
        const rows = [...(prev ?? []), ...page.sessions.filter((s) => !have.has(s.id)).map((s) => ({ ...s, cloud: true }))];
        listCache.current.set(key, { rows, next, at: Date.now() });
        return rows;
      });
      setNextCursor(next);
    } catch (e) {
      if (seq === loadSeq.current) setMoreError(errorText(t, e));
    } finally {
      if (seq === loadSeq.current) setLoadingMore(false);
    }
  }, [source, accountId, nextCursor, loadingMore, t]);

  useEffect(() => {
    setPreview(null);
    setSessions(null);
    load();
  }, [load]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = (sessions ?? []).filter((s) => !q || `${s.title} ${s.id}`.toLowerCase().includes(q));
    return rows.sort((a, b) => b.updated_at - a.updated_at);
  }, [sessions, query]);

  useEffect(() => setShown(RENDER_STEP), [query]);

  const rendered = visible.slice(0, shown);
  const moreToShow = shown < visible.length || (cloud && !!nextCursor);

  // the footer row coming into view shows more rows, then fetches the next page
  const more = useRef<() => void>(() => {});
  more.current = () => {
    if (shown < visible.length) setShown((n) => n + RENDER_STEP);
    else if (cloud && nextCursor && !moreError) void loadMore();
  };
  useEffect(() => {
    const el = sentinel.current;
    if (!el || !moreToShow || loadingMore) return;
    const io = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && more.current(), { rootMargin: "0px 0px 120px 0px" });
    io.observe(el);
    return () => io.disconnect();
  }, [moreToShow, loadingMore, rendered.length]);

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

  const swap = () => {
    if (!target) return;
    const from = accountId;
    setAccountId(target.id);
    setTargetId(from);
  };

  const runExport = async (ids: string[]) => {
    if (!account) return;
    setBusy("export");
    try {
      const result = cloud ? await exportCloudSessions(account.id, ids, format, lang) : await exportSessions(account.id, ids, format, lang);
      const count = "events" in result ? result.events : result.messages;
      toast(t.sessions.exported(result.sessions, count, cloud), "ok", {
        label: t.common.revealInFolder,
        run: () => revealItemInDir(result.path).catch((e) => toast(errorText(t, e), "bad")),
      });
      if ("truncated" in result && result.truncated.length > 0) toast(t.sessions.truncated(result.truncated.length), "info");
    } catch (e) {
      toast(t.sessions.exportFailed(errorText(t, e)), "bad");
    } finally {
      setBusy("");
    }
  };

  const runImport = async () => {
    if (!account || !target || selected.size === 0) return;
    const ok = await confirm({
      title: t.sessions.copyTitle(selected.size, who(target)),
      message: t.sessions.copyMessage,
      confirmLabel: t.sessions.transferConfirm,
    });
    if (!ok) return;
    setBusy("import");
    try {
      const result = await importCloudSessions(account.id, target.id, [...selected]);
      listCache.current.delete(`cloud:${target.id}`);
      toast(t.sessions.copied(result.length, who(target)));
      setSelected(new Set());
    } catch (e) {
      toast(t.sessions.copyFailed(errorText(t, e)), "bad");
    } finally {
      setBusy("");
    }
  };

  const removeSessions = async (ids: string[]) => {
    if (!account || !cloud || ids.length === 0) return;
    const titles = sessions?.filter((s) => ids.includes(s.id)).map((s) => s.title || s.id) ?? [];
    const ok = await confirm({
      title: ids.length === 1 ? t.sessions.deleteTitle(titles[0] ?? ids[0]) : t.sessions.deleteTitleCount(ids.length),
      message: t.sessions.deleteMessage,
      confirmLabel: t.common.delete,
      danger: true,
    });
    if (!ok) return;
    setBusy("delete");
    try {
      const result = await deleteCloudSessions(account.id, ids);
      const done = result.filter((r) => r.success).map((r) => r.session_id);
      const failed = result.filter((r) => !r.success);
      if (done.length > 0) {
        const gone = new Set(done);
        setSessions((prev) => {
          const next = prev?.filter((s) => !gone.has(s.id)) ?? null;
          if (next) listCache.current.set(`cloud:${account.id}`, { rows: next, next: nextCursor, at: Date.now() });
          return next;
        });
        clearSessionCache(account.id, done.length === 1 ? done[0] : undefined);
        if (preview && gone.has(preview.id)) setPreview(null);
        setSelected((prev) => new Set([...prev].filter((id) => !gone.has(id))));
        toast(t.sessions.deleted(done.length));
      }
      if (failed.length > 0) {
        toast(t.sessions.deleteFailed(failed.map((r) => (r.error ? errorText(t, r.error) : r.session_id)).join(t.common.joinSep)), "bad");
      }
    } catch (e) {
      toast(t.sessions.deleteFailed(errorText(t, e)), "bad");
    } finally {
      setBusy("");
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

  const accountOptions = (list: Account[]) =>
    list.map((a) => (
      <option key={a.id} value={a.id}>
        {who(a)}
        {a.active ? t.sessions.currentSuffix : ""}
      </option>
    ));

  return (
    <div className="page">
      <header className="pageHead">
        <div>
          <h1>{t.sessions.title}</h1>
          <p className="pageSub">{t.sessions.sub}</p>
        </div>
        <button className="btn ghost" onClick={() => load(true)} disabled={loading || !!busy}>
          <Icon name="refresh" className={loading ? "spin" : ""} />
          {t.common.refresh}
        </button>
      </header>

      <div className="sessionTabs" role="tablist">
        <button
          role="tab"
          aria-selected={cloud}
          className={cx("sessionTab", "primaryTab", cloud && "on")}
          onClick={() => setSource("cloud")}
          disabled={!!busy}
        >
          <span className="sessionTabIcon">
            <Icon name="swap" size={18} />
          </span>
          <span className="sessionTabText">
            <b>{t.sessions.tabCloud}</b>
            <small>{t.sessions.tabCloudDesc}</small>
          </span>
        </button>
        <button
          role="tab"
          aria-selected={!cloud}
          className={cx("sessionTab", !cloud && "on")}
          onClick={() => setSource("local")}
          disabled={!!busy}
        >
          <span className="sessionTabIcon">
            <Icon name="monitor" size={18} />
          </span>
          <span className="sessionTabText">
            <b>{t.sessions.tabLocal}</b>
            <small>{t.sessions.tabLocalDesc}</small>
          </span>
        </button>
      </div>

      {cloud ? (
        <section className="transfer">
          <div className="transferSide">
            <span className="transferLabel">{t.sessions.from}</span>
            <div className="transferPick">
              {account && <Avatar name={who(account)} hue={hue(account.id)} size={28} />}
              <select className="select" value={accountId} onChange={(e) => setAccountId(e.target.value)} disabled={!!busy} aria-label={t.sessions.from}>
                {accountOptions(accounts)}
              </select>
            </div>
          </div>
          <button className="transferSwap" onClick={swap} disabled={!!busy || !target} title={t.sessions.swap} aria-label={t.sessions.swap}>
            <Icon name="arrowRight" size={18} />
          </button>
          <div className="transferSide">
            <span className="transferLabel">{t.sessions.to}</span>
            <div className="transferPick">
              {target && <Avatar name={who(target)} hue={hue(target.id)} size={28} />}
              <select
                className="select"
                value={targetId}
                onChange={(e) => setTargetId(e.target.value)}
                disabled={!!busy || targets.length === 0}
                aria-label={t.sessions.to}
              >
                {targets.length === 0 ? <option value="">{t.sessions.noTarget}</option> : accountOptions(targets)}
              </select>
            </div>
          </div>
          <button className="btn primary transferGo" onClick={runImport} disabled={!!busy || selected.size === 0 || !target}>
            {busy === "import" ? <Spinner /> : <Icon name="swap" size={15} />}
            {busy === "import" ? t.sessions.transferring : t.sessions.transfer(selected.size)}
          </button>
          <p className="transferHint">
            {targets.length === 0 ? t.sessions.needTwo : selected.size === 0 ? t.sessions.pickHint : t.sessions.copyMessage.split("\n")[0]}
          </p>
        </section>
      ) : (
        <div className="toolbar">
          <select className="select wide" value={accountId} onChange={(e) => setAccountId(e.target.value)} disabled={!!busy} aria-label={t.sessions.account}>
            {accountOptions(accounts)}
          </select>
        </div>
      )}

      {error && <Alert tone="bad">{error}</Alert>}

      {account && !account.on_desktop && !cloud && <Alert tone="info">{t.sessions.notOnDesktop}</Alert>}

      {sessions && sessions.length > 0 && (
        <div className="listBar">
          <label className="checkLabel">
            <input type="checkbox" checked={allSelected} onChange={toggleAll} />
            {selected.size > 0
              ? t.sessions.selected(selected.size)
              : cloud && nextCursor
                ? t.sessions.totalMore(visible.length)
                : t.sessions.total(visible.length)}
          </label>
          <div className="search sm">
            <Icon name="search" size={14} />
            <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t.sessions.searchPlaceholder} />
          </div>
          <div className="exportGroup">
            <Segmented<"md" | "json">
              value={format}
              onChange={setFormat}
              disabled={!!busy}
              options={[
                { value: "md", label: "MD" },
                { value: "json", label: "JSON" },
              ]}
            />
            <button className="btn ghost sm" onClick={() => runExport([...selected])} disabled={!!busy || selected.size === 0}>
              {busy === "export" ? <Spinner /> : <Icon name="download" size={14} />}
              {t.sessions.exportSelected}
            </button>
            <button className="btn ghost sm" onClick={() => runExport([])} disabled={!!busy}>
              {t.sessions.exportAll}
            </button>
            {cloud && (
              <button className="btn danger sm" onClick={() => removeSessions([...selected])} disabled={!!busy || selected.size === 0}>
                {busy === "delete" ? <Spinner /> : <Icon name="trash" size={14} />}
                {busy === "delete" ? t.sessions.deleting : t.sessions.deleteSelected}
              </button>
            )}
          </div>
        </div>
      )}

      {sessions === null && !error && accountId && <ListSkeleton text={cloud ? t.sessions.loadingCloud : t.sessions.loadingLocal} />}

      {!loading && sessions?.length === 0 && !error && <Empty icon="chat" title={cloud ? t.sessions.emptyCloud : t.sessions.emptyLocal} />}

      {query.trim() && cloud && nextCursor && <p className="listHint">{t.sessions.searchLoaded(sessions?.length ?? 0)}</p>}

      {sessions && sessions.length > 0 && visible.length === 0 && !nextCursor && <Empty icon="search" title={t.sessions.noMatch} />}

      {(visible.length > 0 || (cloud && nextCursor && sessions && sessions.length > 0)) && (
        <ul className={cx("rows", loading && "isLoading")}>
          {rendered.map((s) => (
            <li
              key={s.id}
              className={cx("row", selected.has(s.id) && "isSelected")}
              onClick={() => setPreview(s)}
              title={t.sessions.previewHint}
            >
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
                  {!s.cloud &&
                    (s.server_session_id ? <span className="tag ok">{t.sessions.synced}</span> : <span className="tag warn">{t.sessions.localOnly}</span>)}
                </div>
                <div className="rowSub mono truncate">{s.id}</div>
              </div>
              <div className="rowMeta">
                <span>{t.sessions.messages(s.message_count)}</span>
                <span title={fmtDate(s.updated_at)}>{t.sessions.updated(fmtRelative(s.updated_at, t))}</span>
                {cloud && (
                  <button
                    className="iconBtn xs"
                    onClick={(e) => {
                      e.stopPropagation();
                      removeSessions([s.id]);
                    }}
                    disabled={!!busy}
                    title={t.common.delete}
                    aria-label={`${t.common.delete} ${s.title || s.id}`}
                  >
                    <Icon name="trash" size={14} />
                  </button>
                )}
              </div>
            </li>
          ))}
          {(moreToShow || loadingMore || moreError || (cloud && (sessions?.length ?? 0) > PAGE_SIZE)) && (
            <li ref={sentinel} className="rowsFooter" aria-live="polite">
              {loadingMore ? (
                <>
                  <Spinner />
                  {t.sessions.loadingMore}
                </>
              ) : moreError ? (
                <>
                  <span className="dangerText">{t.sessions.loadMoreFailed(moreError)}</span>
                  <button
                    className="btn ghost sm"
                    onClick={() => {
                      setMoreError("");
                      void loadMore();
                    }}
                  >
                    {t.common.retry}
                  </button>
                </>
              ) : moreToShow ? (
                <button className="btn ghost sm" onClick={() => more.current()}>
                  {t.sessions.loadMore}
                </button>
              ) : (
                <span>{t.sessions.allLoaded(sessions?.length ?? 0)}</span>
              )}
            </li>
          )}
        </ul>
      )}

      {preview && account && (
        <SessionPreview
          accountId={account.id}
          source={cloud ? "cloud" : "local"}
          session={{ id: preview.id, title: preview.title }}
          onClose={() => setPreview(null)}
          onDelete={cloud ? (item) => removeSessions([item.id]) : undefined}
        />
      )}
    </div>
  );
}
