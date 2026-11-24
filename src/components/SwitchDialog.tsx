import { useState } from "react";
import { Account, DesktopStatus, SwitchReport, forceQuitDesktop, switchAccount } from "../lib/api";
import { useT } from "../i18n";
import { errorCode, errorText } from "../lib/errors";
import { hue } from "../lib/format";
import { getLaunchAfterSwitch, setLaunchAfterSwitch } from "../lib/prefs";
import { who } from "../lib/status";
import Icon from "./Icon";
import Modal from "./Modal";
import { Alert, Avatar, Spinner, Toggle } from "./ui";

interface Props {
  target: Account;
  current?: Account;
  desktop: DesktopStatus | null;
  onClose: () => void;
  onDone: (report: SwitchReport, target: Account) => void;
}

type Phase = "confirm" | "running" | "needForce" | "error";

/** Quick didn't go away on request: offer to force it. */
const FORCE_CODES = ["desktop.still_running", "desktop.close_refused", "desktop.backend_still_running"];

export default function SwitchDialog({ target, current, desktop, onClose, onDone }: Props) {
  const t = useT();
  const [phase, setPhase] = useState<Phase>("confirm");
  const [stage, setStage] = useState("");
  const [message, setMessage] = useState("");
  const [launch, setLaunch] = useState(getLaunchAfterSwitch);
  const running = !!desktop?.running;

  const changeLaunch = (on: boolean) => {
    setLaunch(on);
    setLaunchAfterSwitch(on);
  };

  const run = async (force: boolean) => {
    setPhase("running");
    setMessage("");
    try {
      if (force) {
        setStage(t.switcher.forceQuitting);
        await forceQuitDesktop();
      }
      setStage(running && !force ? t.switcher.refreshingAndQuitting : t.switcher.refreshingAndWriting);
      const report = await switchAccount(target.id, launch);
      onDone(report, target);
    } catch (e) {
      setMessage(errorText(t, e));
      setPhase(FORCE_CODES.includes(errorCode(e) ?? "") ? "needForce" : "error");
    }
  };

  const busy = phase === "running";
  const steps = [
    { icon: "refresh" as const, text: t.switcher.stepRefresh },
    {
      icon: "power" as const,
      text: running ? t.switcher.stepQuit : t.switcher.stepSkipQuit,
    },
    { icon: "save" as const, text: target.on_desktop ? t.switcher.stepWriteCreds : t.switcher.stepWriteNew },
    { icon: "zap" as const, text: launch ? t.switcher.stepLaunch : t.switcher.stepNoLaunch },
  ];

  const footer =
    phase === "confirm" ? (
      <>
        <button className="btn ghost" onClick={onClose}>
          {t.common.cancel}
        </button>
        <button className="btn primary" onClick={() => run(false)} autoFocus>
          <Icon name="swap" size={14} />
          {t.common.switch}
        </button>
      </>
    ) : phase === "running" ? (
      <button className="btn primary" disabled>
        <Spinner />
        {t.switcher.switching}
      </button>
    ) : phase === "needForce" ? (
      <>
        <button className="btn ghost" onClick={onClose}>
          {t.common.cancel}
        </button>
        <button className="btn" onClick={() => run(false)}>
          {t.common.retry}
        </button>
        <button className="btn danger" onClick={() => run(true)}>
          <Icon name="power" size={14} />
          {t.switcher.forceContinue}
        </button>
      </>
    ) : (
      <>
        <button className="btn ghost" onClick={onClose}>
          {t.common.close}
        </button>
        <button className="btn primary" onClick={() => run(false)}>
          {t.common.retry}
        </button>
      </>
    );

  return (
    <Modal title={t.switcher.title} onClose={onClose} footer={footer} locked={busy}>
      <div className="switchPair">
        <div className="switchSide">
          {current ? (
            <>
              <Avatar name={who(current)} hue={hue(current.id)} size={40} />
              <b className="truncate">{who(current)}</b>
              <small className="truncate">{current.email || t.common.currentAccount}</small>
            </>
          ) : (
            <>
              <span className="avatar placeholder" style={{ width: 40, height: 40 }} />
              <b>{t.common.none}</b>
              <small>{t.switcher.noCurrent}</small>
            </>
          )}
        </div>
        <span className="switchArrow">
          <Icon name="arrowRight" size={20} />
        </span>
        <div className="switchSide to">
          <Avatar name={who(target)} hue={hue(target.id)} size={40} />
          <b className="truncate">{who(target)}</b>
          <small className="truncate">{target.email || target.id}</small>
        </div>
      </div>

      {phase === "running" ? (
        <div className="progressLine">
          <Spinner size={16} />
          <span>{stage}</span>
        </div>
      ) : (
        <ol className="steps">
          {steps.map((s) => (
            <li key={s.text}>
              <Icon name={s.icon} size={14} />
              <span>{s.text}</span>
            </li>
          ))}
        </ol>
      )}

      {phase === "needForce" && (
        <Alert tone="warn">
          <b>{t.switcher.didNotQuit}</b>
          <p>{message}</p>
          <p className="muted">{t.switcher.forceHint}</p>
        </Alert>
      )}
      {phase === "error" && (
        <Alert tone="bad">
          <b>{t.switcher.failed}</b>
          <p>{message}</p>
        </Alert>
      )}

      {(phase === "confirm" || phase === "error") && (
        <div className="optionRow">
          <div>
            <b>{t.switcher.launchAfter}</b>
            <small>{desktop && !desktop.installed ? t.switcher.noInstall : t.switcher.defaultInSettings}</small>
          </div>
          <Toggle checked={launch} onChange={changeLaunch} label={t.switcher.launchAfter} />
        </div>
      )}
    </Modal>
  );
}
