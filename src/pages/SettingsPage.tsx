import { ReactNode, useEffect, useState } from "react";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { AppInfo, DesktopStatus, UpdateInfo, autostartEnabled, checkForUpdate, setAutostart } from "../lib/api";
import { Theme, getLaunchAfterSwitch, getTheme, setLaunchAfterSwitch, setTheme } from "../lib/prefs";
import { LangPref, Messages, useI18n, useT } from "../i18n";
import Icon from "../components/Icon";
import { CopyButton, Segmented, Toggle, useUI } from "../components/ui";

interface Props {
  info: AppInfo | null;
  desktop: DesktopStatus | null;
  onRefreshDesktop: () => void;
}

type Shortcut = keyof Messages["settings"]["shortcutList"];

const SHORTCUTS: [string, Shortcut][] = [
  ["Ctrl + R / F5", "refresh"],
  ["Ctrl + N", "add"],
  ["Ctrl + 1 … 5", "pages"],
  ["/", "search"],
  ["Esc", "escape"],
];

export default function SettingsPage({ info, desktop, onRefreshDesktop }: Props) {
  const { toast } = useUI();
  const { t, pref: lang, setPref: setLang } = useI18n();
  const [theme, setThemeState] = useState<Theme>(getTheme);
  const [launch, setLaunch] = useState(getLaunchAfterSwitch);
  const [autostart, setAutostartState] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    autostartEnabled()
      .then(setAutostartState)
      .catch(() => setAutostartState(null));
  }, []);

  const changeTheme = (next: Theme) => {
    setThemeState(next);
    setTheme(next);
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
      toast(on ? t.settings.autostartOn : t.settings.autostartOff);
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
          <h1>{t.settings.title}</h1>
        </div>
      </header>

      <section className="panel">
        <h2>{t.settings.general}</h2>
        <SettingRow title={t.settings.appearance} desc={t.settings.appearanceDesc}>
          <Segmented<Theme>
            value={theme}
            onChange={changeTheme}
            options={[
              { value: "system", label: <Icon name="monitor" size={14} />, title: t.settings.system },
              { value: "light", label: <Icon name="sun" size={14} />, title: t.settings.light },
              { value: "dark", label: <Icon name="moon" size={14} />, title: t.settings.dark },
            ]}
          />
        </SettingRow>
        <SettingRow title={t.settings.language} desc={t.settings.languageDesc}>
          <Segmented<LangPref>
            value={lang}
            onChange={setLang}
            options={[
              { value: "system", label: <Icon name="monitor" size={14} />, title: t.settings.system },
              { value: "zh", label: "中文", title: "中文" },
              { value: "en", label: "EN", title: "English" },
            ]}
          />
        </SettingRow>
        <SettingRow title={t.settings.launchAfter} desc={t.settings.launchAfterDesc}>
          <Toggle checked={launch} onChange={changeLaunch} label={t.settings.launchAfter} />
        </SettingRow>
        <SettingRow title={t.settings.autostart} desc={autostart === null ? t.settings.autostartUnknown : t.settings.autostartDesc}>
          <Toggle checked={autostart === true} onChange={changeAutostart} disabled={autostart === null || saving} label={t.settings.autostart} />
        </SettingRow>
      </section>

      <section className="panel">
        <div className="panelHead">
          <h2>{t.desktop.label}</h2>
          <button className="btn ghost sm" onClick={onRefreshDesktop}>
            <Icon name="refresh" size={13} />
            {t.common.redetect}
          </button>
        </div>
        <SettingRow title={t.settings.status} desc={desktop?.running ? `PID ${desktop.pid}` : undefined}>
          <span className={`status ${desktop?.running ? "on" : desktop?.installed ? "" : "bad"}`}>
            <i />
            {!desktop ? t.settings.detecting : desktop.running ? t.desktop.running : desktop.installed ? t.desktop.stopped : t.desktop.notInstalled}
          </span>
        </SettingRow>
        {desktop?.exe && <PathRow title={t.settings.exe} path={desktop.exe} onReveal={reveal} />}
      </section>

      <section className="panel">
        <h2>{t.settings.dataDirs}</h2>
        <PathRow title={t.settings.baseDir} path={info?.base_dir} onReveal={reveal} />
        <PathRow title={t.settings.vaultDir} path={info?.vault_dir} onReveal={reveal} />
        <PathRow title={t.settings.backupsDir} path={info?.backups_dir} onReveal={reveal} />
      </section>

      <section className="panel">
        <h2>{t.settings.shortcuts}</h2>
        <ul className="shortcuts">
          {SHORTCUTS.map(([keys, what]) => (
            <li key={keys}>
              <span>{t.settings.shortcutList[what]}</span>
              <kbd>{keys}</kbd>
            </li>
          ))}
        </ul>
      </section>

      <AboutUpdates />

      <p className="about">
        AWS Quick Manager {info ? `v${info.version}` : ""} · {t.settings.about} · MIT
        {" · "}
        <a href="https://github.com/httpdone/aws-quick-manager" target="_blank" rel="noopener noreferrer">
          {t.settings.github}
        </a>
      </p>
    </div>
  );
}

type UpdateState =
  | { status: "idle" | "checking" }
  | { status: "done"; info: UpdateInfo }
  | { status: "error"; reason: string };

function AboutUpdates() {
  const { toast } = useUI();
  const t = useT();
  const [update, setUpdate] = useState<UpdateState>({ status: "idle" });

  const check = async () => {
    setUpdate({ status: "checking" });
    try {
      setUpdate({ status: "done", info: await checkForUpdate() });
    } catch (e) {
      setUpdate({ status: "error", reason: String(e) });
    }
  };

  const result = update.status === "done" ? update.info : null;
  const status = result
    ? result.update_available
      ? t.settings.updateAvailable(result.latest)
      : t.settings.upToDate(result.current)
    : update.status === "error"
      ? t.settings.updateFailed(update.reason)
      : update.status === "checking" ? t.settings.checking : "";

  return (
    <section className="panel">
      <h2>{t.settings.aboutTitle}</h2>
      <div className="settingRow">
        <div>
          <b>{t.settings.updates}</b>
          <small role="status">{status}</small>
        </div>
        <div className="actions">
          <button className="btn sm" onClick={check} disabled={update.status === "checking"}>
            {t.settings.checkUpdates}
          </button>
          {result?.update_available && (
            <button className="btn sm" onClick={() => openUrl(result.release_url).catch((e) => toast(String(e), "bad"))}>
              {t.settings.openRelease}
            </button>
          )}
        </div>
      </div>
    </section>
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
  const t = useT();
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
          <button className="iconBtn xs" onClick={() => onReveal(path)} title={t.common.revealInFolder}>
            <Icon name="folder" size={13} />
          </button>
        )}
      </div>
    </div>
  );
}
