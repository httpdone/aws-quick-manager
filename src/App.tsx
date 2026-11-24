import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Account,
  AppInfo,
  DesktopStatus,
  ImportReport,
  SwitchReport,
  Usage,
  accountUsage,
  reloginAccount,
  appInfo,
  desktopStatus,
  listAccounts,
  syncFromDesktop,
} from "./lib/api";
import { useT } from "./i18n";
import type { Messages } from "./i18n";
import { errorText } from "./lib/errors";
import { canQuota, who } from "./lib/status";
import { REPO_URL, useUpdates } from "./lib/updates";
import Icon, { IconName } from "./components/Icon";
import { cx, useUI } from "./components/ui";
import AccountDetailsDialog from "./components/AccountDetailsDialog";
import AddAccountDialog from "./components/AddAccountDialog";
import DeleteAccountsDialog from "./components/DeleteAccountsDialog";
import SwitchDialog from "./components/SwitchDialog";
import UpdateDialog from "./components/UpdateDialog";
import AccountsPage from "./pages/AccountsPage";
import BackupsPage from "./pages/BackupsPage";
import DevicesPage from "./pages/DevicesPage";
import SessionsPage from "./pages/SessionsPage";
import SettingsPage from "./pages/SettingsPage";
import logo from "./assets/logo.svg";

type Page = "accounts" | "sessions" | "devices" | "backups" | "settings";

const PAGES: { id: Page; icon: IconName }[] = [
  { id: "accounts", icon: "users" },
  { id: "sessions", icon: "chat" },
  { id: "devices", icon: "cpu" },
  { id: "backups", icon: "archive" },
  { id: "settings", icon: "settings" },
];

const DESKTOP_POLL_MS = 15000;

/** Only news worth a toast: newly imported or skipped accounts. */
function syncNotice(r: ImportReport, t: Messages): string {
  const parts: string[] = [];
  if (r.imported.length) parts.push(t.app.imported(r.imported.length));
  if (r.skipped.length) parts.push(t.app.skipped(r.skipped.length));
  return parts.join(t.common.joinSep);
}

export default function App() {
  const { toast } = useUI();
  const t = useT();
  const tRef = useRef(t);
  tRef.current = t;
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
  const [relogging, setRelogging] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const updates = useUpdates();
  const [updateOpen, setUpdateOpen] = useState(false);
  const hasUpdate = !!updates.update;

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
          .catch((e) => setUsageErrors((prev) => ({ ...prev, [a.id]: errorText(tRef.current, e) })))
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
          const text = syncNotice(await syncFromDesktop(), tRef.current);
          if (text) toast(text, "info");
        } catch (e) {
          toast(tRef.current.app.syncFailed(errorText(tRef.current, e)), "bad");
        }
      }
      try {
        const [list, d] = await Promise.all([listAccounts(), desktopStatus().catch(() => null)]);
        setAccounts(list);
        if (d) setDesktop(d);
        void loadUsage(list);
      } catch (e) {
        setAccounts((prev) => prev ?? []);
        toast(errorText(tRef.current, e), "bad");
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

  // tray menu "refresh"
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
    const note = r.notes.map((n) => errorText(t, n)).join(t.common.joinSep);
    toast(note || (r.launched ? t.app.switchedLaunching(who(a)) : t.app.switched(who(a))), note ? "info" : "ok");
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

  const relogin = async (a: Account) => {
    setRelogging(a.id);
    toast(t.accounts.reloginStarted(who(a)), "info");
    try {
      const r = await reloginAccount(a.id);
      toast(t.accounts.relogged(r.email || r.display_name || who(a)));
      await refresh();
    } catch (e) {
      toast(errorText(t, e), "bad");
    } finally {
      setRelogging(null);
    }
  };

  const deleted = (message: string) => {
    setDeleteTargets(null);
    toast(message);
    refresh();
  };

  const desktopText = !desktop ? t.desktop.detecting : desktop.running ? t.desktop.running : desktop.installed ? t.desktop.stopped : t.desktop.notInstalled;

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <img className="logo" src={logo} alt="" width={30} height={30} />
          <div className="brandText">
            <b>Quick Manager</b>
            <small>{t.app.tagline}</small>
          </div>
        </div>

        <nav className="nav">
          {PAGES.map((p, i) => (
            <button key={p.id} className={cx("navItem", page === p.id && "on")} onClick={() => setPage(p.id)} title={t.app.navTitle(t.app.pages[p.id], i + 1)}>
              <Icon name={p.icon} size={17} />
              <span className="navLabel">{t.app.pages[p.id]}</span>
              {p.id === "accounts" && accounts && <em>{accounts.length}</em>}
            </button>
          ))}
        </nav>

        <div className="sideFoot">
          {current && (
            <button className="sideCurrent" onClick={() => setDetailsFor(current)} title={t.common.currentAccount}>
              <span className="sideLabel">{t.common.currentAccount}</span>
              <b className="truncate">{who(current)}</b>
            </button>
          )}
          <div className="deskStatus" title={desktop?.exe ?? undefined}>
            <span className={cx("dot", desktop?.running && "on", desktop && !desktop.installed && "bad")} />
            <span className="deskText">
              <span className="sideLabel">{t.desktop.label}</span>
              <b>{desktopText}</b>
            </span>
            <button className="iconBtn xs" onClick={refreshDesktop} title={t.common.redetect}>
              <Icon name="refresh" size={13} />
            </button>
          </div>
        </div>
      </aside>

      <main className="content">
        <div className="topbar">
          {updates.update && (
            <button className="updatePill" onClick={() => setUpdateOpen(true)} title={t.update.pillTitle}>
              <span className="pulse" />
              {t.update.pill(updates.update.version)}
            </button>
          )}
          <button
            className="ghBtn"
            onClick={() => (hasUpdate ? setUpdateOpen(true) : openUrl(REPO_URL).catch((e) => toast(errorText(t, e), "bad")))}
            title={hasUpdate ? t.update.githubUpdateTitle : t.update.githubTitle}
            aria-label="GitHub"
          >
            <Icon name="github" size={17} />
            <span className="ghLabel">GitHub</span>
            {hasUpdate && <i className="ghDot" />}
          </button>
        </div>
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
            onRelogin={relogin}
            relogging={relogging}
            onDetails={setDetailsFor}
            onDelete={setDeleteTargets}
          />
        )}
        {page === "sessions" && <SessionsPage accounts={accounts ?? []} />}
        {page === "devices" && <DevicesPage desktopRunning={!!desktop?.running} onChanged={() => refresh()} />}
        {page === "backups" && <BackupsPage desktopRunning={!!desktop?.running} onRestored={() => refresh()} />}
        {page === "settings" && <SettingsPage info={info} desktop={desktop} onRefreshDesktop={refreshDesktop} updates={updates} onOpenUpdate={() => setUpdateOpen(true)} />}
      </main>

      {updateOpen && updates.update && <UpdateDialog update={updates.update} onClose={() => setUpdateOpen(false)} />}
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
