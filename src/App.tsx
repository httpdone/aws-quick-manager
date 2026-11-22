import { useCallback, useEffect, useState } from "react";
import { Account, listAccounts } from "./lib/api";
import "./App.css";

const fmtDate = (secs: number) => (secs ? new Date(secs * 1000).toLocaleString() : "—");

function status(a: Account): { label: string; tone: "ok" | "warn" | "bad" } {
  if (a.error) return { label: "凭据读取失败", tone: "bad" };
  if (!a.has_credentials) return { label: "未登录", tone: "bad" };
  if (a.refresh_failing_since) return { label: "续期失败", tone: "bad" };
  const now = Date.now() / 1000;
  if (a.refresh_expires_at && a.refresh_expires_at < now) return { label: "登录已过期", tone: "bad" };
  if (a.access_expires_at && a.access_expires_at < now) return { label: "待续期", tone: "warn" };
  return { label: "正常", tone: "ok" };
}

function AccountRow({ a }: { a: Account }) {
  const s = status(a);
  return (
    <li className={a.active ? "row active" : "row"}>
      <div className="who">
        <div className="name">
          {a.display_name || a.id}
          {a.active && <span className="badge">当前</span>}
        </div>
        <div className="sub">{a.email || a.id}</div>
      </div>
      <div className="meta">
        <span className={`pill ${s.tone}`}>{s.label}</span>
        <span className="dim">{a.role || "—"}</span>
      </div>
      <div className="meta dim" title="登录有效期（refresh token 过期时间）">
        登录有效至 {fmtDate(a.refresh_expires_at)}
      </div>
    </li>
  );
}

export default function App() {
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      setAccounts(await listAccounts());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <main>
      <header>
        <h1>Amazon Quick 账号</h1>
        <button onClick={refresh} disabled={loading}>
          {loading ? "读取中…" : "刷新"}
        </button>
      </header>
      {error && <p className="error">{error}</p>}
      {accounts?.length === 0 && <p className="empty">桌面端还没有登录过任何账号。</p>}
      <ul className="list">
        {accounts?.map((a) => <AccountRow key={a.id} a={a} />)}
      </ul>
    </main>
  );
}
