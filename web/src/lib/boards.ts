// Game boards: the calls, and the words for what went wrong. Everything but reading a board needs
// a signed-in user; reading works for anyone with the link, so `getBoard` does not bounce a
// signed-out visitor to sign-in.

import { accountRequest, ApiError } from './api';
import { authMessage } from './auth';
import { forgetGuest, guestKey } from './guests';
import { strings } from './strings';
import type { BoardChange, BoardOut, BoardView, NewBoard, StartPlayers } from './types';

const enc = encodeURIComponent;
const board = (token: string, rest = '') => `/api/boards/${enc(token)}${rest}`;
const signedIn = { signedIn: true } as const;

/**
 * What a guest-capable call sends: the guest key when this browser holds one for the board (and
 * then no `signedIn`, so a 401 doesn't send a guest to sign-in), else the signed-in session.
 */
function asMember(token: string): { signedIn: true } | { headers: Record<string, string> } {
	const key = guestKey(token);
	return key === null ? signedIn : { headers: { 'x-board-guest': key } };
}

/** 409 `too_many_boards` (20 open ones), 422 `title`, `place`, `time_zone`, `week`. */
export function createBoard(req: NewBoard): Promise<BoardView> {
	return accountRequest('/api/boards', { ...signedIn, method: 'POST', body: req, ok: 201 });
}

/** Anyone with the link; signed out, `me` is null. */
export function getBoard(token: string): Promise<BoardView> {
	const key = guestKey(token);
	return accountRequest(board(token), key === null ? {} : { headers: { 'x-board-guest': key } });
}

/** The creator only (409 `creator_only`). */
export function updateBoard(token: string, change: BoardChange): Promise<BoardView> {
	return accountRequest(board(token), { ...signedIn, method: 'PATCH', body: change });
}

/** The creator only; players with games are told. */
export async function deleteBoard(token: string): Promise<void> {
	await accountRequest(board(token), { ...signedIn, method: 'DELETE', ok: 204 });
}

/** 409 `board_full` (64 members), `board_over`. */
export function joinBoard(token: string): Promise<BoardView> {
	return accountRequest(board(token, '/join'), { ...signedIn, method: 'POST' });
}

export async function leaveBoard(token: string): Promise<void> {
	const wasGuest = guestKey(token) !== null;
	await accountRequest(board(token, '/leave'), { ...asMember(token), method: 'POST', ok: 204 });
	if (wasGuest) forgetGuest(token);
}

/** Adding free time joins the board too. 409 `too_many_ranges`, 422 `range`. */
export function addRange(token: string, start: number, end: number): Promise<BoardView> {
	return accountRequest(board(token, '/ranges'), { ...asMember(token), method: 'POST', body: { start, end } });
}

export function removeRange(token: string, start: number, end: number): Promise<BoardView> {
	return accountRequest(board(token, '/ranges/remove'), { ...asMember(token), method: 'POST', body: { start, end } });
}

// ---- game actions ------------------------------------------------------------------------
// Each sends the game's `version` from the view the player saw: a stale one is 409
// `game_changed`, and nothing moves. They answer with the board as it is after.

const game = (token: string, gameId: string, action: string) => board(token, `/games/${enc(gameId)}/${action}`);

/** 409 `game_changed`, `game_played`, `game_started`, `board_over`. Removes my free time for it too. */
export function leaveGame(token: string, gameId: string, version: number): Promise<BoardView> {
	return accountRequest(game(token, gameId, 'leave'), { ...asMember(token), method: 'POST', body: { version } });
}

/** 409 `nobody_can_take`, `too_late`. */
export function giveGame(token: string, gameId: string, version: number): Promise<BoardView> {
	return accountRequest(game(token, gameId, 'give'), { ...asMember(token), method: 'POST', body: { version } });
}

/** Take an open place in a short game: 409 `busy`, `game_full`, `already_in_game`. */
export function takeGame(token: string, gameId: string, version: number): Promise<BoardView> {
	return accountRequest(game(token, gameId, 'take'), { ...asMember(token), method: 'POST', body: { version } });
}

/** Ask a player of an overlapping game to trade places: 409 `cannot_swap`, `request_pending`. */
export function askSwap(
	token: string,
	gameId: string,
	version: number,
	otherGameId: string,
	otherUserId: string
): Promise<BoardView> {
	const body = { version, other_game_id: otherGameId, other_user_id: otherUserId };
	return accountRequest(game(token, gameId, 'swap'), { ...asMember(token), method: 'POST', body });
}

/**
 * Propose `[start, end)` for the game, or with `agreed` move it at once and only tell the others.
 * 409 `too_soon`, `players_busy`, `request_pending`, 422 `range`.
 */
export function askTime(
	token: string,
	gameId: string,
	version: number,
	start: number,
	end: number,
	agreed: boolean
): Promise<BoardView> {
	return accountRequest(game(token, gameId, 'time'), {
		...asMember(token),
		method: 'POST',
		body: { version, start, end, agreed }
	});
}

/** Accept or decline a request addressed to me; answers with the request's board. */
export function answerRequest(requestId: string, accept: boolean): Promise<BoardView> {
	return accountRequest(`/api/board-requests/${enc(requestId)}/${accept ? 'accept' : 'decline'}`, {
		...signedIn,
		method: 'POST'
	});
}

/** Accept or decline a request addressed to me, as a member or a guest of the board. */
export function answerBoardRequest(token: string, requestId: string, accept: boolean): Promise<BoardView> {
	return accountRequest(board(token, `/requests/${enc(requestId)}/${accept ? 'accept' : 'decline'}`), {
		...asMember(token),
		method: 'POST'
	});
}

/** The game's players for New match (a short game has fewer than four): 409 `not_yet`, `game_over`, `already_started`. */
export function startGame(token: string, gameId: string): Promise<StartPlayers> {
	return accountRequest(game(token, gameId, 'start'), { ...signedIn, method: 'POST' });
}

/** The boards I am a member of or made, soonest week first. */
export async function myBoards(): Promise<BoardOut[]> {
	const page = await accountRequest<{ items: BoardOut[] }>('/api/me/boards', signedIn);
	return page.items;
}

/** What to tell the user about a failed board call. */
export function boardMessage(e: unknown): string {
	if (e instanceof ApiError) {
		const errors: Record<string, string> = strings.boards.errors;
		const fields: Record<string, string> = strings.boards.fields;
		if (e.code === 'validation' && e.field !== undefined && Object.hasOwn(fields, e.field)) return fields[e.field]!;
		if (Object.hasOwn(errors, e.code)) return errors[e.code]!;
	}
	return authMessage(e);
}
