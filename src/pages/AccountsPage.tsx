import { RefObject, useEffect, useMemo, useState } from "react";
import { useT } from "../i18n";
import { Account, DesktopStatus, Usage } from "../lib/api";
import { fmtDate, fmtHours, fmtRelative, fmtTime, hue, isoSecs, nextResetLabel } from "../lib/format";
import { getStored } from "../lib/prefs";
import { Tone, accountStatus, canQuota, canSwitch, leftHours, who } from "../lib/status";
import Icon from "../components/Icon";
import { Alert, Avatar, Empty, Segmented, StatusPill, cx } from "../components/ui";

type Filter = "all" | "ok" | "attention" | "vault";
type Sort = "default" | "quota" | "recent" | "name";
type View = "list" | "grid";
const SORTS = ["default", "quota", "recent", "name"] as const;
const VIEWS = ["list", "grid"] as const;
const PAGE_SIZE = 12;

interface Props {
  accounts: Account[] | null;
  usages: Record<string, Usage>;
  usageErrors: Record<string, string>;
  usageLoading: Set<string>;
  desktop: DesktopStatus | null;
  loading: boolean;
  searchRef: RefObject<HTMLInputElement | null>;
  onRefresh: () => void;
  onReloadUsage: (ids: string[]) => void;
  onAdd: () => void;
  onSwitch: (a: Account) => void;
  onDetails: (a: Account) => void;
  onDelete: (list: Account[]) => void;
}

const lastUsed = (a: Account) => isoSecs(a.last_used_at);

