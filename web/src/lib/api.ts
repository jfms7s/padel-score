// The match API (crates/server/src/api.rs). Paths are relative: in production the same binary
// serves the app and the API, and in dev vite proxies /api to it.

import { loginPath } from './next';
import { strings } from './strings';
import type {
	ApiErrorBody,
	CommandRequest,
	CreatedMatch,
	CreateMatchRequest,
	ErrorKind,
	MatchView,
	ReplaceRequest
} from './types';

/** What some refusals add: the field a 422 names, the wait a 429 asks for. */
export interface ErrorDetail {
	field?: string;
	retryAfterSecs?: number;
}

/**
 * A request that failed. `retryable` means the server may never have seen or answered it (no
 * connection, a gateway error while it restarts, a dropped body), so a command should be queued
 * and resent; the server's `client_id` idempotency makes a resend safe. Such failures have
 * `code: 'network'` (`status` 0 when there was no response at all).
 */
export class ApiError extends Error {
	readonly field?: string;
	readonly retryAfterSecs?: number;

	constructor(
		readonly status: number,
		readonly code: ApiErrorBody['error'] | 'network',
		message: string,
		readonly retryable = false,
		detail: ErrorDetail = {}
	) {
		super(message);
		this.name = 'ApiError';
		if (detail.field !== undefined) this.field = detail.field;
		if (detail.retryAfterSecs !== undefined) this.retryAfterSecs = detail.retryAfterSecs;
	}
}

// ---- the signed-out redirect -------------------------------------------------------------

/** Send the browser to sign-in, coming back here after. Not from sign-in itself. */
function goToSignIn(): void {
	const loc = globalThis.location;
	if (!loc || loc.pathname === '/login') return;
	// The fragment stays out: on /invite and /reset it is a secret code.
	loc.assign(loginPath(loc.pathname + loc.search));
}

let onUnauthorized: () => void = goToSignIn;

/** Send the browser to sign-in now, as a signed-in call's 401 does (a page that needs a user). */
export function redirectToSignIn(): void {
	onUnauthorized();
}

/** Replace what a signed-in call does on a 401 (tests); returns the undo. */
export function setUnauthorizedHandler(handler: () => void): () => void {
	const previous = onUnauthorized;
	onUnauthorized = handler;
	return () => {
		onUnauthorized = previous;
	};
}

/** What became of a command or replace request. A 409 or 422 is an answer, not a failure. */
export type CommandOutcome =
	| { status: 'applied'; view: MatchView }
	| { status: 'conflict'; view: MatchView }
	| { status: 'rejected'; kind: ErrorKind; index: number };

const enc = encodeURIComponent;

/** Proxy answers while the server is down or restarting. */
const GATEWAY = new Set([502, 503, 504]);

const unreachable = (status: number) =>
	new ApiError(status, 'network', strings.networkError, true);

/** How long a read or a command may take before it counts as lost (and a command is resent). */
export const REQUEST_TIMEOUT_MS = 10_000;

/**
 * A signal that aborts after `ms`. The same as `AbortSignal.timeout`, but built on the global
 * `setTimeout` so tests with fake timers can drive it; `done()` clears the timer.
 */
function deadline(ms: number): { signal: AbortSignal; done: () => void } {
	const ctrl = new AbortController();
	const timer = setTimeout(() => ctrl.abort(new DOMException('request timed out', 'TimeoutError')), ms);
	return { signal: ctrl.signal, done: () => clearTimeout(timer) };
}

interface Answer {
	status: number;
	body: unknown;
	/** The `Retry-After` header in seconds, when a 429 sends one. */
	retryAfter?: number;
}

async function call(path: string, init?: RequestInit, timeoutMs?: number): Promise<Answer> {
	const limit = timeoutMs === undefined ? null : deadline(timeoutMs);
	try {
		return await exchange(path, { ...init, signal: limit?.signal });
	} finally {
		limit?.done();
	}
}

function retryAfterHeader(res: Response): number | undefined {
	const secs = Number(res.headers.get('retry-after') ?? '');
	return Number.isInteger(secs) && secs >= 0 && res.headers.has('retry-after') ? secs : undefined;
}

/** One request; a timeout aborts it like a dropped connection, so it is retryable. */
async function exchange(path: string, init: RequestInit): Promise<Answer> {
	let res: Response;
	try {
		res = await fetch(path, {
			...init,
			headers: init.body ? { 'content-type': 'application/json' } : undefined
		});
	} catch {
		throw unreachable(0);
	}
	if (GATEWAY.has(res.status)) throw unreachable(res.status);
	let text: string;
	try {
		text = await res.text();
	} catch {
		// The connection dropped mid-body: whatever the status, the answer never arrived.
		throw unreachable(res.status);
	}
	// The account routes answer some requests with no content.
	if (res.status === 204) return { status: 204, body: null };
	try {
		const answer: Answer = { status: res.status, body: JSON.parse(text) };
		const retryAfter = retryAfterHeader(res);
		if (retryAfter !== undefined) answer.retryAfter = retryAfter;
		return answer;
	} catch {
		// The server only sends JSON, so an unreadable success was cut short or rewritten on the
		// way; an unreadable error page came from something in between.
		if (res.ok) throw unreachable(res.status);
		throw new ApiError(res.status, 'internal', strings.serverError(res.status));
	}
}

