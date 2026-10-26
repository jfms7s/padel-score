import { defineConfig } from 'vitest/config';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';

export default defineConfig({
	plugins: [
		sveltekit({
			compilerOptions: {
				runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true)
			},
			// Pure SPA: the Rust server embeds build/ and serves index.html for unknown paths.
			adapter: adapter({ pages: 'build', assets: 'build', fallback: 'index.html' }),
			// An open page asks for a new deploy every minute (and on focus); see #lib/update.ts.
			version: { pollInterval: 60_000 }
		})
	],
	server: {
		proxy: {
			'/api': 'http://localhost:8080'
		}
	},
	test: {
		include: ['src/**/*.{test,spec}.ts'],
		environment: 'node'
	}
});
