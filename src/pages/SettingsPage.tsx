import { ReactNode, useEffect, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { AppInfo, DesktopStatus, autostartEnabled, setAutostart } from "../lib/api";
import { Theme, getLaunchAfterSwitch, getTheme, setLaunchAfterSwitch, setTheme } from "../lib/prefs";
import Icon from "../components/Icon";
import { CopyButton, Segmented, Toggle, useUI } from "../components/ui";

interface Props {
  info: AppInfo | null;
  desktop: DesktopStatus | null;
  onRefreshDesktop: () => void;
}

const SHORTCUTS: [string, string][] = [
  ["Ctrl + R / F5", "从 Quick 同步并刷新"],
  ["Ctrl + N", "添加账号"],
  ["Ctrl + 1 … 5", "切换页面"],
  ["/", "搜索账号"],
  ["Esc", "关闭对话框 / 清空搜索"],
];

export default function SettingsPage({ info, desktop, onRefreshDesktop }: Props) {
  const { toast } = useUI();
  const [theme, setThemeState] = useState<Theme>(getTheme);
  const [launch, setLaunch] = useState(getLaunchAfterSwitch);
  const [autostart, setAutostartState] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    autostartEnabled()
      .then(setAutostartState)
      .catch(() => setAutostartState(null));
  }, []);

  const changeTheme = (t: Theme) => {
    setThemeState(t);
    setTheme(t);
  };

  const changeLaunch = (on: boolean) => {
    setLaunch(on);
    setLaunchAfterSwitch(on);
  };

  const changeAutostart = async (on: boolean) => {
    setSaving(true);
    try {
      await setAutostart(on);
      setAutostartState(on);
      toast(on ? "已开启开机自启" : "已关闭开机自启");
    } catch (e) {
      toast(String(e), "bad");
    } finally {
      setSaving(false);
    }
  };

  const reveal = (path: string) => revealItemInDir(path).catch((e) => toast(String(e), "bad"));

  return (
    <div className="page narrow">
      <header className="pageHead">
        <div>
          <h1>设置</h1>
        </div>
      </header>

      <section className="panel">
        <h2>常规</h2>
        <SettingRow title="外观" desc="跟随系统，或固定为浅色 / 深色">
          <Segmented<Theme>
            value={theme}
            onChange={changeTheme}
            options={[
              { value: "system", label: <Icon name="monitor" size={14} />, title: "跟随系统" },
              { value: "light", label: <Icon name="sun" size={14} />, title: "浅色" },
              { value: "dark", label: <Icon name="moon" size={14} />, title: "深色" },
            ]}
          />
        </SettingRow>
        <SettingRow title="切换后启动 Quick" desc="切换完成后自动用新账号打开 Quick">
          <Toggle checked={launch} onChange={changeLaunch} label="切换后启动 Quick" />
        </SettingRow>
        <SettingRow title="开机自启" desc={autostart === null ? "读取状态失败" : "登录系统后在托盘里启动"}>
          <Toggle checked={autostart === true} onChange={changeAutostart} disabled={autostart === null || saving} label="开机自启" />
        </SettingRow>
      </section>

      <section className="panel">
        <div className="panelHead">
          <h2>Quick 桌面端</h2>
          <button className="btn ghost sm" onClick={onRefreshDesktop}>
            <Icon name="refresh" size={13} />
            重新检测
          </button>
        </div>
        <SettingRow title="状态" desc={desktop?.running ? `PID ${desktop.pid}` : undefined}>
          <span className={`status ${desktop?.running ? "on" : desktop?.installed ? "" : "bad"}`}>
            <i />
            {!desktop ? "检测中…" : desktop.running ? "运行中" : desktop.installed ? "未运行" : "没找到安装"}
          </span>
        </SettingRow>
        {desktop?.exe && <PathRow title="程序位置" path={desktop.exe} onReveal={reveal} />}
      </section>

      <section className="panel">
        <h2>数据位置</h2>
        <PathRow title="Quick 账号目录" path={info?.base_dir} onReveal={reveal} />
        <PathRow title="账号库" path={info?.vault_dir} onReveal={reveal} />
        <PathRow title="清单备份" path={info?.backups_dir} onReveal={reveal} />
      </section>

      <section className="panel">
        <h2>快捷键</h2>
        <ul className="shortcuts">
          {SHORTCUTS.map(([keys, what]) => (
            <li key={keys}>
              <span>{what}</span>
              <kbd>{keys}</kbd>
            </li>
          ))}
        </ul>
      </section>

      <p className="about">
        AWS Quick Manager {info ? `v${info.version}` : ""} · 非官方工具，与 Amazon Web Services 无关 · MIT
        {" · "}
        <a href="https://github.com/httpdone/aws-quick-manager" target="_blank" rel="noopener noreferrer">
          GitHub 主页
        </a>
      </p>
    </div>
  );
}

function SettingRow({ title, desc, children }: { title: string; desc?: string; children: ReactNode }) {
  return (
    <div className="settingRow">
      <div>
        <b>{title}</b>
        {desc && <small>{desc}</small>}
      </div>
      {children}
    </div>
  );
}

function PathRow({ title, path, onReveal }: { title: string; path?: string | null; onReveal: (p: string) => void }) {
  return (
    <div className="settingRow">
      <div className="minw0">
        <b>{title}</b>
        <small className="mono truncate" title={path ?? undefined}>
          {path ?? "…"}
        </small>
      </div>
      <div className="actions">
        {path && <CopyButton text={path} />}
        {path && (
          <button className="iconBtn xs" onClick={() => onReveal(path)} title="在文件夹中显示">
            <Icon name="folder" size={13} />
          </button>
        )}
      </div>
    </div>
  );
}
