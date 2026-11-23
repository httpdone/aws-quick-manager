import { useCallback, useEffect, useState } from "react";
import { DeviceAccount, DeviceOverview, IdentityChange, deviceOverview, replaceAccountDevice, restoreAccountDevice } from "../lib/api";
import { fmtTime, hue } from "../lib/format";
import { who } from "../lib/status";
import Icon from "../components/Icon";
import { Alert, Avatar, CopyButton, Empty, Field, StatusPill, cx, useUI } from "../components/ui";

interface Props {
  desktopRunning: boolean;
  onChanged: () => void;
}

export default function DevicesPage({ desktopRunning, onChanged }: Props) {
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
      setError(String(e));
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
      toast(String(e), "bad");
    } finally {
      setBusy(false);
    }
  };

  const replace = async (a: DeviceAccount) => {
    const ok = await confirm({
      title: `更换 ${who(a)} 的整套设备身份？`,
      message:
        "会同时生成新的 device_id、device_configuration_id，以及由它推导的 device_space_id。\n\n旧身份会进历史，可以恢复；这个账号已有的同步文件仍绑在旧 space 上，不会迁移。",
      confirmLabel: "更换",
      danger: true,
    });
    if (ok) {
      run(async () => {
        const next = await replaceAccountDevice(a.id);
        return `已更换 ${who(a)} 的设备身份：${next.device_id}`;
      });
    }
  };

  const restore = async (a: DeviceAccount, h: IdentityChange) => {
    const ok = await confirm({
      title: `恢复 ${who(a)} 的设备身份？`,
      message: `device  ${h.device_id || "—"}\nconfig  ${h.config || "—"}\nspace   ${h.space || "—"}`,
      confirmLabel: "恢复",
    });
    if (ok) {
      run(async () => {
        await restoreAccountDevice(a.id, h);
        return `已把 ${who(a)} 恢复到 ${h.device_id}`;
      });
    }
  };

  const owner = data?.accounts.find((a) => a.id === data.base.owner);
  const drift = !!data && !!data.base.config && !data.base.owner;

  return (
    <div className="page">
      <header className="pageHead">
        <div>
          <h1>设备 ID</h1>
          <p className="pageSub">每个账号一套独立的设备身份，切换时写给 Quick</p>
        </div>
        <button className="btn ghost" onClick={load} disabled={loading || busy}>
          <Icon name="refresh" className={loading ? "spin" : ""} />
          刷新
        </button>
      </header>

      {desktopRunning && <Alert tone="info">更换或恢复设备身份前需要先退出 Quick。</Alert>}
      {error && <Alert tone="bad">{error}</Alert>}
      {data && data.conflicts.length > 0 && (
        <Alert tone="bad">
          <b>这些 ID 被多个账号共用，切换会被拒绝：</b>
          <p className="mono">{data.conflicts.join("、")}</p>
        </Alert>
      )}

      {data && (
        <section className="panel">
          <div className="panelHead">
            <div>
              <h2>Quick 当前使用的安装 ID</h2>
              <p>Quick 启动时从这里读取；切换账号时会替换成目标账号的值</p>
            </div>
            {owner ? <span className="tag accent">属于 {who(owner)}</span> : data.base.config ? <span className="tag warn">没有匹配的账号</span> : <span className="tag">Quick 还没生成</span>}
          </div>
          <Field label="configuration" value={data.base.config} mono copy />
          <Field label="space" value={data.base.space} mono copy />
          {drift && <Alert tone="warn">这组安装 ID 不属于任何账号；下一次切换会替换成目标账号的值，切换前不要手改。</Alert>}
        </section>
      )}

      {loading && !data && <div className="skeleton tall" />}
      {data && data.accounts.length === 0 && <Empty icon="cpu" title="还没有账号" />}

      {data && data.accounts.length > 0 && (
        <ul className="devices">
          {data.accounts.map((a) => {
            const conflict =
              data.conflicts.includes(`设备 ID ${a.device_id}`) ||
              data.conflicts.includes(`安装 ID ${a.config}`) ||
              data.conflicts.includes(`space ${a.space}`);
            const ready = !!a.device_id && !!a.config && a.space_matches_config;
            const expanded = open === a.id;
            return (
              <li key={a.id} className={cx("device", expanded && "open", conflict && "isBad")}>
                <button className="deviceHead" onClick={() => setOpen(expanded ? null : a.id)} aria-expanded={expanded}>
                  <Avatar name={who(a)} hue={hue(a.id)} size={32} />
                  <span className="deviceWho">
                    <b className="truncate">
                      {who(a)}
                      {a.active && <span className="tag accent">使用中</span>}
                      {!a.on_desktop && <span className="tag">仅账号库</span>}
                    </b>
                    <small className="mono truncate">{a.device_id || "未分配 device_id"}</small>
                  </span>
                  <StatusPill tone={conflict ? "bad" : ready ? "ok" : "warn"} label={conflict ? "ID 冲突" : ready ? "已配置" : "待补齐"} />
                  <Icon name="chevronRight" className="chev" />
                </button>

                {expanded && (
                  <div className="deviceBody">
                    {a.config && !a.space_matches_config && <Alert tone="warn">space 和 config 推导值不一致，切换会被拒绝；先恢复或更换。</Alert>}
                    {a.profile_device_id && a.vault_device_id && !a.matches && (
                      <Alert tone="warn">profile 里的 device_id 和账号库不一致；切换时以文件为准。</Alert>
                    )}
                    <Field label="device_id" value={a.device_id} mono copy />
                    <Field label="configuration" value={a.config} mono copy />
                    <Field label="space" value={a.space} mono copy />
                    <Field label="profile 里" value={a.profile_device_id ?? "未创建"} mono />
                    <Field label="账号库里" value={a.vault_device_id} mono />

                    <div className="deviceActions">
                      <button className="btn sm" onClick={() => replace(a)} disabled={busy || desktopRunning}>
                        <Icon name="refresh" size={14} />
                        更换整套身份
                      </button>
                    </div>

                    {a.history.length > 0 && (
                      <div className="history">
                        <h3>历史（{a.history.length}）</h3>
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
                                <span className="muted">{fmtTime(h.replaced_at)} 换下</span>
                                <button className="btn ghost sm" onClick={() => restore(a, h)} disabled={busy || desktopRunning}>
                                  <Icon name="history" size={13} />
                                  恢复
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
