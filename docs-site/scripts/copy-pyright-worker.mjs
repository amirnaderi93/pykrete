// Copy the prebuilt pyright-browser worker into the site's public/
// directory so it gets served at a stable, base-prefixed URL
// (`<base>/pyright.worker.js`).
//
// The worker bundle ships at
// `node_modules/@typefox/pyright-browser/dist/pyright.worker.js`. It
// can't be imported as a regular module — it's a Web Worker entry
// point. And it also can't be referenced via Vite's
// `new URL('./worker.js', import.meta.url)` trick because the worker
// reads `self.location.toString()` at runtime to spawn *background*
// pyright workers (the foreground worker boots additional MessageChannel
// workers internally — see the `BrowserWorkersHost.createWorker` impl
// in pyright-browser's dist). A `blob:` or content-hashed module URL
// breaks that self-spawn, so the worker must live at a stable HTTP URL
// the browser will accept as a Worker source.
//
// Copying into `public/` solves both constraints: the file lands at
// `<base>/pyright.worker.js` in dev (Astro's dev server serves
// `public/` verbatim) and in the production build (Astro copies
// `public/` into `dist/` at deploy time).
//
// This script runs:
//   - as part of `npm run dev` (so a fresh checkout's dev server
//     finds the file).
//   - as part of `npm run prebuild` (so the production build picks
//     up the latest version after a dep upgrade).

import { copyFileSync, mkdirSync, statSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const docsRoot = resolve(here, '..');
const src = resolve(
  docsRoot,
  'node_modules/@typefox/pyright-browser/dist/pyright.worker.js',
);
const dst = resolve(docsRoot, 'public/pyright.worker.js');

try {
  statSync(src);
} catch {
  // Best-effort: if the dep isn't installed yet (e.g. a checkout
  // that ran `npm run dev` before `npm install`), surface a clear
  // message rather than a confusing ENOENT. The dev server will
  // still come up — the playground page just shows a "pyright
  // failed to load" status until install completes.
  console.warn(
    `[copy-pyright-worker] ${src} not found — skipping. ` +
      `Run \`npm install\` to install @typefox/pyright-browser, then re-run.`,
  );
  process.exit(0);
}

mkdirSync(dirname(dst), { recursive: true });
copyFileSync(src, dst);
console.log(`[copy-pyright-worker] ${src} -> ${dst}`);
