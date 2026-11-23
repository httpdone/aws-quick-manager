import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Account,
  AppInfo,
  DesktopStatus,
  ImportReport,
  SwitchReport,
  Usage,
  accountUsage,
  appInfo,
  desktopStatus,
  listAccounts,
  syncFromDesktop,
} from "./lib/api";
import { canQuota, who } from "./lib/status";
import Icon, { IconName } from "./components/Icon";
import { cx, useUI } from "./components/ui";
import AccountDetailsDialog from "./components/AccountDetailsDialog";
import AddAccountDialog from "./components/AddAccountDialog";
import DeleteAccountsDialog from "./components/DeleteAccountsDialog";
import SwitchDialog from "./components/SwitchDialog";
import AccountsPage from "./pages/AccountsPage";
import BackupsPage from "./pages/BackupsPage";
import DevicesPage from "./pages/DevicesPage";
import SessionsPage from "./pages/SessionsPage";
import SettingsPage from "./pages/SettingsPage";

type Page = "accounts" | "sessions" | "devices" | "backups" | "settings";

const PAGES: { id: Page; label: string; icon: IconName }[] = [
  { id: "accounts", label: "账号", icon: "users" },
  { id: "sessions", label: "会话", icon: "chat" },
  { id: "devices", label: "设备 ID", icon: "cpu" },
  { id: "backups", label: "清单备份", icon: "archive" },
  { id: "settings", label: "设置", icon: "settings" },
];

const DESKTOP_POLL_MS = 15000;

/** Only news worth a toast: newly imported or skipped accounts. */
function syncNotice(r: ImportReport): string {
  const parts: string[] = [];
  if (r.imported.length) parts.push(`从 Quick 导入了 ${r.imported.length} 个新账号`);
  if (r.skipped.length) parts.push(`${r.skipped.length} 个账号读不到凭据，已跳过`);
  return parts.join("；");
}

