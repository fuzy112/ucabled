import { useMemo, type ReactNode } from "react";
import { LangContext, type Lang } from "./i18n";

export default function LangProvider({ lang, children }: { lang: Lang; children: ReactNode }) {
  const value = useMemo(() => ({ lang }), [lang]);
  return <LangContext.Provider value={value}>{children}</LangContext.Provider>;
}
