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
      setError(String(e));
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
      toast(`已导出 ${result.sessions} 个会话，${count} 条${source === "cloud" ? "消息" : "事件"}`, "ok", {
        label: "在文件夹中显示",
        run: () => revealItemInDir(result.path).catch((e) => toast(String(e), "bad")),
      });
    } catch (e) {
      toast(`导出失败：${String(e)}`, "bad");
    } finally {
      setBusy(false);
    }
  };

  const runImport = async () => {
    const target = accounts.find((a) => a.id === targetId);
    if (!account || !target || selected.size === 0) return;
    const ok = await confirm({
      title: `复制 ${selected.size} 个会话到 ${who(target)}`,
      message: "会在目标账号新建会话，原会话不变。\n附件、工作区文件和工具产物不会跟着复制。",
      confirmLabel: "复制",
    });
    if (!ok) return;
    setBusy(true);
    try {
      const result = await importCloudSessions(account.id, target.id, [...selected]);
      toast(`已复制 ${result.length} 个会话到 ${who(target)}`);
      setSelected(new Set());
    } catch (e) {
      toast(`复制失败：${String(e)}`, "bad");
    } finally {
      setBusy(false);
    }
  };

  if (accounts.length === 0) {
    return (
      <div className="page">
        <header className="pageHead">
          <div>
            <h1>会话</h1>
          </div>
        </header>
        <Empty icon="chat" title="还没有账号" />
      </div>
    );
  }

  return (
    <div className="page">
      <header className="pageHead">
        <div>
          <h1>会话</h1>
          <p className="pageSub">查看、导出账号的会话，或复制到另一个账号</p>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading || busy}>
          <Icon name="refresh" className={loading ? "spin" : ""} />
          刷新
        </button>
      </header>

      <div className="toolbar">
        <select className="select wide" value={accountId} onChange={(e) => setAccountId(e.target.value)} disabled={busy} aria-label="账号">
          {accounts.map((a) => (
            <option key={a.id} value={a.id}>
              {who(a)}
              {a.active ? "（当前）" : ""}
            </option>
          ))}
        </select>
        <Segmented<Source>
          value={source}
          onChange={setSource}
          disabled={busy}
          options={[
            { value: "cloud", label: "云端", title: "服务器上保存的会话" },
            { value: "local", label: "本地", title: "这台电脑上 Quick 的会话库" },
          ]}
        />
        <div className="search">
          <Icon name="search" size={15} />
          <input type="search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="搜索标题或 ID" />
        </div>
      </div>

      {error && <Alert tone="bad">{error}</Alert>}

      {account && !account.on_desktop && source === "local" && (
        <Alert tone="info">这个账号还没写进 Quick，没有本地会话库。切换一次后再来看。</Alert>
      )}

      {sessions && sessions.length > 0 && (
        <div className="actionBar">
          <label className="checkLabel">
            <input type="checkbox" checked={allSelected} onChange={toggleAll} />
            {selected.size > 0 ? `已选 ${selected.size} 个` : `共 ${visible.length} 个`}
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
            导出选中
          </button>
          <button className="btn ghost sm" onClick={() => runExport([])} disabled={busy}>
            导出全部
          </button>
          {source === "cloud" && targets.length > 0 && (
            <>
              <span className="sep" />
              <span className="muted">复制到</span>
              <select className="select sm" value={targetId} onChange={(e) => setTargetId(e.target.value)} disabled={busy} aria-label="目标账号">
                {targets.map((a) => (
                  <option key={a.id} value={a.id}>
                    {who(a)}
                  </option>
                ))}
              </select>
              <button className="btn primary sm" onClick={runImport} disabled={busy || selected.size === 0 || !targetId}>
                {busy ? <Spinner /> : <Icon name="copy" size={14} />}
                复制
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
        <Empty icon="chat" title={source === "cloud" ? "云端没有会话" : "本地会话库里没有会话"} />
      )}

      {sessions && sessions.length > 0 && visible.length === 0 && <Empty icon="search" title="没有匹配的会话" />}

      {visible.length > 0 && (
        <ul className={cx("rows", loading && "isLoading")}>
          {visible.map((s) => (
            <li key={s.id} className={cx("row", selected.has(s.id) && "isSelected")} onClick={() => toggle(s.id)}>
              <input
                type="checkbox"
                checked={selected.has(s.id)}
                onChange={() => toggle(s.id)}
                onClick={(e) => e.stopPropagation()}
                aria-label={`选择 ${s.title}`}
              />
              <div className="rowMain">
                <div className="rowTitle">
                  <span className="truncate">{s.title || "（无标题）"}</span>
                  {s.archived && <span className="tag">已归档</span>}
                  {!s.cloud && (s.server_session_id ? <span className="tag ok">已同步云端</span> : <span className="tag warn">仅本地</span>)}
                </div>
                <div className="rowSub mono truncate">{s.id}</div>
              </div>
              <div className="rowMeta">
                <span>{s.message_count} 条消息</span>
                <span title={fmtDate(s.updated_at)}>{fmtRelative(s.updated_at)}更新</span>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
