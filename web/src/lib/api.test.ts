import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, createMatch, getScore, getView, postCommands, replace, streamUrl } from './api';
import type { CommandRequest, MatchView } from './types';
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
		const body = {
			format: (view as MatchView).format,
			start: (view as MatchView).start,
			teams: (view as MatchView).teams
		};
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

	it('turns a network failure into a status-0 error', async () => {
		vi.stubGlobal('fetch', async () => {
			throw new TypeError('Failed to fetch');
		});
		await expect(getScore('tok')).rejects.toMatchObject({ status: 0, code: 'network' });
	});

	it('turns a non-JSON answer into an error', async () => {
		vi.stubGlobal('fetch', async () => new Response('<html>bad gateway</html>', { status: 502 }));
		await expect(getScore('tok')).rejects.toMatchObject({ status: 502, code: 'internal' });
	});

	it('builds the stream URLs', () => {
		expect(streamUrl('score', 'a b')).toBe('/api/score/a%20b/stream');
		expect(streamUrl('view', 't')).toBe('/api/view/t/stream');
	});
});
