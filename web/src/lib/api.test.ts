import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
	ApiError,
	courtUrl,
	createMatch,
	deleteMatch,
	getScore,
	getView,
	postCommands,
	postFinal,
	putStart,
	replace,
	REQUEST_TIMEOUT_MS,
	accountRequest,
	streamUrl
} from './api';
import type { CommandRequest } from './types';
import { asView } from './fixtures/load';
import created from './fixtures/create-response.json';
import view from './fixtures/view-golden-point-40-40.json';
import conflict from './fixtures/commands-409-conflict.json';
import rejected from './fixtures/commands-422-rejected.json';
import invalid from './fixtures/commands-400-invalid.json';
import notFound from './fixtures/not-found-404.json';
import invalidMatch from './fixtures/create-422-invalid-match.json';

type Call = { url: string; init: RequestInit | undefined };

function serve(status: number, body: unknown): Call[] {
	const calls: Call[] = [];
	vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
		calls.push({ url, init });
		return new Response(JSON.stringify(body), {
			status,
			headers: { 'content-type': 'application/json' }
		});
	});
	return calls;
}

const REQ: CommandRequest = {
	client_id: 'c1',
	batch: false,
	base_revision: 6,
	commands: [{ kind: 'point', team: 'a' }]
};

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('api', () => {
	it('creates a match', async () => {
		const calls = serve(201, created);
		const { format, start, teams } = asView(view);
		const body = { format, start, teams };
		expect(await createMatch(body)).toEqual(created);
		expect(calls[0]?.url).toBe('/api/matches');
		expect(calls[0]?.init?.method).toBe('POST');
		expect(JSON.parse(String(calls[0]?.init?.body))).toEqual(body);
	});

	it('throws the server error code when a match is refused', async () => {
		serve(422, invalidMatch);
		const err = await createMatch({} as never).catch((e: unknown) => e);
		expect(err).toBeInstanceOf(ApiError);
		expect((err as ApiError).status).toBe(422);
		expect((err as ApiError).code).toBe('invalid_match');
	});

	it('reads the scorer and spectator views with an encoded token', async () => {
		const calls = serve(200, view);
		expect(await getScore('ab/c')).toEqual(view);
		expect(await getView('xyz')).toEqual(view);
		expect(calls.map((c) => c.url)).toEqual(['/api/score/ab%2Fc', '/api/view/xyz']);
	});

	it('throws not_found for an unknown token', async () => {
		serve(404, notFound);
		await expect(getView('nope')).rejects.toMatchObject({ status: 404, code: 'not_found' });
	});

	it('reports an applied command with the new view', async () => {
		const calls = serve(200, view);
		expect(await postCommands('tok', REQ)).toEqual({ status: 'applied', view });
		expect(calls[0]?.url).toBe('/api/score/tok/commands');
		expect(JSON.parse(String(calls[0]?.init?.body))).toEqual(REQ);
	});

	it('reports a conflict with the current view', async () => {
		serve(409, conflict);
		expect(await postCommands('tok', REQ)).toEqual({ status: 'conflict', view: conflict.view });
	});

	it('reports a rejected command with its kind and index', async () => {
		serve(422, rejected);
		expect(await postCommands('tok', REQ)).toEqual({ status: 'rejected', kind: 'match_over', index: 0 });
	});

	it('throws on an invalid request', async () => {
		serve(400, invalid);
		await expect(postCommands('tok', REQ)).rejects.toMatchObject({
			status: 400,
			code: 'invalid',
			message: 'commands must not be empty'
		});
	});

	it('replaces the tail with the same outcome shape', async () => {
		const calls = serve(409, conflict);
		const req = { client_id: 'c2', base_revision: 1, base_seq: 0, commands: [] };
		expect(await replace('tok', req)).toEqual({ status: 'conflict', view: conflict.view });
		expect(calls[0]?.url).toBe('/api/score/tok/replace');
	});

	it('turns a network failure into a retryable status-0 error', async () => {
		vi.stubGlobal('fetch', async () => {
			throw new TypeError('Failed to fetch');
		});
		await expect(getScore('tok')).rejects.toMatchObject({ status: 0, code: 'network', retryable: true });
	});

	it.each([
		[502, '<html>bad gateway</html>'],
		[503, '<html>service unavailable</html>'],
		[504, ''],
		[503, '{"error":"internal"}']
	])('treats a %i gateway answer as retryable, whatever the body', async (status, body) => {
		vi.stubGlobal('fetch', async () => new Response(body, { status }));
		await expect(postCommands('tok', REQ)).rejects.toMatchObject({
			status,
			code: 'network',
			retryable: true
		});
	});

	it('treats a body that drops on a 200 as retryable', async () => {
		const broken = new ReadableStream({
			start(c) {
				c.enqueue(new TextEncoder().encode('{"id":'));
				c.error(new TypeError('network connection lost'));
			}
		});
		vi.stubGlobal('fetch', async () => new Response(broken, { status: 200 }));
		await expect(postCommands('tok', REQ)).rejects.toMatchObject({
			status: 200,
			code: 'network',
			retryable: true
		});
	});

	it('treats a truncated 200 body as retryable', async () => {
		vi.stubGlobal('fetch', async () => new Response('{"id":"x","form', { status: 200 }));
		await expect(getScore('tok')).rejects.toMatchObject({ code: 'network', retryable: true });
	});

	it('keeps a real JSON 500 on a read internal and not retryable', async () => {
		serve(500, { error: 'internal' });
		await expect(getScore('tok')).rejects.toMatchObject({
			status: 500,
			code: 'internal',
			retryable: false
		});
	});

	it.each([
		[500, '{"error":"internal"}'],
		[500, '<html>oops</html>'],
		[507, '']
	])('treats any %i on a command as retryable: client_id makes the resend safe', async (status, body) => {
		vi.stubGlobal('fetch', async () => new Response(body, { status }));
		await expect(postCommands('tok', REQ)).rejects.toMatchObject({ status, code: 'network', retryable: true });
		await expect(replace('tok', { client_id: 'c', base_revision: 1, base_seq: 1, commands: [] })).rejects.toMatchObject(
			{ status, code: 'network', retryable: true }
		);
	});

	describe('timeouts', () => {
		beforeEach(() => {
			vi.useFakeTimers();
		});
		afterEach(() => {
			vi.useRealTimers();
		});

		/** A fetch that never answers, but gives up when its signal aborts, as fetch does. */
		function hang() {
			const signals: AbortSignal[] = [];
			vi.stubGlobal('fetch', (_url: string, init?: RequestInit) => {
				const signal = init?.signal;
				if (signal) signals.push(signal);
				return new Promise((_, reject) => {
					signal?.addEventListener('abort', () => reject(new DOMException('timed out', 'TimeoutError')));
				});
			});
			return signals;
		}

		it.each([
			['postCommands', () => postCommands('tok', REQ)],
			['getScore', () => getScore('tok')],
			['getView', () => getView('tok')]
		])('%s gives up after 10 s as a retryable network failure', async (_name, run) => {
			const signals = hang();
			const out = run().catch((e: unknown) => e);
			await vi.advanceTimersByTimeAsync(REQUEST_TIMEOUT_MS - 1);
			expect(signals[0]?.aborted).toBe(false);
			await vi.advanceTimersByTimeAsync(1);
			expect(await out).toMatchObject({ status: 0, code: 'network', retryable: true });
		});
	});

	it('keeps a non-JSON error page that is not a gateway error not retryable', async () => {
		vi.stubGlobal('fetch', async () => new Response('<html>oops</html>', { status: 500 }));
		await expect(getScore('tok')).rejects.toMatchObject({ code: 'internal', retryable: false });
	});

	it('server refusals are not retryable', async () => {
		serve(400, invalid);
		await expect(postCommands('tok', REQ)).rejects.toMatchObject({ retryable: false });
		serve(404, notFound);
		await expect(getView('x')).rejects.toMatchObject({ retryable: false });
	});

	it('builds the stream URLs', () => {
		expect(streamUrl('score', 'a b')).toBe('/api/score/a%20b/stream');
		expect(streamUrl('view', 't')).toBe('/api/view/t/stream');
	});
});

