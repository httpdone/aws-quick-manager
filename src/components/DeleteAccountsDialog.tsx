import { useState } from "react";
import { Account, deleteAccount } from "../lib/api";
import { useT } from "../i18n";
import { hue } from "../lib/format";
import { who } from "../lib/status";
import Icon from "./Icon";
import Modal from "./Modal";
import { Alert, Avatar, Spinner, cx } from "./ui";

interface Props {
  accounts: Account[];
  desktopRunning: boolean;
  onClose: () => void;
  onDone: (message: string) => void;
}

export default function DeleteAccountsDialog({ accounts, desktopRunning, onClose, onDone }: Props) {
  const t = useT();
  const hasDesktop = accounts.some((a) => a.on_desktop);
  const hasActive = accounts.some((a) => a.active);
  const [removeDesktop, setRemoveDesktop] = useState(hasDesktop);
  const [removeData, setRemoveData] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const blocked = removeDesktop && hasDesktop && desktopRunning;
  const single = accounts.length === 1;

  const run = async () => {
    setBusy(true);
    setError("");
    let done = 0;
    const failures: string[] = [];
    for (const a of accounts) {
      try {
        await deleteAccount(a.id, removeDesktop && a.on_desktop, removeDesktop && removeData && a.on_desktop);
        done += 1;
      } catch (e) {
        failures.push(`${who(a)}：${String(e)}`);
      }
    }
    setBusy(false);
    if (failures.length === 0) onDone(single ? t.remove.deleted(who(accounts[0])) : t.remove.deletedCount(done));
    else setError(t.remove.partial(done, failures.length, failures.join(t.common.joinSep)));
  };

  const shown = accounts.slice(0, 6);

  return (
    <Modal
      title={single ? t.remove.titleOne : t.remove.titleMany(accounts.length)}
      onClose={onClose}
      locked={busy}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            {t.common.cancel}
          </button>
          <button className="btn danger" onClick={run} disabled={busy || blocked}>
            {busy ? <Spinner /> : <Icon name="trash" size={14} />}
            {busy ? t.remove.deleting : removeData && removeDesktop ? t.remove.permanent : t.common.delete}
          </button>
        </>
      }
    >
      <ul className="chipList">
        {shown.map((a) => (
          <li key={a.id} className="chip">
            <Avatar name={who(a)} hue={hue(a.id)} size={22} />
            <span className="truncate">{who(a)}</span>
            {a.active && <span className="tag accent">{t.common.inUse}</span>}
          </li>
        ))}
        {accounts.length > shown.length && <li className="chip muted">{t.remove.more(accounts.length)}</li>}
      </ul>

      <div className="checkCards">
        <label className="checkCard on locked">
          <input type="checkbox" checked disabled />
          <span>
            <b>{t.remove.fromVault}</b>
            <small>{t.remove.fromVaultDesc}</small>
          </span>
        </label>
        <label className={cx("checkCard", removeDesktop && "on", !hasDesktop && "disabled")}>
          <input type="checkbox" checked={removeDesktop} disabled={!hasDesktop || busy} onChange={(e) => setRemoveDesktop(e.target.checked)} />
          <span>
            <b>{t.remove.fromDesktop}</b>
            <small>{hasDesktop ? t.remove.fromDesktopDesc : t.remove.notOnDesktop}</small>
          </span>
        </label>
        <label className={cx("checkCard", "danger", removeData && removeDesktop && "on", (!removeDesktop || !hasDesktop) && "disabled")}>
          <input
            type="checkbox"
            checked={removeData && removeDesktop}
            disabled={!removeDesktop || !hasDesktop || busy}
            onChange={(e) => setRemoveData(e.target.checked)}
          />
          <span>
            <b>{t.remove.dataDir}</b>
            <small>{t.remove.dataDirDesc}</small>
          </span>
        </label>
      </div>

      {hasActive && removeDesktop && <Alert tone="warn">{t.remove.includesActive}</Alert>}
      {blocked && <Alert tone="bad">{t.remove.desktopRunning}</Alert>}
      {error && <Alert tone="bad">{error}</Alert>}
    </Modal>
  );
}
