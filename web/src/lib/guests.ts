// Guests on a game board: the key a guest keeps in this browser, and the calls that make or
// settle one. The key is the only proof of being that guest, so it lives under the board's token
// in localStorage and is never put in a URL.

import { accountRequest, ApiError } from './api';
import type { StorageLike } from './setup';
import { safeStorage } from './storage';
import { strings } from './strings';
import type { BoardView, GuestJoined } from './types';

export const GUEST_KEY_PREFIX = 'padel.boardGuest.';

const enc = encodeURIComponent;
const guests = (token: string, rest = '') => `/api/boards/${enc(token)}/guests${rest}`;
const signedIn = { signedIn: true } as const;

interface Remembered {
	key: string;
	id: string;
}

function remembered(token: string, storage: StorageLike): Remembered | null {
	try {
		const raw = storage.getItem(GUEST_KEY_PREFIX + token);
		if (raw === null) return null;
		const v: unknown = JSON.parse(raw);
		if (typeof v !== 'object' || v === null) return null;
		const { key, id } = v as Record<string, unknown>;
		return typeof key === 'string' && typeof id === 'string' ? { key, id } : null;
	} catch {
		return null;
	}
}

/** The secret this browser holds as a guest of the board, or null. */
export function guestKey(token: string, storage: StorageLike = safeStorage()): string | null {
	return remembered(token, storage)?.key ?? null;
}

/** The id of the guest this browser is on the board, or null. */
export function rememberedGuestId(token: string, storage: StorageLike = safeStorage()): string | null {
	return remembered(token, storage)?.id ?? null;
}

export function rememberGuest(token: string, key: string, id: string, storage: StorageLike = safeStorage()): void {
	try {
		storage.setItem(GUEST_KEY_PREFIX + token, JSON.stringify({ key, id }));
	} catch {
		// Storage is full or blocked: the guest works for this visit only.
	}
}

export function forgetGuest(token: string, storage: StorageLike = safeStorage()): void {
	try {
		storage.removeItem(GUEST_KEY_PREFIX + token);
	} catch {
		// Nothing to forget in a store that cannot be reached.
	}
}

/** Keep the key with the guest id the view names; a view without `me` is a server error. */
function keep(token: string, joined: GuestJoined): GuestJoined {
	const me = joined.view.me;
	if (!me) throw new ApiError(500, 'internal', strings.serverError(500));
	rememberGuest(token, joined.guest_key, me.user_id);
	return joined;
}

/** Join the board under a name, signed out. 409 `guests_off`, `guest_name_taken`, `board_full`, `board_over`; 422 `name`. */
export async function joinAsGuest(token: string, name: string): Promise<GuestJoined> {
	const joined = await accountRequest<GuestJoined>(guests(token), { method: 'POST', body: { name }, ok: 201 });
	return keep(token, joined);
}

/** Take an existing guest's name ("I'm already on this board"), and its key. */
export async function pickGuest(token: string, guestId: string): Promise<GuestJoined> {
	const joined = await accountRequest<GuestJoined>(guests(token, `/${enc(guestId)}/pick`), { method: 'POST' });
	return keep(token, joined);
}

/** A signed-in member removes a guest; their times and places go. */
export function removeGuest(token: string, guestId: string): Promise<BoardView> {
	return accountRequest(guests(token, `/${enc(guestId)}`), { ...signedIn, method: 'DELETE' });
}

/** A signed-in user takes over a guest's times and places; this browser stops being that guest. */
export async function claimGuest(token: string, guestId: string): Promise<BoardView> {
	const view = await accountRequest<BoardView>(guests(token, `/${enc(guestId)}/claim`), {
		...signedIn,
		method: 'POST'
	});
	if (rememberedGuestId(token) === guestId) forgetGuest(token);
	return view;
}
