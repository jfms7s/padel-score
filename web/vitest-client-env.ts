// A Node test environment whose modules compile for the browser, so Svelte effects in a
// `*.svelte.test.ts` file actually run (no DOM is provided; see vite.config.ts).
import type { Environment } from 'vitest/environments';

export default {
	name: 'client',
	viteEnvironment: 'client',
	setup: () => ({ teardown() {} })
} satisfies Environment;
