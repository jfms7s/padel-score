// Game boards: the calls, and the words for what went wrong. Everything but reading a board needs
// a signed-in user; reading works for anyone with the link, so `getBoard` does not bounce a
// signed-out visitor to sign-in.

import { accountRequest, ApiError } from './api';
import { authMessage } from './auth';
import { strings } from './strings';
import type { BoardChange, BoardOut, BoardView, NewBoard } from './types';

const enc = encodeURIComponent;
const board = (token: string, rest = '') => `/api/boards/${enc(token)}${rest}`;
const signedIn = { signedIn: true } as const;

/** 409 `too_many_boards` (20 open ones), 422 `title`, `place`, `time_zone`, `week`. */
export function createBoard(req: NewBoard): Promise<BoardView> {
	return accountRequest('/api/boards', { ...signedIn, method: 'POST', body: req, ok: 201 });
}

/** Anyone with the link; signed out, `me` is null. */
export function getBoard(token: string): Promise<BoardView> {
	return accountRequest(board(token));
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
	await accountRequest(board(token, '/leave'), { ...signedIn, method: 'POST', ok: 204 });
}

/** Adding free time joins the board too. 409 `too_many_ranges`, 422 `range`. */
export function addRange(token: string, start: number, end: number): Promise<BoardView> {
	return accountRequest(board(token, '/ranges'), { ...signedIn, method: 'POST', body: { start, end } });
}

export function removeRange(token: string, start: number, end: number): Promise<BoardView> {
	return accountRequest(board(token, '/ranges/remove'), { ...signedIn, method: 'POST', body: { start, end } });
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
