import { useState } from "react";
import { Account, DesktopStatus, SwitchReport, forceQuitDesktop, switchAccount } from "../lib/api";
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

export default function SwitchDialog({ target, current, desktop, onClose, onDone }: Props) {
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
        setStage("正在强制结束 Quick…");
        await forceQuitDesktop();
      }
      setStage(running && !force ? "正在刷新令牌并退出 Quick…" : "正在刷新令牌并写入账号…");
      const report = await switchAccount(target.id, launch);
      onDone(report, target);
    } catch (e) {
      const text = String(e);
      setMessage(text);
      setPhase(text.includes("没有退出") ? "needForce" : "error");
    }
  };

  const busy = phase === "running";
  const steps = [
    { icon: "refresh" as const, text: "刷新目标账号的登录令牌，失败就不切换" },
    {
      icon: "power" as const,
      text: running ? "退出 Quick，正在进行的任务会中断" : "Quick 没在运行，跳过退出",
    },
    { icon: "save" as const, text: target.on_desktop ? "备份账号清单，写入凭据和设备 ID" : "备份账号清单，把账号写进 Quick 并生成数据目录" },
    { icon: "zap" as const, text: launch ? "用新账号启动 Quick" : "不启动 Quick，下次打开时生效" },
  ];

  const footer =
    phase === "confirm" ? (
      <>
        <button className="btn ghost" onClick={onClose}>
          取消
        </button>
        <button className="btn primary" onClick={() => run(false)} autoFocus>
          <Icon name="swap" size={14} />
          切换
        </button>
      </>
    ) : phase === "running" ? (
      <button className="btn primary" disabled>
        <Spinner />
        切换中…
      </button>
    ) : phase === "needForce" ? (
      <>
        <button className="btn ghost" onClick={onClose}>
          取消
        </button>
        <button className="btn" onClick={() => run(false)}>
          重试
        </button>
        <button className="btn danger" onClick={() => run(true)}>
          <Icon name="power" size={14} />
          强制结束并继续
        </button>
      </>
    ) : (
      <>
        <button className="btn ghost" onClick={onClose}>
          关闭
        </button>
        <button className="btn primary" onClick={() => run(false)}>
          重试
        </button>
      </>
    );

  return (
    <Modal title="切换账号" onClose={onClose} footer={footer} locked={busy}>
      <div className="switchPair">
        <div className="switchSide">
          {current ? (
            <>
              <Avatar name={who(current)} hue={hue(current.id)} size={40} />
              <b className="truncate">{who(current)}</b>
              <small className="truncate">{current.email || "当前账号"}</small>
            </>
          ) : (
            <>
              <span className="avatar placeholder" style={{ width: 40, height: 40 }} />
              <b>无</b>
              <small>Quick 里没有当前账号</small>
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
          <b>Quick 没有按请求退出。</b>
          <p>{message}</p>
          <p className="muted">强制结束会直接终止 Quick 和它的后台，正在写的数据可能丢失。也可以先从托盘手动退出 Quick，再点重试。</p>
        </Alert>
      )}
      {phase === "error" && (
        <Alert tone="bad">
          <b>切换没有完成。</b>
          <p>{message}</p>
        </Alert>
      )}

      {(phase === "confirm" || phase === "error") && (
        <div className="optionRow">
          <div>
            <b>切换后启动 Quick</b>
            <small>{desktop && !desktop.installed ? "没找到 Quick 安装位置，需要手动打开" : "可在设置里修改默认值"}</small>
          </div>
          <Toggle checked={launch} onChange={changeLaunch} label="切换后启动 Quick" />
        </div>
      )}
    </Modal>
  );
}