describe('typed results and the serve choice', () => {
	afterEach(() => vi.unstubAllGlobals());
	const v = asView(view);
	const FINAL = { client_id: 'f1', base_revision: 3, score: { sets: [{ a: 6, b: 4, tiebreak: null }] } };
	const START = { first_server: { team: 'b' as const, index: 1 }, other_first_server: 0 };

	it('postFinal sends the typed score and reads every answer', async () => {
		const calls = serve(200, v);
		await expect(postFinal('t/1', FINAL)).resolves.toEqual({ status: 'applied', view: v });
		expect(calls[0]!.url).toBe('/api/score/t%2F1/final');
		expect(calls[0]!.init?.method).toBe('POST');
		expect(JSON.parse(calls[0]!.init?.body as string)).toEqual(FINAL);
		serve(409, { error: 'conflict', view: v });
		await expect(postFinal('t', FINAL)).resolves.toEqual({ status: 'conflict', view: v });
		serve(409, { error: 'locked', view: { ...v, locked: true } });
		await expect(postFinal('t', FINAL)).resolves.toEqual({ status: 'locked', view: { ...v, locked: true } });
		serve(422, { error: 'invalid_score', reason: 'match_not_finished' });
		await expect(postFinal('t', FINAL)).resolves.toEqual({ status: 'invalid_score', reason: 'match_not_finished' });
		serve(503, {});
		await expect(postFinal('t', FINAL)).rejects.toMatchObject({ code: 'network', retryable: true });
	});

	it('a locked command is an answer too', async () => {
		serve(409, { error: 'locked', view: { ...v, locked: true } });
		await expect(postCommands('t', REQ)).resolves.toEqual({ status: 'locked', view: { ...v, locked: true } });
	});

	it('putStart sends the start and reads locked and already started', async () => {
		const calls = serve(200, v);
		await expect(putStart('t', START)).resolves.toEqual({ status: 'applied', view: v });
		expect(calls[0]!.url).toBe('/api/score/t/start');
		expect(calls[0]!.init?.method).toBe('PUT');
		expect(JSON.parse(calls[0]!.init?.body as string)).toEqual({ start: START });
		serve(409, { error: 'locked', view: v });
		await expect(putStart('t', START)).resolves.toEqual({ status: 'locked', view: v });
		serve(409, { error: 'already_started' });
		const err = await putStart('t', START).catch((e: unknown) => e);
		expect(err).toBeInstanceOf(ApiError);
		expect(err).toMatchObject({ status: 409, code: 'already_started' });
	});

	it('courtUrl', () => {
		expect(courtUrl('a b')).toBe('/api/court/a%20b');
		expect(courtUrl('t', '/advance')).toBe('/api/court/t/advance');
	});

	describe('deleteMatch', () => {
		it('deletes by the scorer token', async () => {
			const calls: Call[] = [];
			vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
				calls.push({ url, init });
				return new Response(null, { status: 204 });
			});
			expect(await deleteMatch('s/1')).toBe('deleted');
			expect(calls[0]?.url).toBe('/api/score/s%2F1');
			expect(calls[0]?.init?.method).toBe('DELETE');
		});

		it('counts a match already gone as deleted', async () => {
			serve(404, notFound);
			expect(await deleteMatch('s1')).toBe('deleted');
		});

		it('reports a finished match as kept', async () => {
			serve(409, { error: 'finished' });
			expect(await deleteMatch('s1')).toBe('finished');
		});

		it("throws for an event's match", async () => {
			serve(409, { error: 'event_match' });
			await expect(deleteMatch('s1')).rejects.toMatchObject({ status: 409, code: 'event_match' });
		});
	});

	it('accountRequest takes a 202 with no body as success', async () => {
		vi.stubGlobal('fetch', async () => new Response('', { status: 202 }));
		expect(await accountRequest('/api/me/push/test', { method: 'POST', ok: 202 })).toBeNull();
	});
});