export default function AccountsPage(props: Props) {
  const { accounts, usages, usageErrors, usageLoading, desktop, loading, searchRef } = props;
  const t = useT();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>(() => getStored("aqm_sort", SORTS, "default"));
  const [view, setView] = useState<View>(() => getStored("aqm_view", VIEWS, "list"));
  const [page, setPage] = useState(0);
  const [selected, setSelected] = useState<Set<string>>(new Set());

  useEffect(() => localStorage.setItem("aqm_sort", sort), [sort]);
  useEffect(() => localStorage.setItem("aqm_view", view), [view]);
  useEffect(() => setPage(0), [query, filter, sort]);

  // drop selections of accounts that are gone
  useEffect(() => {
    setSelected((prev) => {
      const next = new Set([...prev].filter((id) => accounts?.some((a) => a.id === id)));
      return next.size === prev.size ? prev : next;
    });
  }, [accounts]);

  const list = accounts ?? [];
  const counts = useMemo(() => {
    let ok = 0;
    let attention = 0;
    let vault = 0;
    for (const a of list) {
      if (accountStatus(a, t).tone === "ok") ok += 1;
      else attention += 1;
      if (!a.on_desktop) vault += 1;
    }
    return { ok, attention, vault };
  }, [list, t]);

  const current = list.find((a) => a.active);
  const quotaKnown = list.filter((a) => leftHours(usages[a.id]) >= 0);
  const totalLeft = quotaKnown.reduce((sum, a) => sum + leftHours(usages[a.id]), 0);

  // the switchable account with the most hours left
  const recommended = useMemo(() => {
    let best: Account | null = null;
    for (const a of list) {
      if (!canSwitch(a) || accountStatus(a, t).tone !== "ok") continue;
      const left = leftHours(usages[a.id]);
      if (left > 0 && (!best || left > leftHours(usages[best.id]))) best = a;
    }
    return best?.id ?? null;
  }, [list, usages, t]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = list.filter((a) => {
      if (q && !`${a.display_name} ${a.email} ${a.id} ${a.role}`.toLowerCase().includes(q)) return false;
      switch (filter) {
        case "ok":
          return accountStatus(a, t).tone === "ok";
        case "attention":
          return accountStatus(a, t).tone !== "ok";
        case "vault":
          return !a.on_desktop;
        default:
          return true;
      }
    });
    return rows.sort((x, y) => {
      switch (sort) {
        case "quota":
          return leftHours(usages[y.id]) - leftHours(usages[x.id]);
        case "recent":
          return lastUsed(y) - lastUsed(x);
        case "name":
          return who(x).localeCompare(who(y), "zh-CN");
        default:
          return Number(y.active) - Number(x.active) || lastUsed(y) - lastUsed(x);
      }
    });
  }, [list, query, filter, sort, usages, t]);

  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const pageIndex = Math.min(page, pages - 1);
  const shown = filtered.slice(pageIndex * PAGE_SIZE, (pageIndex + 1) * PAGE_SIZE);
  const selecting = selected.size > 0;
  const selectedAccounts = list.filter((a) => selected.has(a.id));
  const allFilteredSelected = filtered.length > 0 && filtered.every((a) => selected.has(a.id));

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const toggleAllFiltered = () =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (allFilteredSelected) filtered.forEach((a) => next.delete(a.id));
      else filtered.forEach((a) => next.add(a.id));
      return next;
    });

  return (
    <div className="page">
      <header className="pageHead">
        <div>
          <h1>{t.app.pages.accounts}</h1>
          <p className="pageSub">{t.accounts.sub}</p>
        </div>
        <div className="actions">
          <button className="btn ghost" onClick={props.onRefresh} disabled={loading} title={t.accounts.refreshTitle}>
            <Icon name="refresh" className={loading ? "spin" : ""} />
            {t.common.refresh}
          </button>
          <button className="btn primary" onClick={props.onAdd} title="Ctrl+N">
            <Icon name="plus" />
            {t.accounts.add}
          </button>
        </div>
      </header>

      {desktop && !desktop.installed && (
        <Alert tone="warn">{t.accounts.notInstalled}</Alert>
      )}

      {list.length > 0 && (
        <div className="stats">
          <div className="stat">
            <span className="statLabel">{t.accounts.statAccounts}</span>
            <b className="statValue">{list.length}</b>
            <span className="statSub">{t.accounts.onDesktop(list.length - counts.vault)}</span>
          </div>
          <div className="stat">
            <span className="statLabel">{t.common.currentAccount}</span>
            <b className="statValue small truncate" title={current ? who(current) : undefined}>
              {current ? who(current) : "—"}
            </b>
            <span className="statSub truncate">
              {current
                ? leftHours(usages[current.id]) >= 0
                  ? t.accounts.left(fmtHours(leftHours(usages[current.id])))
                  : current.email || current.id
                : t.accounts.noCurrent}
            </span>
          </div>
          <div className="stat">
            <span className="statLabel">{t.accounts.statTotal}</span>
            <b className="statValue">{quotaKnown.length ? fmtHours(totalLeft) : "—"}</b>
            <span className="statSub">
              {t.accounts.quotaRead(quotaKnown.length, list.filter(canQuota).length)}
            </span>
          </div>
          <button
            className={cx("stat", "statButton", counts.attention > 0 && "statBad")}
            onClick={() => setFilter(counts.attention > 0 ? "attention" : "all")}
            title={counts.attention > 0 ? t.accounts.onlyAttention : undefined}
          >
            <span className="statLabel">{t.accounts.statAttention}</span>
            <b className="statValue">{counts.attention}</b>
            <span className="statSub">{counts.attention > 0 ? t.accounts.clickToView : t.accounts.allOk}</span>
          </button>
        </div>
      )}

      {list.length > 0 && (
        <div className="toolbar">
          <div className="search">
            <Icon name="search" size={15} />
            <input
              ref={searchRef}
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setQuery("");
                  e.currentTarget.blur();
                }
              }}
              placeholder={t.accounts.searchPlaceholder}
            />
            {!query && <kbd>/</kbd>}
          </div>
          <Segmented<Filter>
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: t.accounts.filterAll, count: list.length },
              { value: "ok", label: t.accounts.filterOk, count: counts.ok },
              { value: "attention", label: t.accounts.filterAttention, count: counts.attention },
              ...(counts.vault > 0 ? [{ value: "vault" as const, label: t.common.vaultOnly, count: counts.vault }] : []),
            ]}
          />
          <select className="select" value={sort} onChange={(e) => setSort(e.target.value as Sort)} aria-label={t.accounts.sortLabel}>
            {SORTS.map((s) => (
              <option key={s} value={s}>
                {t.accounts.sorts[s]}
              </option>
            ))}
          </select>
          <div className="iconGroup" role="group" aria-label={t.accounts.viewLabel}>
            <button className={cx("iconBtn", view === "list" && "on")} onClick={() => setView("list")} title={t.accounts.viewList}>
              <Icon name="list" />
            </button>
            <button className={cx("iconBtn", view === "grid" && "on")} onClick={() => setView("grid")} title={t.accounts.viewGrid}>
              <Icon name="grid" />
            </button>
          </div>
        </div>
      )}

      {accounts === null && (
        <ul className="accounts">
          {[0, 1, 2].map((i) => (
            <li key={i} className="acct ghostCard">
              <div className="skeleton tall" />
              <div className="skeleton" />
              <div className="skeleton short" />
            </li>
          ))}
        </ul>
      )}

      {accounts?.length === 0 && (
        <Empty icon="users" title={t.common.noAccounts}>
          <p>{t.accounts.emptyHint}</p>
          <button className="btn primary" onClick={props.onAdd}>
            <Icon name="plus" />
            {t.accounts.add}
          </button>
        </Empty>
      )}

      {list.length > 0 && filtered.length === 0 && (
        <Empty icon="search" title={t.accounts.noMatch}>
          <button
            className="btn ghost"
            onClick={() => {
              setQuery("");
              setFilter("all");
            }}
          >
            {t.accounts.clearFilters}
          </button>
        </Empty>
      )}

      {shown.length > 0 && (
        <ul className={cx("accounts", view === "grid" && "grid", selecting && "selecting")}>
          {shown.map((a) => (
            <AccountCard
              key={a.id}
              a={a}
              usage={usages[a.id]}
              usageError={usageErrors[a.id]}
              usageLoading={usageLoading.has(a.id)}
              selected={selected.has(a.id)}
              recommended={recommended === a.id}
              onToggle={() => toggle(a.id)}
              onSwitch={() => props.onSwitch(a)}
              onDetails={() => props.onDetails(a)}
              onDelete={() => props.onDelete([a])}
              onReloadUsage={() => props.onReloadUsage([a.id])}
            />
          ))}
        </ul>
      )}

      {pages > 1 && (
        <div className="pager">
          <button className="iconBtn" onClick={() => setPage(pageIndex - 1)} disabled={pageIndex === 0} aria-label={t.accounts.prevPage}>
            <Icon name="chevronLeft" />
          </button>
          <span>
            {pageIndex + 1} / {pages}
          </span>
          <button className="iconBtn" onClick={() => setPage(pageIndex + 1)} disabled={pageIndex >= pages - 1} aria-label={t.accounts.nextPage}>
            <Icon name="chevronRight" />
          </button>
        </div>
      )}

      {selecting && (
        <div className="batchBar" role="toolbar" aria-label={t.accounts.batchLabel}>
          <span>
            {t.accounts.selectedPrefix} <b>{selected.size}</b> {t.accounts.selectedSuffix}
          </span>
          <button className="btn ghost sm" onClick={toggleAllFiltered}>
            {allFilteredSelected ? t.accounts.unselectAll : t.accounts.selectAllCount(filtered.length)}
          </button>
          <span className="sep" />
          <button className="btn sm" onClick={() => props.onReloadUsage([...selected])}>
            <Icon name="refresh" size={14} />
            {t.accounts.refreshQuota}
          </button>
          <button className="btn danger sm" onClick={() => props.onDelete(selectedAccounts)}>
            <Icon name="trash" size={14} />
            {t.common.delete}
          </button>
          <button className="iconBtn" onClick={() => setSelected(new Set())} title={t.accounts.clearSelection}>
            <Icon name="x" />
          </button>
        </div>
      )}
    </div>
  );
}

