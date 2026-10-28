import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './api';
import {
	addRange,
	boardMessage,
	createBoard,
	deleteBoard,
	getBoard,
	joinBoard,
	leaveBoard,
	myBoards,
	removeRange,
	updateBoard
} from './boards';
import { strings } from './strings';

type Call = { url: string; init: RequestInit | undefined };

function serve(status: number, body: unknown): Call[] {
	const calls: Call[] = [];
	vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
		calls.push({ url, init });
		return new Response(status === 204 ? null : JSON.stringify(body), {
			status,
			headers: { 'content-type': 'application/json' }
		});
	});
	return calls;
}

afterEach(() => vi.unstubAllGlobals());

const err = (code: string, field?: string) =>
	new ApiError(code === 'validation' ? 422 : 409, code as never, 'x', false, field === undefined ? {} : { field });

describe('boardMessage', () => {
	it('maps the 409 codes to copy', () => {
		const e = strings.boards.errors;
		expect(boardMessage(err('board_over'))).toBe(e.board_over);
		expect(boardMessage(err('board_full'))).toBe(e.board_full);
		expect(boardMessage(err('too_many_boards'))).toBe(e.too_many_boards);
		expect(boardMessage(err('creator_only'))).toBe(e.creator_only);
		expect(boardMessage(err('too_many_ranges'))).toBe(e.too_many_ranges);
		expect(e.board_over).toBe("This board's week is over.");
		expect(e.too_many_boards).toBe('You have 20 open boards already.');
	});

	it('maps the 422 fields', () => {
		for (const f of ['range', 'title', 'place', 'time_zone', 'week']) {
			expect(boardMessage(err('validation', f))).toBe(strings.boards.fields[f as 'range']);
		}
		expect(strings.boards.fields.range).toBe('Pick times on the half-hour grid inside the week.');
	});

	it('falls back to the account copy', () => {
		expect(boardMessage(new Error('boom'))).toBe(strings.networkError);
		expect(boardMessage(err('internal'))).toBe(strings.serverError(409));
	});
});

describe('boards client', () => {
	const view = { board: { token: 'tok' } };

	it('creates a board with 201', async () => {
		const calls = serve(201, view);
		const body = { title: 'T', place: null, time_zone: 'UTC', starts_at: 1, ends_at: 2 };
		expect(await createBoard(body)).toEqual(view);
		expect(calls[0]?.url).toBe('/api/boards');
		expect(calls[0]?.init?.method).toBe('POST');
		expect(JSON.parse(String(calls[0]?.init?.body))).toEqual(body);
	});

	it('reads a board by its escaped token', async () => {
		const calls = serve(200, view);
		await getBoard('a/b');
		expect(calls[0]?.url).toBe('/api/boards/a%2Fb');
		expect(calls[0]?.init?.method).toBe('GET');
	});

	it('patches, joins and sends ranges with 200', async () => {
		const calls = serve(200, view);
		await updateBoard('t', { title: 'New', place: null });
		await joinBoard('t');
		await addRange('t', 10, 20);
		await removeRange('t', 10, 20);
		expect(calls.map((c) => [c.init?.method, c.url])).toEqual([
			['PATCH', '/api/boards/t'],
			['POST', '/api/boards/t/join'],
			['POST', '/api/boards/t/ranges'],
			['POST', '/api/boards/t/ranges/remove']
		]);
		expect(JSON.parse(String(calls[0]?.init?.body))).toEqual({ title: 'New', place: null });
		expect(JSON.parse(String(calls[2]?.init?.body))).toEqual({ start: 10, end: 20 });
	});

	it('deletes and leaves with 204', async () => {
		const calls = serve(204, null);
		await deleteBoard('t');
		await leaveBoard('t');
		expect(calls.map((c) => [c.init?.method, c.url])).toEqual([
			['DELETE', '/api/boards/t'],
			['POST', '/api/boards/t/leave']
		]);
	});

	it('lists my boards', async () => {
		const calls = serve(200, { items: [{ id: 'b' }] });
		expect(await myBoards()).toEqual([{ id: 'b' }]);
		expect(calls[0]?.url).toBe('/api/me/boards');
	});

	it('raises the server s code on a conflict', async () => {
		serve(409, { error: 'board_full' });
		await expect(joinBoard('t')).rejects.toMatchObject({ code: 'board_full' });
	});
});
