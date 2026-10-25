// Who is signed in, loaded once from /api/auth/me when the app starts. Quietly: a 401 just
// means signed out, and an unreachable server leaves it unknown; neither is an error to show.
// Pages and the header read the `session` store; sign-in and sign-out pages update it.

import { writable, type Readable } from 'svelte/store';
import { ApiError, redirectToSignIn } from './api';
import { getMe } from './auth';
import type { Me } from './types';

export type Session =
	| { status: 'loading' }
	| { status: 'signed_in'; me: Me }
	| { status: 'signed_out' }
	/** The server couldn't be reached (offline at the court); the user may well be signed in. */
	| { status: 'unavailable' };

const LOADING: Session = { status: 'loading' };

const store = writable<Session>(LOADING);

function update(s: Session): Session {
	store.set(s);
	return s;
}

export const session: Readable<Session> = { subscribe: store.subscribe };

let booting: Promise<Session> | null = null;

/** The server didn't answer the question (offline, restarting, failing): ask again later. */
const unanswered = (e: unknown) => !(e instanceof ApiError) || e.code === 'network' || e.status >= 500;

let retryArmed = false;

/** Ask again the next time the browser comes back online. */
function retryWhenOnline(): void {
	if (retryArmed || typeof globalThis.addEventListener !== 'function') return;
	retryArmed = true;
	const again = () => {
		globalThis.removeEventListener('online', again);
		retryArmed = false;
		if (booting === null) void bootSession();
	};
	globalThis.addEventListener('online', again);
}

/**
 * Load the session once; later calls share the answer. `unavailable` is not kept: the next call,
 * or the browser coming back online, asks again.
 */
export function bootSession(): Promise<Session> {
	if (booting) return booting;
	const attempt: Promise<Session> = getMe().then(
		(me) => update({ status: 'signed_in', me }),
		(e: unknown) => {
			if (!unanswered(e)) return update({ status: 'signed_out' });
			if (booting === attempt) booting = null;
			retryWhenOnline();
			return update({ status: 'unavailable' });
		}
	);
	booting = attempt;
	return attempt;
}

/** After setup, sign-in or accepting an invite. */
export function signedIn(me: Me): void {
	booting = Promise.resolve(update({ status: 'signed_in', me }));
}

/** After sign-out, or once the server says the session is gone. */
export function signedOut(): void {
	booting = Promise.resolve(update({ status: 'signed_out' }));
}

/** Ask the server again, keeping what is shown until it answers (after a reset, say). */
export function refreshSession(): Promise<Session> {
	booting = null;
	return bootSession();
}

/**
 * For a page that needs a user: the session once known. Signed out, the browser goes to sign-in
 * (coming back after); `unavailable` is left to the page to show.
 */
export async function requireSignedIn(): Promise<Session> {
	const s = await bootSession();
	if (s.status === 'signed_out') redirectToSignIn();
	return s;
}

/** Forget everything (tests). */
export function resetSession(): void {
	booting = null;
	retryArmed = false;
	update(LOADING);
}
