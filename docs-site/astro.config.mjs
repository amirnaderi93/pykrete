// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// Deployed to GitHub Pages at amirnaderi93.github.io/pykrete.
// If/when a custom domain is added, drop `base` and update `site`.
export default defineConfig({
  site: 'https://amirnaderi93.github.io',
  base: '/pykrete',
  integrations: [
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
        {
          label: 'Getting started',
          items: [
            { label: 'Why pykrete', slug: 'getting-started/why' },
            { label: 'Install', slug: 'getting-started/install' },
            { label: 'Quickstart', slug: 'getting-started/quickstart' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Schemas', slug: 'reference/schemas' },
            { label: 'Diagnostics', slug: 'reference/diagnostics' },
            { label: 'Configuration', slug: 'reference/configuration' },
          ],
        },
        {
          label: 'About',
          items: [
            { label: 'How it works', slug: 'about/how-it-works' },
            { label: 'Roadmap', slug: 'about/roadmap' },
            { label: 'Real-codebase tests', slug: 'about/pykrete-tests' },
          ],
        },
      ],
    }),
  ],
});