interface CardProps {
  a: Account;
  usage?: Usage;
  usageError?: string;
  usageLoading: boolean;
  selected: boolean;
  recommended: boolean;
  onToggle: () => void;
  onSwitch: () => void;
  onDetails: () => void;
  onDelete: () => void;
  onReloadUsage: () => void;
}

function AccountCard(p: CardProps) {
  const { a } = p;
  const t = useT();
  const s = accountStatus(a, t);
  const name = who(a);
  const now = Date.now() / 1000;
  const expires = a.refresh_expires_at;
  return (
    <li className={cx("acct", a.active && "isActive", p.selected && "isSelected")}>
      <div className="acctHead">
        <label className="acctCheck" title={t.accounts.select}>
          <input type="checkbox" checked={p.selected} onChange={p.onToggle} aria-label={t.accounts.selectName(name)} />
        </label>
        <Avatar name={name} hue={hue(a.id)} />
        <div className="acctWho">
          <div className="acctName">
            <span className="truncate" title={name}>
              {name}
            </span>
            {a.active && <span className="tag accent">{t.common.inUse}</span>}
            {p.recommended && (
              <span className="tag ok" title={t.accounts.mostLeftTitle}>
                <Icon name="star" size={11} />
                {t.accounts.mostLeft}
              </span>
            )}
            {!a.on_desktop && (
              <span className="tag" title={t.accounts.vaultOnlyTitle}>
                {t.common.vaultOnly}
              </span>
            )}
          </div>
          <div className="acctSub truncate">{a.email || a.id}</div>
        </div>
        <StatusPill tone={s.tone} label={s.label} />
      </div>

      <QuotaBlock a={a} usage={p.usage} error={p.usageError} loading={p.usageLoading} onReload={p.onReloadUsage} />

      <div className="acctFoot">
        <div className="acctMeta">
          {a.role && <span>{a.role}</span>}
          {expires > 0 && (
            <span title={t.accounts.validUntil(fmtDate(expires))}>
              <Icon name="shield" size={12} />
              {expires < now ? t.status.expired : t.accounts.expiresIn(fmtRelative(expires, t))}
            </span>
          )}
          {a.last_used_at && (
            <span title={fmtTime(a.last_used_at)}>
              <Icon name="clock" size={12} />
              {t.accounts.usedAt(fmtRelative(lastUsed(a), t))}
            </span>
          )}
        </div>
        <div className="acctActions">
          <button className="iconBtn" onClick={p.onDetails} title={t.accounts.details}>
            <Icon name="info" />
          </button>
          <button className="iconBtn dangerHover" onClick={p.onDelete} title={t.common.delete}>
            <Icon name="trash" />
          </button>
          {a.active ? (
            <span className="activeMark">
              <Icon name="check" size={14} />
              {t.common.currentAccount}
            </span>
          ) : (
            <button
              className="btn primary sm"
              onClick={p.onSwitch}
              disabled={!canSwitch(a)}
              title={canSwitch(a) ? t.accounts.switchTitle : t.accounts.noSignIn}
            >
              <Icon name="swap" size={14} />
              {t.common.switch}
            </button>
          )}
        </div>
      </div>
    </li>
  );
}