function failure(status: number, body: unknown, retryAfter?: number): ApiError {
	const b = body as Partial<{
		error: ApiErrorBody['error'];
		message: string;
		field: string;
		retry_after_secs: number;
	}> | null;
	const code = typeof b?.error === 'string' ? b.error : 'internal';
	const message = typeof b?.message === 'string' ? b.message : strings.serverError(status);
	const detail: ErrorDetail = {};
	if (typeof b?.field === 'string') detail.field = b.field;
	const wait = Number.isInteger(b?.retry_after_secs) ? b?.retry_after_secs : retryAfter;
	if (wait !== undefined) detail.retryAfterSecs = wait;
	return new ApiError(status, code, message, false, detail);
}

async function request<T>(path: string, ok: number, init?: RequestInit, timeoutMs?: number): Promise<T> {
	const { status, body, retryAfter } = await call(path, init, timeoutMs);
	if (status !== ok) throw failure(status, body, retryAfter);
	return body as T;
}

function post(body: unknown): RequestInit {
	return { method: 'POST', body: JSON.stringify(body) };
}

/** Account calls run argon2 on the server, so they get longer than a score read. */
export const ACCOUNT_TIMEOUT_MS = 20_000;

export interface AccountCall {
	method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
	body?: unknown;
	/** The success status, or the statuses that all mean success: 200 unless given. */
	ok?: number | readonly number[];
	/**
	 * The call needs the user signed in, so a 401 means the session ended (expired, revoked,
	 * disabled) and the browser goes to sign-in. A 401 `invalid_credentials` is a wrong password,
	 * an answer, and never redirects. Leave it off for the quiet boot check, the sign-in routes and
	 * the public code routes.
	 */
	signedIn?: boolean;
	/** The answer's shape; one that fails it is a server error. */
	guard?: (v: unknown) => boolean;
}

/** A request to an account, device or owned-match route (see ./auth). */
export async function accountRequest<T>(path: string, opts: AccountCall = {}): Promise<T> {
	const init: RequestInit = { method: opts.method ?? 'GET' };
	if (opts.body !== undefined) init.body = JSON.stringify(opts.body);
	const { status, body, retryAfter } = await call(path, init, ACCOUNT_TIMEOUT_MS);
	const ok = opts.ok ?? 200;
	if (typeof ok === 'number' ? status !== ok : !ok.includes(status)) {
		const err = failure(status, body, retryAfter);
		if (status === 401 && opts.signedIn && err.code !== 'invalid_credentials') onUnauthorized();
		throw err;
	}
	if (opts.guard && !opts.guard(body)) throw new ApiError(status, 'internal', strings.serverError(status));
	return body as T;
}

async function outcome(path: string, req: unknown): Promise<CommandOutcome> {
	let answer: { status: number; body: unknown };
	try {
		answer = await call(path, post(req), REQUEST_TIMEOUT_MS);
	} catch (e) {
		// A server error page on a command: whatever it was, a resend with the same client_id
		// is safe, so it is retryable like any other lost answer.
		if (e instanceof ApiError && e.status >= 500) throw unreachable(e.status);
		throw e;
	}
	const { status, body } = answer;
	if (status >= 500) throw unreachable(status);
	const b = body as ApiErrorBody;
	if (status === 200) return { status: 'applied', view: body as MatchView };
	if (status === 409 && b.error === 'conflict') return { status: 'conflict', view: b.view };
	if (status === 422 && b.error === 'rejected') {
		return { status: 'rejected', kind: b.kind, index: b.index };
	}
	throw failure(status, body);
}

export function createMatch(req: CreateMatchRequest): Promise<CreatedMatch> {
	return request('/api/matches', 201, post(req));
}

/** The scorer's view; the token must be a scoring token. */
export function getScore(token: string): Promise<MatchView> {
	return request(`/api/score/${enc(token)}`, 200, undefined, REQUEST_TIMEOUT_MS);
}

/** The read-only view; either token works. */
export function getView(token: string): Promise<MatchView> {
	return request(`/api/view/${enc(token)}`, 200, undefined, REQUEST_TIMEOUT_MS);
}

export function postCommands(token: string, req: CommandRequest): Promise<CommandOutcome> {
	return outcome(`/api/score/${enc(token)}/commands`, req);
}

/** "Keep mine" after a conflict. */
export function replace(token: string, req: ReplaceRequest): Promise<CommandOutcome> {
	return outcome(`/api/score/${enc(token)}/replace`, req);
}

/** The SSE stream of `state` events (each a MatchView) for a scoring or viewing token. */
export function streamUrl(kind: 'score' | 'view', token: string): string {
	return `/api/${kind}/${enc(token)}/stream`;
}
