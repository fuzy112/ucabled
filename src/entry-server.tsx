import { renderToString } from "react-dom/server";
import App from "./App";
import LangProvider from "./lib/LangProvider";
import type { Lang } from "./lib/i18n";

/** Rendered into the HTML shell at build time so the site works without JS. */
export function render(lang: Lang) {
  return renderToString(
    <LangProvider lang={lang}>
      <App />
    </LangProvider>
  );
}
