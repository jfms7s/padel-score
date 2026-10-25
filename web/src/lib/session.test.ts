import { afterEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { setUnauthorizedHandler } from './api';
import { bootSession, resetSession, session, signedIn, signedOut } from './session';
import me from './fixtures/me-admin.json';
import unauthorized from './fixtures/auth-401-unauthorized.json';

function serve(status: number, body: unknown) {
	const fetch = vi.fn(async (_url: string) => new Response(JSON.stringify(body), { status }));
	vi.stubGlobal('fetch', fetch);
	return fetch;
}

afterEach(() => {
	vi.unstubAllGlobals();
	resetSession();
});

describe('session', () => {
	it('session_boot_signed_out_on_401_quietly', async () => {
		const redirect = vi.fn();
		const restore = setUnauthorizedHandler(redirect);
		try {
			serve(401, unauthorized);
			await expect(bootSession()).resolves.toEqual({ status: 'signed_out' });
			expect(get(session)).toEqual({ status: 'signed_out' });
			expect(redirect).not.toHaveBeenCalled();
		} finally {
			restore();
		}
	});

	it('boots signed in from /api/auth/me', async () => {
		const fetch = serve(200, me);
		expect(get(session)).toEqual({ status: 'loading' });
		await bootSession();
		expect(get(session)).toEqual({ status: 'signed_in', me });
		expect(fetch.mock.calls[0]?.[0]).toBe('/api/auth/me');
	});

	it('loads once, however many callers ask', async () => {
		const fetch = serve(200, me);
		await Promise.all([bootSession(), bootSession()]);
		await bootSession();
		expect(fetch).toHaveBeenCalledTimes(1);
	});

	it('is unavailable, without throwing, when the server cannot be reached', async () => {
		vi.stubGlobal('fetch', async () => {
			throw new TypeError('Failed to fetch');
		});
		await expect(bootSession()).resolves.toEqual({ status: 'unavailable' });
	});

	it('a 5xx from /api/auth/me is unavailable, not signed out', async () => {
		serve(500, { error: 'internal' });
		await expect(bootSession()).resolves.toEqual({ status: 'unavailable' });
		serve(502, '<html>bad gateway</html>');
		await expect(bootSession()).resolves.toEqual({ status: 'unavailable' });
	});

	it('does not keep unavailable: the next call asks again', async () => {
		serve(503, {});
		await bootSession();
		const fetch = serve(200, me);
		await expect(bootSession()).resolves.toEqual({ status: 'signed_in', me });
		expect(fetch).toHaveBeenCalledTimes(1);
		// Once known, it is kept.
		await bootSession();
		expect(fetch).toHaveBeenCalledTimes(1);
	});

	it('asks again when the browser comes back online', async () => {
		const target = new EventTarget();
		vi.stubGlobal('addEventListener', target.addEventListener.bind(target));
		vi.stubGlobal('removeEventListener', target.removeEventListener.bind(target));
		serve(503, {});
		await bootSession();
		const fetch = serve(200, me);
		target.dispatchEvent(new Event('online'));
		await vi.waitFor(() => expect(get(session)).toEqual({ status: 'signed_in', me }));
		expect(fetch).toHaveBeenCalledTimes(1);
		// Only once: a later online event asks nothing.
		target.dispatchEvent(new Event('online'));
		await Promise.resolve();
		expect(fetch).toHaveBeenCalledTimes(1);
	});

	it('treats an answer of the wrong shape as signed out', async () => {
		serve(200, { id: 1 });
		await expect(bootSession()).resolves.toEqual({ status: 'signed_out' });
	});

	it('follows sign-in and sign-out', () => {
		signedIn(me);
		expect(get(session)).toEqual({ status: 'signed_in', me });
		signedOut();
		expect(get(session)).toEqual({ status: 'signed_out' });
	});
});