interface QuotaProps {
  a: Account;
  usage?: Usage;
  error?: string;
  loading: boolean;
  onReload: () => void;
}

function QuotaBlock({ a, usage, error, loading, onReload }: QuotaProps) {
  const t = useT();
  if (!canQuota(a)) return <div className="quota muted">{t.accounts.noQuotaInfo}</div>;

  if (!usage) {
    if (error && !loading) {
      return (
        <div className="quota">
          <div className="quotaErr">
            <Icon name="alert" size={13} />
            <span className="truncate" title={error}>
              {t.accounts.quotaFailed(error)}
            </span>
            <button className="linkBtn" onClick={onReload}>
              {t.common.retry}
            </button>
          </div>
        </div>
      );
    }
    return (
      <div className="quota">
        <div className="skeleton" />
      </div>
    );
  }

  return (
    <div className={cx("quota", loading && "isLoading")}>
      <div className="quotaHead">
        <span className="quotaPlan">{usage.plan || t.accounts.quota}</span>
        <span className="quotaHint">
          {usage.trial_ends_at ? t.accounts.trialUntil(new Date(usage.trial_ends_at).toLocaleDateString()) : ""}
          {usage.windows.length > 0 && nextResetLabel(t)}
        </span>
        <button className="iconBtn xs" onClick={onReload} disabled={loading} title={t.accounts.refreshQuota}>
          <Icon name="refresh" size={13} className={loading ? "spin" : ""} />
        </button>
      </div>
      {usage.windows.length === 0 ? (
        <div className="muted">{t.accounts.noQuotaData}</div>
      ) : (
        usage.windows.map((w) => {
          const ratio = w.limit_hours > 0 ? Math.max(0, Math.min(1, w.left_hours / w.limit_hours)) : 0;
          const tone: Tone = ratio > 0.5 ? "ok" : ratio > 0.2 ? "warn" : "bad";
          return (
            <div className="quotaRow" key={w.name}>
              <span className="quotaName">{w.name}</span>
              <span className={`meter ${tone}`} title={t.accounts.used(fmtHours(w.used_hours))}>
                <span style={{ width: `${Math.round(ratio * 100)}%` }} />
              </span>
              <span className="quotaNum">
                {t.accounts.leftPrefix} <b>{fmtHours(w.left_hours)}</b>
                <i> / {fmtHours(w.limit_hours)}</i>
              </span>
            </div>
          );
        })
      )}
    </div>
  );
}
