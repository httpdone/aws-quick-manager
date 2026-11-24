import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Account, importDesktopAccount, loginAccount, manualLoginStart, manualLoginStatus, manualLoginSubmit } from "../lib/api";
import { useT } from "../i18n";
import { errorText } from "../lib/errors";
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

const METHODS: { id: Method; icon: IconName }[] = [
  { id: "web", icon: "globe" },
  { id: "local", icon: "monitor" },
  { id: "manual", icon: "link" },
];

export default function AddAccountDialog({ accounts, onClose, onDone }: Props) {
  const t = useT();
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
      onDone(t.add.added(r.email || r.display_name));
    } catch (e) {
      setError(errorText(t, e));
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
        failed.push(`${id}: ${errorText(t, e)}`);
      }
    }
    setBusy(false);
    if (failed.length === 0 && done > 0) {
      onDone(t.add.importedAll(done));
    } else if (done > 0) {
      onDone(t.add.importedSome(done, failed.length));
    } else {
      setError(failed.length ? t.add.importFailedList(failed.join(t.common.joinSep)) : t.add.importFailed);
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
      setError(errorText(t, e));
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
      setError(errorText(t, e));
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
        if (status.error) setError(errorText(t, status.error));
        if (status.step === "done" && status.account) {
          alive = false;
          onDone(t.add.added(status.account.email || status.account.display_name));
        }
      } catch (e) {
        if (alive) setError(errorText(t, e));
      }
    };
    const timer = window.setInterval(check, 1000);
    check();
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [method, manualUrl, onDone]);

  const open = (url: string) => openUrl(url).catch((e) => setError(errorText(t, e)));

  const footer = (
    <>
      <button className="btn ghost" onClick={onClose} disabled={busy}>
        {t.common.cancel}
      </button>
      {method === "web" && (
        <button className="btn primary" onClick={startWeb} disabled={busy}>
          {busy ? <Spinner /> : <Icon name="external" size={14} />}
          {busy ? t.add.waitingBrowser : t.add.openLogin}
        </button>
      )}
      {method === "local" && (
        <button className="btn primary" onClick={importLocal} disabled={busy || selected.size === 0}>
          {busy ? <Spinner /> : <Icon name="download" size={14} />}
          {busy ? t.add.importing : t.add.importCount(selected.size)}
        </button>
      )}
      {method === "manual" &&
        (manualUrl ? (
          <button className="btn primary" onClick={submitManual} disabled={busy || !pasted.trim()}>
            {busy && <Spinner />}
            {t.add.submitUrl}
          </button>
        ) : (
          <button className="btn primary" onClick={startManual} disabled={busy}>
            {busy && <Spinner />}
            {t.add.generateUrl}
          </button>
        ))}
    </>
  );

  return (
    <Modal title={t.add.title} subtitle={t.add.subtitle} onClose={onClose} footer={footer} size="lg" locked={busy}>
      <div className="methods">
        {METHODS.map((m) => (
          <button key={m.id} className={cx("method", method === m.id && "on")} onClick={() => pick(m.id)} disabled={busy}>
            <span className="methodIcon">
              <Icon name={m.icon} size={18} />
            </span>
            <b>
              {t.add.methods[m.id].title}
              {m.id === "local" && importable.length > 0 && <em>{importable.length}</em>}
            </b>
            <small>{t.add.methods[m.id].desc}</small>
          </button>
        ))}
      </div>

      {error && <Alert tone="bad">{error}</Alert>}

      {method === "web" && (
        <div className="methodBody">
          {busy ? (
            <div className="progressLine">
              <Spinner size={16} />
              <span>{t.add.webWaiting}</span>
            </div>
          ) : (
            <ol className="steps">
              <li>
                <Icon name="external" size={14} />
                <span>{t.add.webStep1}</span>
              </li>
              <li>
                <Icon name="check" size={14} />
                <span>{t.add.webStep2}</span>
              </li>
            </ol>
          )}
        </div>
      )}

      {method === "local" && (
        <div className="methodBody">
          {importable.length === 0 ? (
            <Empty icon="monitor" title={t.add.nothingToImport}>
              <p>{t.add.allImported}</p>
            </Empty>
          ) : (
            <>
              <div className="pickHead">
                <span>{t.add.importableHead}</span>
                <button
                  className="linkBtn"
                  onClick={() => setSelected(selected.size === importable.length ? new Set() : new Set(importable.map((a) => a.id)))}
                  disabled={busy}
                >
                  {selected.size === importable.length ? t.add.selectNone : t.add.selectAll}
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
                      {a.active && <span className="tag accent">{t.common.inUse}</span>}
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
                <span>{t.add.manualStep1}</span>
              </li>
              <li>
                <Icon name="copy" size={14} />
                <span>{t.add.manualStep2}</span>
              </li>
            </ol>
          ) : (
            <div className="manualFlow">
              <UrlStep n={1} label={t.add.loginUrl} url={manualUrl} onOpen={open} done={!!authorizeUrl} />
              {authorizeUrl && <UrlStep n={2} label={t.add.authorizeUrl} url={authorizeUrl} onOpen={open} />}
              <label className="pasteField">
                <span>
                  <em className="stepNo">{authorizeUrl ? 3 : 2}</em>
                  {authorizeUrl ? t.add.pasteAfterAuthorize : t.add.pasteAfterRedirect}
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
                <span>{t.add.autoCompletes}</span>
                <button className="linkBtn" onClick={startManual} disabled={busy}>
                  {t.add.regenerate}
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
  const t = useT();
  return (
    <div className={cx("urlStep", done && "done")}>
      <span className="urlLabel">
        <em className="stepNo">{done ? <Icon name="check" size={11} /> : n}</em>
        {label}
      </span>
      <div className="urlRow">
        <input readOnly value={url} onFocus={(e) => e.currentTarget.select()} className="mono" />
        <CopyButton text={url} label={t.common.copy} />
        <button className="btn ghost sm" onClick={() => onOpen(url)}>
          <Icon name="external" size={13} />
          {t.common.open}
        </button>
      </div>
    </div>
  );
}
