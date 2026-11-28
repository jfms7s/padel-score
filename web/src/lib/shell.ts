// The service worker's answer to a page load (src/service-worker/index.ts). It is plain logic
// over fetch, AbortController, setTimeout and Response, so it runs in the node tests too.

/** How long a page load waits for response headers before the cached shell is served. */
export const NAV_TIMEOUT_MS = 3000;

// A proxy with nothing behind it: the app is as unreachable as with no connection.
const GATEWAY_ERRORS = [502, 503, 504];

type FetchFn = (request: Request, init?: RequestInit) => Promise<Response>;

/**
 * Answers a page load. Online, the server answers; offline, any route is the cached shell (the
 * app is an SPA, so every route is that one document). A gateway error, a failed fetch, or no
 * response headers within `ms` all count as offline. A response whose headers arrived in time
 * is passed through as it streams. With no shell cached there is nothing to fall back on, so
 * the network gets as long as the browser gives it.
 */
export async function navigate(
	request: Request,
	fetchFn: FetchFn,
	shell: () => Promise<Response | undefined>,
	ms = NAV_TIMEOUT_MS
): Promise<Response> {
	const cached = await shell();
	if (!cached) {
		try {
			return await fetchFn(request);
		} catch {
			return Response.error();
		}
	}

	const controller = new AbortController();
	let timer: ReturnType<typeof setTimeout> | undefined;
	const timedOut = new Promise<'timeout'>((resolve) => {
		timer = setTimeout(() => resolve('timeout'), ms);
	});
	try {
		const res = await Promise.race([fetchFn(request, { signal: controller.signal }), timedOut]);
		if (res === 'timeout') {
			controller.abort();
			return cached;
		}
		return GATEWAY_ERRORS.includes(res.status) ? cached : res;
	} catch {
		return cached;
	} finally {
		clearTimeout(timer);
	}
}