export default function App() {
  const { toast } = useUI();
  const [page, setPage] = useState<Page>("accounts");
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  const [usages, setUsages] = useState<Record<string, Usage>>({});
  const [usageErrors, setUsageErrors] = useState<Record<string, string>>({});
  const [usageLoading, setUsageLoading] = useState<Set<string>>(new Set());
  const [desktop, setDesktop] = useState<DesktopStatus | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [loading, setLoading] = useState(false);

  const [addOpen, setAddOpen] = useState(false);
  const [switchTarget, setSwitchTarget] = useState<Account | null>(null);
  const [detailsFor, setDetailsFor] = useState<Account | null>(null);
  const [deleteTargets, setDeleteTargets] = useState<Account[] | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const refreshDesktop = useCallback(() => {
    desktopStatus()
      .then(setDesktop)
      .catch(() => {});
  }, []);

  /** Read quota for some accounts; earlier values stay on screen meanwhile. */
  const loadUsage = useCallback(async (list: Account[], only?: string[]) => {
    const targets = list.filter((a) => canQuota(a) && (!only || only.includes(a.id)));
    if (targets.length === 0) return;
    setUsageLoading((prev) => new Set([...prev, ...targets.map((a) => a.id)]));
    await Promise.allSettled(
      targets.map((a) =>
        accountUsage(a.id)
          .then((u) => {
            setUsages((prev) => ({ ...prev, [a.id]: u }));
            setUsageErrors((prev) => {
              if (!(a.id in prev)) return prev;
              const next = { ...prev };
              delete next[a.id];
              return next;
            });
          })
          .catch((e) => setUsageErrors((prev) => ({ ...prev, [a.id]: String(e) })))
          .finally(() =>
            setUsageLoading((prev) => {
              const next = new Set(prev);
              next.delete(a.id);
              return next;
            }),
          ),
      ),
    );
    // a quota read may have refreshed the access token; pick up the new expiry
    listAccounts()
      .then(setAccounts)
      .catch(() => {});
  }, []);

  const refresh = useCallback(
    async (sync = false) => {
      setLoading(true);
      if (sync) {
        try {
          const text = syncNotice(await syncFromDesktop());
          if (text) toast(text, "info");
        } catch (e) {
          toast(`从 Quick 同步账号失败：${String(e)}`, "bad");
        }
      }
      try {
        const [list, d] = await Promise.all([listAccounts(), desktopStatus().catch(() => null)]);
        setAccounts(list);
        if (d) setDesktop(d);
        void loadUsage(list);
      } catch (e) {
        setAccounts((prev) => prev ?? []);
        toast(String(e), "bad");
      } finally {
        setLoading(false);
      }
    },
    [loadUsage, toast],
  );

  const reloadUsage = useCallback(
    (ids: string[]) => {
      if (accounts) void loadUsage(accounts, ids);
    },
    [accounts, loadUsage],
  );

  // first load
  useEffect(() => {
    refresh(true);
    appInfo()
      .then(setInfo)
      .catch(() => {});
  }, [refresh]);

  // tray menu "刷新账号"
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    listen("tray://refresh", () => refresh(true))
      .then((fn) => {
        if (alive) unlisten = fn;
        else fn();
      })
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [refresh]);

  // keep the Quick status current while the window is in view
  useEffect(() => {
    const tick = () => {
      if (document.visibilityState === "visible") refreshDesktop();
    };
    const timer = window.setInterval(tick, DESKTOP_POLL_MS);
    window.addEventListener("focus", tick);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", tick);
    };
  }, [refreshDesktop]);

  // keyboard shortcuts
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing = !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable);
      const mod = e.ctrlKey || e.metaKey;
      const key = e.key.toLowerCase();
      if (e.key === "F5" || (mod && key === "r")) {
        e.preventDefault();
        refresh(true);
        return;
      }
      if (document.querySelector(".overlay")) return;
      if (mod && key === "n") {
        e.preventDefault();
        setAddOpen(true);
      } else if (mod && /^[1-5]$/.test(e.key)) {
        e.preventDefault();
        setPage(PAGES[Number(e.key) - 1].id);
      } else if (!typing && !mod && e.key === "/") {
        e.preventDefault();
        setPage("accounts");
        window.setTimeout(() => searchRef.current?.focus(), 0);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [refresh]);

  const current = accounts?.find((a) => a.active);

  const switched = (r: SwitchReport, a: Account) => {
    setSwitchTarget(null);
    toast(r.note ?? (r.launched ? `已切换到 ${who(a)}，Quick 正在启动` : `已切换到 ${who(a)}`), r.note ? "info" : "ok");
    refresh();
  };

  const added = useCallback(
    (message: string) => {
      setAddOpen(false);
      toast(message);
      refresh();
    },
    [refresh, toast],
  );

  const deleted = (message: string) => {
    setDeleteTargets(null);
    toast(message);
    refresh();
  };

  const desktopText = !desktop ? "正在检测…" : desktop.running ? "运行中" : desktop.installed ? "未运行" : "没找到安装";

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <span className="logo">
            <Icon name="zap" size={16} />
          </span>
          <div className="brandText">
            <b>Quick Manager</b>
            <small>AWS Quick 账号管理</small>
          </div>
        </div>

        <nav className="nav">
          {PAGES.map((p, i) => (
            <button key={p.id} className={cx("navItem", page === p.id && "on")} onClick={() => setPage(p.id)} title={`${p.label}（Ctrl+${i + 1}）`}>
              <Icon name={p.icon} size={17} />
              <span className="navLabel">{p.label}</span>
              {p.id === "accounts" && accounts && <em>{accounts.length}</em>}
            </button>
          ))}
        </nav>

        <div className="sideFoot">
          {current && (
            <button className="sideCurrent" onClick={() => setDetailsFor(current)} title="当前账号">
              <span className="sideLabel">当前账号</span>
              <b className="truncate">{who(current)}</b>
            </button>
          )}
          <div className="deskStatus" title={desktop?.exe ?? undefined}>
            <span className={cx("dot", desktop?.running && "on", desktop && !desktop.installed && "bad")} />
            <span className="deskText">
              <span className="sideLabel">Quick 桌面端</span>
              <b>{desktopText}</b>
            </span>
            <button className="iconBtn xs" onClick={refreshDesktop} title="重新检测">
              <Icon name="refresh" size={13} />
            </button>
          </div>
        </div>
      </aside>

      <main className="content">
        {page === "accounts" && (
          <AccountsPage
            accounts={accounts}
            usages={usages}
            usageErrors={usageErrors}
            usageLoading={usageLoading}
            desktop={desktop}
            loading={loading}
            searchRef={searchRef}
            onRefresh={() => refresh(true)}
            onReloadUsage={reloadUsage}
            onAdd={() => setAddOpen(true)}
            onSwitch={setSwitchTarget}
            onDetails={setDetailsFor}
            onDelete={setDeleteTargets}
          />
        )}
        {page === "sessions" && <SessionsPage accounts={accounts ?? []} />}
        {page === "devices" && <DevicesPage desktopRunning={!!desktop?.running} onChanged={() => refresh()} />}
        {page === "backups" && <BackupsPage desktopRunning={!!desktop?.running} onRestored={() => refresh()} />}
        {page === "settings" && <SettingsPage info={info} desktop={desktop} onRefreshDesktop={refreshDesktop} />}
      </main>

      {addOpen && <AddAccountDialog accounts={accounts ?? []} onClose={() => setAddOpen(false)} onDone={added} />}
      {detailsFor && (
        <AccountDetailsDialog
          account={detailsFor}
          onClose={() => setDetailsFor(null)}
          onSwitch={(a) => {
            setDetailsFor(null);
            setSwitchTarget(a);
          }}
          onDelete={(a) => {
            setDetailsFor(null);
            setDeleteTargets([a]);
          }}
        />
      )}
      {switchTarget && (
        <SwitchDialog
          target={switchTarget}
          current={current}
          desktop={desktop}
          onClose={() => {
            setSwitchTarget(null);
            refreshDesktop();
          }}
          onDone={switched}
        />
      )}
      {deleteTargets && deleteTargets.length > 0 && (
        <DeleteAccountsDialog
          accounts={deleteTargets}
          desktopRunning={!!desktop?.running}
          onClose={() => setDeleteTargets(null)}
          onDone={deleted}
        />
      )}
    </div>
  );
}
