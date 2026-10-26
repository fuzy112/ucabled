# ucabled website

Source for <https://fuzy112.github.io/ucabled/>, the project site for
[ucabled](https://github.com/fuzy112/ucabled) — a phone passkey bridge for
Linux (virtual FIDO2 device relaying WebAuthn/SSH ceremonies to a phone over
caBLE v2).

Static React + TypeScript + Vite + Tailwind site (English at `/`, Chinese at
`/zh/`), CRT phosphor terminal theme. All content lives in `src/lib/i18n.ts`
and `src/sections/`.

The pages are prerendered at build time (`src/entry-server.tsx` +
`scripts/prerender.mjs`), so all content is present in the HTML and remains
readable with JavaScript disabled. Without JS the boot overlay, reveal
animations and copy buttons are dropped; the install cache panel uses a native
`<details>` and language switching is a pair of plain links (`/` for English,
`/zh/` for Chinese). With JS the same markup is hydrated and the effects run.

## Develop

```bash
npm ci
npm run dev      # local dev server (client-rendered, root and /zh/)
npm run build    # typecheck + prerender both pages to dist/
npm run preview  # serve dist/ at http://localhost:4173/ucabled/
npm run lint
```

The dev server does not prerender, so inspect the no-JS behaviour with
`npm run build && npm run preview` and scripts disabled.

## Deploy

This branch is `website`. Pushing to it runs
`.github/workflows/deploy.yml`, which builds the site and deploys `dist/` to
GitHub Pages (`gh-pages` environment). There is nothing to deploy by hand.

Screenshots in `src/assets/` are captured from the real GTK4 helper in a
headless wlroots compositor; see the main repository for the procedure.
