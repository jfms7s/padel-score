import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { NAV_TIMEOUT_MS, navigate } from './shell';

const request = () => new Request('https://fence.test/m/abc');
const shellPage = () => new Response('<!doctype html>shell', { status: 200 });
const page = (status = 200) => new Response('<!doctype html>network', { status });

/** A fetch that accepts the connection and never answers, unless it is aborted. */
function stalledFetch() {
	const seen: { signal?: AbortSignal } = {};
	const fn = vi.fn(
		(_r: Request, init?: RequestInit) =>
			new Promise<Response>((_resolve, reject) => {
				seen.signal = init?.signal ?? undefined;
				init?.signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')));
			})
	);
	return { fn, seen };
}

beforeEach(() => {
	vi.useFakeTimers();
});

afterEach(() => {
	vi.useRealTimers();
});

describe('navigate', () => {
	it('serves_the_cached_shell_when_the_network_does_not_answer_in_time', async () => {
		const shell = shellPage();
		const { fn, seen } = stalledFetch();
		const answer = navigate(request(), fn, async () => shell);

		await vi.advanceTimersByTimeAsync(NAV_TIMEOUT_MS - 1);
		let settled = false;
		void answer.then(() => (settled = true));
		await Promise.resolve();
		expect(settled).toBe(false);

		await vi.advanceTimersByTimeAsync(1);
		expect(await answer).toBe(shell);
		expect(seen.signal?.aborted).toBe(true);
		expect(vi.getTimerCount()).toBe(0);
	});

	it('passes_a_timely_answer_through', async () => {
		const res = page();
		let signal: AbortSignal | undefined;
		const fn = vi.fn(async (_r: Request, init?: RequestInit) => {
			signal = init?.signal ?? undefined;
			return res;
		});
		expect(await navigate(request(), fn, async () => shellPage())).toBe(res);
		expect(vi.getTimerCount()).toBe(0);
		// The body is still streaming: the timer must not abort it once headers are in.
		await vi.advanceTimersByTimeAsync(NAV_TIMEOUT_MS * 2);
		expect(signal?.aborted).toBe(false);
	});

	it('serves_the_shell_on_a_gateway_error', async () => {
		for (const status of [502, 503, 504]) {
			const shell = shellPage();
			expect(
				await navigate(
					request(),
					async () => page(status),
					async () => shell
				)
			).toBe(shell);
		}
		// Any other error is the server's real answer.
		const notFound = page(404);
		expect(
			await navigate(
				request(),
				async () => notFound,
				async () => shellPage()
			)
		).toBe(notFound);
		expect(vi.getTimerCount()).toBe(0);
	});

	it('serves_the_shell_when_the_fetch_fails', async () => {
		const shell = shellPage();
		const fn = vi.fn(async () => {
			throw new TypeError('Failed to fetch');
		});
		expect(await navigate(request(), fn, async () => shell)).toBe(shell);
		expect(vi.getTimerCount()).toBe(0);
	});

	it('waits_for_the_network_when_no_shell_is_cached', async () => {
		let answer: (r: Response) => void = () => {};
		const fn = vi.fn(
			(_r: Request, init?: RequestInit) =>
				new Promise<Response>((resolve) => {
					expect(init?.signal).toBeUndefined();
					answer = resolve;
				})
		);
		const pending = navigate(request(), fn, async () => undefined);

		await vi.advanceTimersByTimeAsync(NAV_TIMEOUT_MS * 10);
		const res = page();
		answer(res);
		expect(await pending).toBe(res);
	});

	it('returns_a_network_error_when_no_shell_is_cached_and_the_fetch_fails', async () => {
		const fn = vi.fn(async () => {
			throw new TypeError('Failed to fetch');
		});
		const res = await navigate(request(), fn, async () => undefined);
		expect(res.type).toBe('error');
	});

	it('passes_a_gateway_error_through_when_no_shell_is_cached', async () => {
		const res = page(503);
		expect(
			await navigate(
				request(),
				async () => res,
				async () => undefined
			)
		).toBe(res);
	});

	it('waits_for_the_network_when_the_cache_cannot_be_read', async () => {
		const res = page();
		const fn = vi.fn(async () => res);
		const broken = async (): Promise<Response | undefined> => {
			throw new DOMException('broken', 'UnknownError');
		};
		expect(await navigate(request(), fn, broken)).toBe(res);
	});
});
