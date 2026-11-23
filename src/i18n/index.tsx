import { ReactNode, createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { setTrayLanguage } from "../lib/api";
import { getStored } from "../lib/prefs";
import en from "./en";
import zh, { Messages } from "./zh";

export type Lang = "zh" | "en";
export type LangPref = "system" | Lang;
export type { Messages };

const LANG_KEY = "aqm_language";
const PREFS = ["system", "zh", "en"] as const;
const MESSAGES: Record<Lang, Messages> = { zh, en };

/** Chinese when the first preferred system language is Chinese, English otherwise. */
export function systemLang(): Lang {
  const first = navigator.languages?.[0] ?? navigator.language ?? "";
  return first.toLowerCase().startsWith("zh") ? "zh" : "en";
}

interface I18n {
  t: Messages;
  lang: Lang;
  pref: LangPref;
  setPref: (pref: LangPref) => void;
}

const I18nContext = createContext<I18n | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [pref, setPrefState] = useState<LangPref>(() => getStored(LANG_KEY, PREFS, "system"));
  const [system, setSystem] = useState<Lang>(systemLang);
  const lang = pref === "system" ? system : pref;

  useEffect(() => {
    const onChange = () => setSystem(systemLang());
    window.addEventListener("languagechange", onChange);
    return () => window.removeEventListener("languagechange", onChange);
  }, []);

  useEffect(() => {
    document.documentElement.lang = lang === "zh" ? "zh-CN" : "en";
    setTrayLanguage(lang).catch(() => {});
  }, [lang]);

  const setPref = useCallback((next: LangPref) => {
    localStorage.setItem(LANG_KEY, next);
    setPrefState(next);
  }, []);

  const value = useMemo(() => ({ t: MESSAGES[lang], lang, pref, setPref }), [lang, pref, setPref]);
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18n {
  const i18n = useContext(I18nContext);
  if (!i18n) throw new Error("I18nProvider is missing");
  return i18n;
}

export const useT = () => useI18n().t;
