import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  preprocess: vitePreprocess(),
  compilerOptions: {
    // Svelte 5 runes mode, project-wide. No legacy reactive statements.
    runes: true,
  },
};
