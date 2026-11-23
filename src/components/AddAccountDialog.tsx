import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Account, importDesktopAccount, loginAccount, manualLoginStart, manualLoginStatus, manualLoginSubmit } from "../lib/api";
import { hue } from "../lib/format";
import { who } from "../lib/status";
import Icon, { IconName } from "./Icon";
import Modal from "./Modal";
import { Alert, Avatar, CopyButton, Empty, Spinner, cx } from "./ui";

type Method = "web" | "local" | "manual";

interface Props {
  accounts: Account[];
  onClose: () => void;
  onDone: (message: string) => void;
}

const METHODS: { id: Method; icon: IconName; title: string; desc: string }[] = [
  { id: "web", icon: "globe", title: "浏览器登录", desc: "在本机浏览器登录，完成后自动保存" },
  { id: "local", icon: "monitor", title: "从 Quick 导入", desc: "导入 Quick 里已登录的账号" },
  { id: "manual", icon: "link", title: "复制登录地址", desc: "浏览器在另一台电脑上时使用" },
];

export default function AddAccountDialog({ accounts, onClose, onDone }: Props) {
  const importable = accounts.filter((a) => a.on_desktop && !a.in_vault);
  const [method, setMethod] = useState<Method>(importable.length > 0 ? "local" : "web");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<Set<string>>(() => new Set(importable.map((a) => a.id)));
  const [manualUrl, setManualUrl] = useState("");
  const [authorizeUrl, setAuthorizeUrl] = useState("");
  const [pasted, setPasted] = useState("");

  const pick = (m: Method) => {
    setMethod(m);
    setError("");
  };

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const startWeb = async () => {
    setBusy(true);
    setError("");
    try {
      const r = await loginAccount();
      onDone(`已添加 ${r.email || r.display_name}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const importLocal = async () => {
    setBusy(true);
    setError("");
    let done = 0;
    const failed: string[] = [];
    for (const id of selected) {
      try {
        if (await importDesktopAccount(id)) done += 1;
        else failed.push(id);
      } catch (e) {
        failed.push(`${id}：${String(e)}`);
      }
    }
    setBusy(false);
    if (failed.length === 0 && done > 0) {
      onDone(`已从 Quick 导入 ${done} 个账号`);
    } else if (done > 0) {
      onDone(`已导入 ${done} 个，${failed.length} 个没有导入成功`);
    } else {
      setError(failed.length ? `没有导入成功：${failed.join("；")}` : "没有导入成功，Quick 的凭据可能读不到");
    }
  };

  const startManual = async () => {
    setBusy(true);
    setError("");
    try {
      setManualUrl(await manualLoginStart());
      setAuthorizeUrl("");
      setPasted("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const submitManual = async () => {
    const text = pasted.trim();
    if (!text) return;
    setBusy(true);
    setError("");
    try {
      await manualLoginSubmit(text);
      setPasted("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // the local callback server may finish the login on its own
  useEffect(() => {
    if (method !== "manual" || !manualUrl) return;
    let alive = true;
    const check = async () => {
      try {
        const status = await manualLoginStatus();
        if (!alive) return;
        if (status.url) setAuthorizeUrl(status.url);
        if (status.error) setError(status.error);
        if (status.step === "done" && status.account) {
          alive = false;
          onDone(`已添加 ${status.account.email || status.account.display_name}`);
        }
      } catch (e) {
        if (alive) setError(String(e));
      }
    };
    const timer = window.setInterval(check, 1000);
    check();
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [method, manualUrl, onDone]);

  const open = (url: string) => openUrl(url).catch((e) => setError(String(e)));

  const footer = (
    <>
      <button className="btn ghost" onClick={onClose} disabled={busy}>
        取消
      </button>
      {method === "web" && (
        <button className="btn primary" onClick={startWeb} disabled={busy}>
          {busy ? <Spinner /> : <Icon name="external" size={14} />}
          {busy ? "等待浏览器登录…" : "打开登录页"}
        </button>
      )}
      {method === "local" && (
        <button className="btn primary" onClick={importLocal} disabled={busy || selected.size === 0}>
          {busy ? <Spinner /> : <Icon name="download" size={14} />}
          {busy ? "导入中…" : `导入 ${selected.size} 个`}
        </button>
      )}
      {method === "manual" &&
        (manualUrl ? (
          <button className="btn primary" onClick={submitManual} disabled={busy || !pasted.trim()}>
            {busy && <Spinner />}
            提交地址
          </button>
        ) : (
          <button className="btn primary" onClick={startManual} disabled={busy}>
            {busy && <Spinner />}
            生成登录地址
          </button>
        ))}
    </>
  );

  return (
    <Modal title="添加账号" subtitle="新账号先存进账号库，不会退出 Quick；第一次切换时才写进 Quick。" onClose={onClose} footer={footer} size="lg" locked={busy}>
      <div className="methods">
        {METHODS.map((m) => (
          <button key={m.id} className={cx("method", method === m.id && "on")} onClick={() => pick(m.id)} disabled={busy}>
            <span className="methodIcon">
              <Icon name={m.icon} size={18} />
            </span>
            <b>
              {m.title}
              {m.id === "local" && importable.length > 0 && <em>{importable.length}</em>}
            </b>
            <small>{m.desc}</small>
          </button>
        ))}
      </div>

      {error && <Alert tone="bad">{error}</Alert>}

      {method === "web" && (
        <div className="methodBody">
          {busy ? (
            <div className="progressLine">
              <Spinner size={16} />
              <span>已在浏览器打开 Amazon 登录页，登录完成后这里会自动继续。</span>
            </div>
          ) : (
            <ol className="steps">
              <li>
                <Icon name="external" size={14} />
                <span>点"打开登录页"，在浏览器里登录 Amazon Quick 账号</span>
              </li>
              <li>
                <Icon name="check" size={14} />
                <span>浏览器提示完成后回到这里，账号会自动出现在列表里</span>
              </li>
            </ol>
          )}
        </div>
      )}

      {method === "local" && (
        <div className="methodBody">
          {importable.length === 0 ? (
            <Empty icon="monitor" title="没有可导入的账号">
              <p>Quick 里已登录的账号都已经在账号库里了。</p>
            </Empty>
          ) : (
            <>
              <div className="pickHead">
                <span>Quick 里已登录、账号库还没有的账号</span>
                <button
                  className="linkBtn"
                  onClick={() => setSelected(selected.size === importable.length ? new Set() : new Set(importable.map((a) => a.id)))}
                  disabled={busy}
                >
                  {selected.size === importable.length ? "全不选" : "全选"}
                </button>
              </div>
              <ul className="pickList">
                {importable.map((a) => (
                  <li key={a.id}>
                    <label className={cx(selected.has(a.id) && "on")}>
                      <input type="checkbox" checked={selected.has(a.id)} onChange={() => toggle(a.id)} disabled={busy} />
                      <Avatar name={who(a)} hue={hue(a.id)} size={28} />
                      <span className="pickWho">
                        <b className="truncate">{who(a)}</b>
                        <small className="truncate">{a.email || a.id}</small>
                      </span>
                      {a.active && <span className="tag accent">使用中</span>}
                    </label>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}

      {method === "manual" && (
        <div className="methodBody">
          {!manualUrl ? (
            <ol className="steps">
              <li>
                <Icon name="link" size={14} />
                <span>生成登录地址，复制到任意一台电脑的浏览器里打开</span>
              </li>
              <li>
                <Icon name="copy" size={14} />
                <span>浏览器在本机时会自动完成；在别的电脑上时，把跳转后的地址栏地址粘贴回来（一共两次）</span>
              </li>
            </ol>
          ) : (
            <div className="manualFlow">
              <UrlStep n={1} label="登录地址" url={manualUrl} onOpen={open} done={!!authorizeUrl} />
              {authorizeUrl && <UrlStep n={2} label="授权地址" url={authorizeUrl} onOpen={open} />}
              <label className="pasteField">
                <span>
                  <em className="stepNo">{authorizeUrl ? 3 : 2}</em>
                  {authorizeUrl ? "授权完成后，把浏览器地址栏的地址粘到这里" : "第一次跳转后，把浏览器地址栏的地址粘到这里"}
                </span>
                <textarea
                  value={pasted}
                  onChange={(e) => setPasted(e.target.value)}
                  placeholder="http://127.0.0.1:…/?code=…&state=…"
                  disabled={busy}
                  rows={3}
                />
              </label>
              <div className="manualHint">
                <Spinner size={12} />
                <span>浏览器在本机时会自动完成，无需粘贴。</span>
                <button className="linkBtn" onClick={startManual} disabled={busy}>
                  重新生成
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </Modal>
  );
}

function UrlStep({ n, label, url, onOpen, done }: { n: number; label: string; url: string; onOpen: (url: string) => void; done?: boolean }) {
  return (
    <div className={cx("urlStep", done && "done")}>
      <span className="urlLabel">
        <em className="stepNo">{done ? <Icon name="check" size={11} /> : n}</em>
        {label}
      </span>
      <div className="urlRow">
        <input readOnly value={url} onFocus={(e) => e.currentTarget.select()} className="mono" />
        <CopyButton text={url} label="复制" />
        <button className="btn ghost sm" onClick={() => onOpen(url)}>
          <Icon name="external" size={13} />
          打开
        </button>
      </div>
    </div>
  );
}
