# PR-0-1 — Add Tailwind to the frontend

Status: ✅ done (commit `38680dd`).

**Scope.** Install Tailwind, configure it for Vite + React, replace
the inline-style approach in `App.tsx` with a small smoke-test of
utility classes to prove the pipeline. Don't restyle the whole UI
yet — that's Phase 4 (Settings UI) work.

**Acceptance criteria.**

- `tailwindcss` and `@tailwindcss/vite` in `package.json`'s
  `devDependencies`.
- `vite.config.ts` registers the Tailwind v4 plugin.
- `src/index.css` (or equivalent) contains `@import "tailwindcss";`.
- `src/main.tsx` imports the stylesheet.
- `App.tsx` uses at least one Tailwind class (e.g. wrap the header
  in `<h1 className="text-2xl font-semibold mb-4">`) and renders
  styled in `npm run tauri dev` — verifiable by opening the app and
  seeing the styled element.
- The existing inline styles in `App.tsx` remain functional during
  the transition (don't break the running UI).

**Effort.** Half a day.

**Dependencies.** None.

**Out of scope.** Restyling the introspection table, the slice form,
or any other surface beyond the smoke header. Theme customization,
dark mode, design tokens — all Phase 4.

**Implementation note (post-delivery).** Tailwind v4 dropped PostCSS
config in favor of the Vite plugin; the ticket originally specified
`tailwind.config.js` + `postcss.config.js`, but v4's Vite-plugin
approach replaces both. See commit `38680dd` for the actual shape.
