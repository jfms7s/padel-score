// The offline app shell: the built app, fonts, icons and the scoring wasm are cached when the
// worker installs, so a scorer who loses signal at the fence can still open and use the page.
// Nothing under /api is ever cached or touched: scores always come from the network.

import { version } from '$app/env';
import { assets, immutable } from '$app/manifest';
import { self as sw } from '$app/service-worker';

const CACHE = `shell-${version}`;
// The app is an SPA (adapter-static `fallback: index.html`): every route is this one document.
const SHELL = '/index.html';
// `immutable` holds what Vite built (JS, CSS, the .wasm), `assets` what is in static/ (fonts,
// icons, manifest).
// Manifest paths have no leading slash; the app is served from the root.
const absolute = (path: string) => (path.startsWith('/') ? path : `/${path}`);
const PRECACHE = [...new Set([SHELL, ...immutable.map((f) => absolute(f.path)), ...assets.map((f) => absolute(f.path))])];
const PRECACHED = new Set(PRECACHE);

sw.addEventListener('install', (event) => {
	event.waitUntil(
		caches
			.open(CACHE)
			.then((cache) => cache.addAll(PRECACHE))
			.then(() => sw.skipWaiting())
	);
});

sw.addEventListener('activate', (event) => {
	event.waitUntil(
		caches
			.keys()
			.then((keys) => Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k))))
			.then(() => sw.clients.claim())
	);
});

sw.addEventListener('fetch', (event) => {
	const { request } = event;
	if (request.method !== 'GET') return;
	const url = new URL(request.url);
	if (url.origin !== sw.location.origin) return;
	// The API and its event streams go straight to the network: no respondWith.
	if (url.pathname === '/api' || url.pathname.startsWith('/api/')) return;

	if (request.mode === 'navigate') {
		// Online, the server answers; offline, any route is the cached shell.
		// A gateway error (502, 503, 504: a proxy with nothing behind it) counts as offline.
		const shell = async () => (await caches.match(SHELL)) ?? Response.error();
		event.respondWith(
			fetch(request).then(
				(res) => ([502, 503, 504].includes(res.status) ? shell() : res),
				shell
			)
		);
		return;
	}

	if (PRECACHED.has(url.pathname)) {
		event.respondWith(caches.match(request).then((hit) => hit ?? fetch(request)));
	}
});
