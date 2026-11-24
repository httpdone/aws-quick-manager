import { useCallback, useEffect, useState } from "react";
import { DeviceAccount, DeviceOverview, IdConflict, IdentityChange, deviceOverview, replaceAccountDevice, restoreAccountDevice } from "../lib/api";
import { useT } from "../i18n";
import { errorText } from "../lib/errors";
import { fmtTime, hue } from "../lib/format";
import { who } from "../lib/status";
import Icon from "../components/Icon";
import { Alert, Avatar, CopyButton, Empty, Field, StatusPill, cx, useUI } from "../components/ui";

const KIND_LABEL: Record<IdConflict["kind"], string> = { device: "device_id", config: "configuration", space: "space" };
const conflictLabel = (c: IdConflict) => `${KIND_LABEL[c.kind]} ${c.id}`;

interface Props {
  desktopRunning: boolean;
  onChanged: () => void;
}

export default function DevicesPage({ desktopRunning, onChanged }: Props) {
  const t = useT();
  const { toast, confirm } = useUI();
  const [data, setData] = useState<DeviceOverview | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [open, setOpen] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setData(await deviceOverview());
      setError("");
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const run = async (action: () => Promise<string>) => {
    setBusy(true);
    try {
      toast(await action());
      await load();
      onChanged();
    } catch (e) {
      toast(errorText(t, e), "bad");
    } finally {
      setBusy(false);
    }
  };

  const replace = async (a: DeviceAccount) => {
    const ok = await confirm({
      title: t.devices.replaceTitle(who(a)),
      message: t.devices.replaceMessage,
      confirmLabel: t.devices.replace,
      danger: true,
    });
    if (ok) {
      run(async () => {
        const next = await replaceAccountDevice(a.id);
        return t.devices.replaced(who(a), next.device_id);
      });
    }
  };

  const restore = async (a: DeviceAccount, h: IdentityChange) => {
    const ok = await confirm({
      title: t.devices.restoreTitle(who(a)),
      message: `device  ${h.device_id || "—"}\nconfig  ${h.config || "—"}\nspace   ${h.space || "—"}`,
      confirmLabel: t.common.restore,
    });
    if (ok) {
      run(async () => {
        await restoreAccountDevice(a.id, h);
        return t.devices.restored(who(a), h.device_id);
      });
    }
  };

  const owner = data?.accounts.find((a) => a.id === data.base.owner);
  const drift = !!data && !!data.base.config && !data.base.owner;

  return (
    <div className="page">
      <header className="pageHead">
        <div>
          <h1>{t.devices.title}</h1>
          <p className="pageSub">{t.devices.sub}</p>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading || busy}>
          <Icon name="refresh" className={loading ? "spin" : ""} />
          {t.common.refresh}
        </button>
      </header>

      {desktopRunning && <Alert tone="info">{t.devices.desktopRunning}</Alert>}
      {error && <Alert tone="bad">{error}</Alert>}
      {data && data.conflicts.length > 0 && (
        <Alert tone="bad">
          <b>{t.devices.sharedIds}</b>
          <p className="mono">{data.conflicts.map(conflictLabel).join(t.common.listSep)}</p>
        </Alert>
      )}

      {data && (
        <section className="panel">
          <div className="panelHead">
            <div>
              <h2>{t.devices.baseTitle}</h2>
              <p>{t.devices.baseDesc}</p>
            </div>
            {owner ? <span className="tag accent">{t.devices.ownedBy(who(owner))}</span> : data.base.config ? <span className="tag warn">{t.devices.noMatchingAccount}</span> : <span className="tag">{t.devices.notGenerated}</span>}
          </div>
          <Field label="configuration" value={data.base.config} mono copy />
          <Field label="space" value={data.base.space} mono copy />
          {drift && <Alert tone="warn">{t.devices.drift}</Alert>}
        </section>
      )}

      {loading && !data && <div className="skeleton tall" />}
      {data && data.accounts.length === 0 && <Empty icon="cpu" title={t.common.noAccounts} />}

      {data && data.accounts.length > 0 && (
        <ul className="devices">
          {data.accounts.map((a) => {
            const conflict = data.conflicts.some(
              (c) => (c.kind === "device" && c.id === a.device_id) || (c.kind === "config" && c.id === a.config) || (c.kind === "space" && c.id === a.space),
            );
            const ready = !!a.device_id && !!a.config && a.space_matches_config;
            const expanded = open === a.id;
            return (
              <li key={a.id} className={cx("device", expanded && "open", conflict && "isBad")}>
                <button className="deviceHead" onClick={() => setOpen(expanded ? null : a.id)} aria-expanded={expanded}>
                  <Avatar name={who(a)} hue={hue(a.id)} size={32} />
                  <span className="deviceWho">
                    <b className="truncate">
                      {who(a)}
                      {a.active && <span className="tag accent">{t.common.inUse}</span>}
                      {!a.on_desktop && <span className="tag">{t.common.vaultOnly}</span>}
                    </b>
                    <small className="mono truncate">{a.device_id || t.devices.noDeviceId}</small>
                  </span>
                  <StatusPill tone={conflict ? "bad" : ready ? "ok" : "warn"} label={conflict ? t.devices.conflict : ready ? t.devices.ready : t.devices.incomplete} />
                  <Icon name="chevronRight" className="chev" />
                </button>

                {expanded && (
                  <div className="deviceBody">
                    {a.config && !a.space_matches_config && <Alert tone="warn">{t.devices.spaceMismatch}</Alert>}
                    {a.profile_device_id && a.vault_device_id && !a.matches && (
                      <Alert tone="warn">{t.devices.profileMismatch}</Alert>
                    )}
                    <Field label="device_id" value={a.device_id} mono copy />
                    <Field label="configuration" value={a.config} mono copy />
                    <Field label="space" value={a.space} mono copy />
                    <Field label={t.devices.inProfile} value={a.profile_device_id ?? t.devices.notCreated} mono />
                    <Field label={t.devices.inVault} value={a.vault_device_id} mono />

                    <div className="deviceActions">
                      <button className="btn sm" onClick={() => replace(a)} disabled={busy || desktopRunning}>
                        <Icon name="refresh" size={14} />
                        {t.devices.replaceAll}
                      </button>
                    </div>

                    {a.history.length > 0 && (
                      <div className="history">
                        <h3>{t.devices.history(a.history.length)}</h3>
                        <ul>
                          {a.history
                            .slice()
                            .reverse()
                            .map((h) => (
                              <li key={`${h.device_id}-${h.config}-${h.replaced_at}`}>
                                <div className="historyIds mono">
                                  <span>
                                    device {h.device_id || "—"} <CopyButton text={h.device_id} />
                                  </span>
                                  <span>config {h.config || "—"}</span>
                                  <span>space {h.space || "—"}</span>
                                </div>
                                <span className="muted">{t.devices.replacedAt(fmtTime(h.replaced_at))}</span>
                                <button className="btn ghost sm" onClick={() => restore(a, h)} disabled={busy || desktopRunning}>
                                  <Icon name="history" size={13} />
                                  {t.common.restore}
                                </button>
                              </li>
                            ))}
                        </ul>
                      </div>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
