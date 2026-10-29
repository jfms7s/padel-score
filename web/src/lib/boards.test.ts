import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError, setUnauthorizedHandler } from './api';
import {
	addRange,
	answerBoardRequest,
	answerRequest,
	askSwap,
	askTime,
	boardMessage,
	createBoard,
	deleteBoard,
	getBoard,
	giveGame,
	joinBoard,
	leaveBoard,
	leaveGame,
	myBoards,
	removeRange,
	startGame,
	takeGame,
	updateBoard
} from './boards';
import { rememberGuest } from './guests';
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

	it('gives every game action code its own sentence', () => {
		const codes = [
			'game_changed',
			'nobody_can_take',
			'too_late',
			'cannot_swap',
			'players_busy',
			'too_soon',
			'request_pending',
			'request_expired',
			'already_answered',
			'not_yet',
			'game_over',
			'already_started',
			'not_in_game',
			'game_played',
			'game_started',
			'already_in_game',
			'game_full',
			'busy'
		];
		const texts = codes.map((c) => boardMessage(err(c)));
		expect(texts.every((t) => t !== strings.serverError(409))).toBe(true);
		expect(new Set(texts).size).toBe(codes.length);
		expect(boardMessage(err('game_changed'))).toBe('This game just changed');
		expect(boardMessage(err('nobody_can_take'))).toBe('Nobody can take your place right now. Use Leave instead.');
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

	it('sends a game action with the game s version', async () => {
		const calls = serve(200, view);
		await leaveGame('t', 'g/1', 3);
		await giveGame('t', 'g1', 4);
		await takeGame('t', 'g1', 5);
		expect(calls.map((c) => [c.init?.method, c.url])).toEqual([
			['POST', '/api/boards/t/games/g%2F1/leave'],
			['POST', '/api/boards/t/games/g1/give'],
			['POST', '/api/boards/t/games/g1/take']
		]);
		expect(calls.map((c) => JSON.parse(String(c.init?.body)))).toEqual([
			{ version: 3 },
			{ version: 4 },
			{ version: 5 }
		]);
	});

	it('asks for a swap and a time change', async () => {
		const calls = serve(200, view);
		await askSwap('t', 'g1', 2, 'g2', 'rui');
		await askTime('t', 'g1', 2, 100, 5500, true);
		await askTime('t', 'g1', 2, 100, 3700, false);
		expect(calls.map((c) => c.url)).toEqual([
			'/api/boards/t/games/g1/swap',
			'/api/boards/t/games/g1/time',
			'/api/boards/t/games/g1/time'
		]);
		expect(calls.map((c) => JSON.parse(String(c.init?.body)))).toEqual([
			{ version: 2, other_game_id: 'g2', other_user_id: 'rui' },
			{ version: 2, start: 100, end: 5500, agreed: true },
			{ version: 2, start: 100, end: 3700, agreed: false }
		]);
	});

	it('answers a request with no body', async () => {
		const calls = serve(200, view);
		await answerRequest('r/1', true);
		await answerRequest('r1', false);
		expect(calls.map((c) => [c.init?.method, c.url, c.init?.body])).toEqual([
			['POST', '/api/board-requests/r%2F1/accept', undefined],
			['POST', '/api/board-requests/r1/decline', undefined]
		]);
	});

	it('starts a match from a game and returns its players', async () => {
		const players = { players: [], format: null, from: { kind: 'game', id: 'g1' } };
		const calls = serve(200, players);
		expect(await startGame('t', 'g1')).toEqual(players);
		expect(calls[0]?.url).toBe('/api/boards/t/games/g1/start');
		expect(calls[0]?.init?.method).toBe('POST');
		expect(calls[0]?.init?.body).toBeUndefined();
	});

	it('raises game_changed as a code the sheet can tell apart', async () => {
		serve(409, { error: 'game_changed' });
		await expect(giveGame('t', 'g1', 1)).rejects.toMatchObject({ code: 'game_changed' });
	});
});

describe('board calls as a guest', () => {
	const store = () => {
		const data = new Map<string, string>();
		return {
			getItem: (k: string) => data.get(k) ?? null,
			setItem: (k: string, v: string) => void data.set(k, v),
			removeItem: (k: string) => void data.delete(k)
		};
	};

	it('sends the guest key and does not treat a 401 as a sign-out', async () => {
		const s = store();
		vi.stubGlobal('localStorage', s);
		rememberGuest('t', 'secret', 'guest_1', s);
		const calls = serve(401, { error: 'unauthorized' });
		const handler = vi.fn();
		const restore = setUnauthorizedHandler(handler);
		try {
			await expect(addRange('t', 1, 2)).rejects.toBeInstanceOf(ApiError);
		} finally {
			restore();
		}
		const headers = calls[0]?.init?.headers as Record<string, string>;
		expect(headers['x-board-guest']).toBe('secret');
		expect(handler).not.toHaveBeenCalled();
	});

	it('sends no key without one and still bounces to sign-in on a 401', async () => {
		vi.stubGlobal('localStorage', store());
		const calls = serve(401, { error: 'unauthorized' });
		const handler = vi.fn();
		const restore = setUnauthorizedHandler(handler);
		try {
			await expect(addRange('t', 1, 2)).rejects.toBeInstanceOf(ApiError);
		} finally {
			restore();
		}
		const headers = (calls[0]?.init?.headers ?? {}) as Record<string, string>;
		expect(headers['x-board-guest']).toBeUndefined();
		expect(handler).toHaveBeenCalledTimes(1);
	});

	it('forgets the key after leaving the board', async () => {
		const s = store();
		vi.stubGlobal('localStorage', s);
		rememberGuest('t', 'secret', 'guest_1', s);
		serve(204, null);
		await leaveBoard('t');
		expect(s.getItem('padel.boardGuest.t')).toBeNull();
	});

	it('answers a request under the board link', async () => {
		vi.stubGlobal('localStorage', store());
		const calls = serve(200, {});
		await answerBoardRequest('t', 'r 1', true);
		expect(calls[0]?.url).toBe('/api/boards/t/requests/r%201/accept');
		expect(calls[0]?.init?.method).toBe('POST');
		await answerBoardRequest('t', 'r1', false);
		expect(calls[1]?.url).toBe('/api/boards/t/requests/r1/decline');
	});

	it('has copy for the guest codes', () => {
		const e = strings.boards.errors;
		expect(boardMessage(err('guests_off'))).toBe(e.guests_off);
		expect(boardMessage(err('guest_name_taken'))).toBe(e.guest_name_taken);
		expect(e.guests_off).not.toBe('');
	});
});
