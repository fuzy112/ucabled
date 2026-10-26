// Injects the server-rendered markup into the two HTML shells produced by the
// client build, so the pages are readable with JavaScript disabled.
import { readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// scripts/ -> project root, so the script also works when invoked from elsewhere
const root = fileURLToPath(new URL("..", import.meta.url));
const { render } = await import(pathToFileURL(path.join(root, "dist-ssr/entry-server.js")).href);

const pages = [
  { file: "dist/index.html", lang: "en" },
  { file: "dist/zh/index.html", lang: "zh" },
];

const ROOT_DIV = '<div id="root"></div>';

for (const { file, lang } of pages) {
  const template = await readFile(path.join(root, file), "utf8");
  if (!template.includes(ROOT_DIV)) {
    throw new Error(`${file}: no empty ${ROOT_DIV} to prerender into`);
  }
  // Function replacement: rendered text may contain "$" sequences.
  const html = template.replace(ROOT_DIV, () => `<div id="root">${render(lang)}</div>`);
  await writeFile(path.join(root, file), html);
  console.log(`prerendered ${file} (${lang})`);
}

await rm(path.join(root, "dist-ssr"), { recursive: true, force: true });
