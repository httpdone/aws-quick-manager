import { RefObject, useEffect, useMemo, useState } from "react";
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
      if (accountStatus(a).tone === "ok") ok += 1;
      else attention += 1;
      if (!a.on_desktop) vault += 1;
    }
    return { ok, attention, vault };
  }, [list]);

  const current = list.find((a) => a.active);
  const quotaKnown = list.filter((a) => leftHours(usages[a.id]) >= 0);
  const totalLeft = quotaKnown.reduce((sum, a) => sum + leftHours(usages[a.id]), 0);

  // the switchable account with the most hours left
  const recommended = useMemo(() => {
    let best: Account | null = null;
    for (const a of list) {
      if (!canSwitch(a) || accountStatus(a).tone !== "ok") continue;
      const left = leftHours(usages[a.id]);
      if (left > 0 && (!best || left > leftHours(usages[best.id]))) best = a;
    }
    return best?.id ?? null;
  }, [list, usages]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = list.filter((a) => {
      if (q && !`${a.display_name} ${a.email} ${a.id} ${a.role}`.toLowerCase().includes(q)) return false;
      switch (filter) {
        case "ok":
          return accountStatus(a).tone === "ok";
        case "attention":
          return accountStatus(a).tone !== "ok";
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
  }, [list, query, filter, sort, usages]);

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
          <h1>账号</h1>
          <p className="pageSub">Quick 桌面端的全部账号，一键切换</p>
        </div>
        <div className="actions">
          <button className="btn ghost" onClick={props.onRefresh} disabled={loading} title="从 Quick 同步账号并刷新额度（Ctrl+R）">
            <Icon name="refresh" className={loading ? "spin" : ""} />
            刷新
          </button>
          <button className="btn primary" onClick={props.onAdd} title="Ctrl+N">
            <Icon name="plus" />
            添加账号
          </button>
        </div>
      </header>

      {desktop && !desktop.installed && (
        <Alert tone="warn">没有找到 Quick 桌面端。账号仍可管理，切换后需要手动打开 Quick。</Alert>
      )}

      {list.length > 0 && (
        <div className="stats">
          <div className="stat">
            <span className="statLabel">账号</span>
            <b className="statValue">{list.length}</b>
            <span className="statSub">{list.length - counts.vault} 个已写入 Quick</span>
          </div>
          <div className="stat">
            <span className="statLabel">当前账号</span>
            <b className="statValue small truncate" title={current ? who(current) : undefined}>
              {current ? who(current) : "—"}
            </b>
            <span className="statSub truncate">
              {current
                ? leftHours(usages[current.id]) >= 0
                  ? `剩 ${fmtHours(leftHours(usages[current.id]))}`
                  : current.email || current.id
                : "Quick 里还没有当前账号"}
            </span>
          </div>
          <div className="stat">
            <span className="statLabel">可用额度合计</span>
            <b className="statValue">{quotaKnown.length ? fmtHours(totalLeft) : "—"}</b>
            <span className="statSub">
              {quotaKnown.length}/{list.filter(canQuota).length} 个账号已读取
            </span>
          </div>
          <button
            className={cx("stat", "statButton", counts.attention > 0 && "statBad")}
            onClick={() => setFilter(counts.attention > 0 ? "attention" : "all")}
            title={counts.attention > 0 ? "只看需要处理的账号" : undefined}
          >
            <span className="statLabel">需要处理</span>
            <b className="statValue">{counts.attention}</b>
            <span className="statSub">{counts.attention > 0 ? "点击查看" : "全部正常"}</span>
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
              placeholder="搜索名称、邮箱、角色或 ID"
            />
            {!query && <kbd>/</kbd>}
          </div>
          <Segmented<Filter>
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: "全部", count: list.length },
              { value: "ok", label: "正常", count: counts.ok },
              { value: "attention", label: "需处理", count: counts.attention },
              ...(counts.vault > 0 ? [{ value: "vault" as const, label: "仅账号库", count: counts.vault }] : []),
            ]}
          />
          <select className="select" value={sort} onChange={(e) => setSort(e.target.value as Sort)} aria-label="排序">
            <option value="default">当前优先</option>
            <option value="quota">剩余额度</option>
            <option value="recent">最近使用</option>
            <option value="name">名称</option>
          </select>
          <div className="iconGroup" role="group" aria-label="视图">
            <button className={cx("iconBtn", view === "list" && "on")} onClick={() => setView("list")} title="列表">
              <Icon name="list" />
            </button>
            <button className={cx("iconBtn", view === "grid" && "on")} onClick={() => setView("grid")} title="网格">
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
        <Empty icon="users" title="还没有账号">
          <p>从浏览器登录一个 Amazon Quick 账号，或者导入这台电脑上 Quick 已登录的账号。</p>
          <button className="btn primary" onClick={props.onAdd}>
            <Icon name="plus" />
            添加账号
          </button>
        </Empty>
      )}

      {list.length > 0 && filtered.length === 0 && (
        <Empty icon="search" title="没有匹配的账号">
          <button
            className="btn ghost"
            onClick={() => {
              setQuery("");
              setFilter("all");
            }}
          >
            清除筛选
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
          <button className="iconBtn" onClick={() => setPage(pageIndex - 1)} disabled={pageIndex === 0} aria-label="上一页">
            <Icon name="chevronLeft" />
          </button>
          <span>
            {pageIndex + 1} / {pages}
          </span>
          <button className="iconBtn" onClick={() => setPage(pageIndex + 1)} disabled={pageIndex >= pages - 1} aria-label="下一页">
            <Icon name="chevronRight" />
          </button>
        </div>
      )}

      {selecting && (
        <div className="batchBar" role="toolbar" aria-label="批量操作">
          <span>
            已选 <b>{selected.size}</b> 个
          </span>
          <button className="btn ghost sm" onClick={toggleAllFiltered}>
            {allFilteredSelected ? "取消全选" : `全选（${filtered.length}）`}
          </button>
          <span className="sep" />
          <button className="btn sm" onClick={() => props.onReloadUsage([...selected])}>
            <Icon name="refresh" size={14} />
            刷新额度
          </button>
          <button className="btn danger sm" onClick={() => props.onDelete(selectedAccounts)}>
            <Icon name="trash" size={14} />
            删除
          </button>
          <button className="iconBtn" onClick={() => setSelected(new Set())} title="取消选择">
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
  const s = accountStatus(a);
  const name = who(a);
  const now = Date.now() / 1000;
  const expires = a.refresh_expires_at;
  return (
    <li className={cx("acct", a.active && "isActive", p.selected && "isSelected")}>
      <div className="acctHead">
        <label className="acctCheck" title="选择">
          <input type="checkbox" checked={p.selected} onChange={p.onToggle} aria-label={`选择 ${name}`} />
        </label>
        <Avatar name={name} hue={hue(a.id)} />
        <div className="acctWho">
          <div className="acctName">
            <span className="truncate" title={name}>
              {name}
            </span>
            {a.active && <span className="tag accent">使用中</span>}
            {p.recommended && (
              <span className="tag ok" title="可切换的账号里剩余额度最多">
                <Icon name="star" size={11} />
                剩余最多
              </span>
            )}
            {!a.on_desktop && (
              <span className="tag" title="只在本工具的账号库里，第一次切换时写入 Quick">
                仅账号库
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
            <span title={`登录有效至 ${fmtDate(expires)}`}>
              <Icon name="shield" size={12} />
              {expires < now ? "登录已过期" : `登录 ${fmtRelative(expires)}到期`}
            </span>
          )}
          {a.last_used_at && (
            <span title={fmtTime(a.last_used_at)}>
              <Icon name="clock" size={12} />
              {fmtRelative(lastUsed(a))}使用
            </span>
          )}
        </div>
        <div className="acctActions">
          <button className="iconBtn" onClick={p.onDetails} title="详情">
            <Icon name="info" />
          </button>
          <button className="iconBtn dangerHover" onClick={p.onDelete} title="删除">
            <Icon name="trash" />
          </button>
          {a.active ? (
            <span className="activeMark">
              <Icon name="check" size={14} />
              当前账号
            </span>
          ) : (
            <button
              className="btn primary sm"
              onClick={p.onSwitch}
              disabled={!canSwitch(a)}
              title={canSwitch(a) ? "把 Quick 切换到这个账号" : "这个账号没有可用的登录信息"}
            >
              <Icon name="swap" size={14} />
              切换
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
  if (!canQuota(a)) return <div className="quota muted">没有登录信息，读不到额度</div>;

  if (!usage) {
    if (error && !loading) {
      return (
        <div className="quota">
          <div className="quotaErr">
            <Icon name="alert" size={13} />
            <span className="truncate" title={error}>
              额度读取失败：{error}
            </span>
            <button className="linkBtn" onClick={onReload}>
              重试
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
        <span className="quotaPlan">{usage.plan || "额度"}</span>
        <span className="quotaHint">
          {usage.trial_ends_at ? `试用至 ${new Date(usage.trial_ends_at).toLocaleDateString()} · ` : ""}
          {usage.windows.length > 0 && nextResetLabel()}
        </span>
        <button className="iconBtn xs" onClick={onReload} disabled={loading} title="刷新额度">
          <Icon name="refresh" size={13} className={loading ? "spin" : ""} />
        </button>
      </div>
      {usage.windows.length === 0 ? (
        <div className="muted">没有额度数据</div>
      ) : (
        usage.windows.map((w) => {
          const ratio = w.limit_hours > 0 ? Math.max(0, Math.min(1, w.left_hours / w.limit_hours)) : 0;
          const tone: Tone = ratio > 0.5 ? "ok" : ratio > 0.2 ? "warn" : "bad";
          return (
            <div className="quotaRow" key={w.name}>
              <span className="quotaName">{w.name}</span>
              <span className={`meter ${tone}`} title={`已用 ${fmtHours(w.used_hours)}`}>
                <span style={{ width: `${Math.round(ratio * 100)}%` }} />
              </span>
              <span className="quotaNum">
                剩 <b>{fmtHours(w.left_hours)}</b>
                <i> / {fmtHours(w.limit_hours)}</i>
              </span>
            </div>
          );
        })
      )}
    </div>
  );
}
