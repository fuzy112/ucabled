import { StrictMode } from 'react'
import { createRoot, hydrateRoot } from 'react-dom/client'
import '@fontsource/jetbrains-mono/400.css'
import '@fontsource/jetbrains-mono/700.css'
import '@fontsource/jetbrains-mono/800.css'
import './index.css'
import App from './App.tsx'
import LangProvider from './lib/LangProvider.tsx'

// The language is part of the URL: / is English, /zh/ is Chinese.  That keeps
// the static pages linkable and lets the switcher work without JavaScript.
const lang = /(?:^|\/)zh(?:\/|$)/.test(window.location.pathname) ? 'zh' : 'en'

const container = document.getElementById('root')!
const app = (
  <StrictMode>
    <LangProvider lang={lang}>
      <App />
    </LangProvider>
  </StrictMode>
)

// Production builds ship prerendered markup; the dev server starts empty.
if (container.hasChildNodes()) hydrateRoot(container, app)
else createRoot(container).render(app)
