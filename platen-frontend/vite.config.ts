import adapter from '@sveltejs/adapter-node';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

const apiTarget = process.env.PLATEN_DEV_BACKEND_ADDRESS ?? 'http://localhost:3000';

export default defineConfig({
	plugins: [
		sveltekit({
			compilerOptions: {
				// Enable runes mode because the app uses Svelte 5 runes.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},

			adapter: adapter()
		})
  ],
  server: {
    proxy: {
      "/api": {
        target: apiTarget,
        rewrite: (path) => path.replace(/^\/api/, '')
      }
    }
  }
});
