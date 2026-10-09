# ucabled website

Source for <https://fuzy112.github.io/ucabled/>, the project site for
[ucabled](https://github.com/fuzy112/ucabled) — a phone passkey bridge for
Linux (virtual FIDO2 device relaying WebAuthn/SSH ceremonies to a phone over
caBLE v2).

Single-page React + TypeScript + Vite + Tailwind app, CRT phosphor terminal
theme. All content lives in `src/sections/`.

## Develop

```bash
npm ci
npm run dev      # local dev server
npm run build    # typecheck + production build to dist/
npm run lint
```

## Deploy

This branch is `website`. Pushing to it runs
`.github/workflows/deploy.yml`, which builds the site and deploys `dist/` to
GitHub Pages (`gh-pages` environment). There is nothing to deploy by hand.

Screenshots in `src/assets/` are captured from the real GTK4 helper in a
headless wlroots compositor; see the main repository for the procedure.
