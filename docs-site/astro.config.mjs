// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import react from '@astrojs/react';

// Deployed to GitHub Pages at amirnaderi93.github.io/pykrete.
// If/when a custom domain is added, drop `base` and update `site`.
export default defineConfig({
  site: 'https://amirnaderi93.github.io',
  base: '/pykrete',
  // React powers the `/playground` page (Monaco editor + live wasm
  // diagnostics). The rest of the site stays static — React is only
  // ever shipped to the client for components marked with a
  // `client:*` directive, so the docs pages aren't affected.
  integrations: [
    react(),
    starlight({
      title: 'pykrete',
      description:
        'Static schema checking for Python dataframes. TypeScript-style annotations catch column-name typos, schema drift, and shape mismatches at check time — across whole transformation chains.',
      logo: {
        src: './src/assets/logo.svg',
        alt: 'pykrete logo',
        replacesTitle: false,
      },
      favicon: '/favicon.svg',
      social: {
        github: 'https://github.com/amirnaderi93/pykrete',
      },
      editLink: {
        baseUrl:
          'https://github.com/amirnaderi93/pykrete/edit/main/docs-site/',
      },
      sidebar: [
        // Playground first — the launch centerpiece. We want "try
        // pykrete instantly in the browser" to be the entry point a
        // new visitor sees before the docs proper.
        { label: 'Playground', slug: 'playground' },
        {
          label: 'Getting started',
          items: [
            { label: 'Why pykrete', slug: 'getting-started/why' },
            { label: 'Install', slug: 'getting-started/install' },
            { label: 'Quickstart', slug: 'getting-started/quickstart' },
            { label: 'Cookbook', slug: 'cookbook' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Schemas', slug: 'reference/schemas' },
            { label: 'Operations', slug: 'reference/operations' },
            { label: 'Diagnostics', slug: 'reference/diagnostics' },
            { label: 'Configuration', slug: 'reference/configuration' },
          ],
        },
        {
          label: 'About',
          items: [
            { label: 'How it works', slug: 'about/how-it-works' },
            { label: 'Production readiness', slug: 'about/production-readiness' },
            { label: 'Roadmap', slug: 'about/roadmap' },
            { label: 'Real-codebase tests', slug: 'about/pykrete-tests' },
          ],
        },
      ],
    }),
  ],
  vite: {
    // Vite warns on `wasm-pack`-built ES modules because the package
    // ships a `.wasm` import; this opts the package into Vite's
    // optimization pipeline so the dev server doesn't choke.
    optimizeDeps: {
      exclude: ['pykrete-wasm'],
    },
  },
});
