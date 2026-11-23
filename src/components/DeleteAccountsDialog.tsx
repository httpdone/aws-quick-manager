import { useState } from "react";
import { Account, deleteAccount } from "../lib/api";
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
    if (failures.length === 0) onDone(single ? `已删除 ${who(accounts[0])}` : `已删除 ${done} 个账号`);
    else setError(`已删除 ${done} 个，失败 ${failures.length} 个：${failures.join("；")}`);
  };

  const shown = accounts.slice(0, 6);

  return (
    <Modal
      title={single ? "删除账号" : `删除 ${accounts.length} 个账号`}
      onClose={onClose}
      locked={busy}
      footer={
        <>
          <button className="btn ghost" onClick={onClose} disabled={busy}>
            取消
          </button>
          <button className="btn danger" onClick={run} disabled={busy || blocked}>
            {busy ? <Spinner /> : <Icon name="trash" size={14} />}
            {busy ? "删除中…" : removeData && removeDesktop ? "永久删除" : "删除"}
          </button>
        </>
      }
    >
      <ul className="chipList">
        {shown.map((a) => (
          <li key={a.id} className="chip">
            <Avatar name={who(a)} hue={hue(a.id)} size={22} />
            <span className="truncate">{who(a)}</span>
            {a.active && <span className="tag accent">使用中</span>}
          </li>
        ))}
        {accounts.length > shown.length && <li className="chip muted">等 {accounts.length} 个</li>}
      </ul>

      <div className="checkCards">
        <label className="checkCard on locked">
          <input type="checkbox" checked disabled />
          <span>
            <b>从账号库移除</b>
            <small>删掉本工具保存的登录副本</small>
          </span>
        </label>
        <label className={cx("checkCard", removeDesktop && "on", !hasDesktop && "disabled")}>
          <input type="checkbox" checked={removeDesktop} disabled={!hasDesktop || busy} onChange={(e) => setRemoveDesktop(e.target.checked)} />
          <span>
            <b>同时从 Quick 里删除</b>
            <small>{hasDesktop ? "删掉 Quick 清单里的条目和系统凭据" : "选中的账号都不在 Quick 里"}</small>
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
            <b>同时删除本地数据目录</b>
            <small>本地会话、知识库都会删掉，无法恢复</small>
          </span>
        </label>
      </div>

      {hasActive && removeDesktop && <Alert tone="warn">包含当前账号。删除后 Quick 下次启动会用剩下的第一个账号。</Alert>}
      {blocked && <Alert tone="bad">Quick 正在运行，先退出 Quick，或者只从账号库移除。</Alert>}
      {error && <Alert tone="bad">{error}</Alert>}
    </Modal>
  );
}
