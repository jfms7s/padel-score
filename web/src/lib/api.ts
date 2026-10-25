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
	FinalRequest,
	InvalidScore,
	MatchView,
	ReplaceRequest,
	Start
} from './types';

/** What some refusals add: the field a 422 names, the wait a 429 asks for, the engine's reason. */
export interface ErrorDetail {
	field?: string;
	retryAfterSecs?: number;
	/** A 422 `invalid_score`: why the engine refused a typed score. */
	reason?: InvalidScore;
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
	readonly reason?: InvalidScore;

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
		if (detail.reason !== undefined) this.reason = detail.reason;
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

/**
 * What became of a command or replace request. A 409 or 422 is an answer, not a failure.
 * `locked`: the coordinator typed the result, so a scoring link can't change the match.
 */
export type CommandOutcome =
	| { status: 'applied'; view: MatchView }
	| { status: 'conflict'; view: MatchView }
	| { status: 'locked'; view: MatchView }
	| { status: 'rejected'; kind: ErrorKind; index: number };

/** What became of a typed final score: as a command, or the engine's reason it can't be. */
export type FinalOutcome =
	| { status: 'applied'; view: MatchView }
	| { status: 'conflict'; view: MatchView }
	| { status: 'locked'; view: MatchView }
	| { status: 'invalid_score'; reason: InvalidScore };

/** What became of choosing who serves first. */
export type StartOutcome = { status: 'applied'; view: MatchView } | { status: 'locked'; view: MatchView };

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
		reason: InvalidScore;
	}> | null;
	const code = typeof b?.error === 'string' ? b.error : 'internal';
	const message = typeof b?.message === 'string' ? b.message : strings.serverError(status);
	const detail: ErrorDetail = {};
	if (typeof b?.field === 'string') detail.field = b.field;
	const wait = Number.isInteger(b?.retry_after_secs) ? b?.retry_after_secs : retryAfter;
	if (wait !== undefined) detail.retryAfterSecs = wait;
	if (b?.error === 'invalid_score' && b.reason !== undefined) detail.reason = b.reason;
	return new ApiError(status, code, message, false, detail);
}

async function request<T>(path: string, ok: number, init?: RequestInit, timeoutMs?: number): Promise<T> {
	const { status, body, retryAfter } = await call(path, init, timeoutMs);
	if (status !== ok) throw failure(status, body, retryAfter);
	return body as T;
}

/**
 * A request to a route that needs no account (a court or viewing link): any status but `ok`
 * throws an ApiError, and a 401 never sends the browser to sign-in.
 */
export function plainRequest<T>(path: string, ok = 200, init?: RequestInit): Promise<T> {
	return request(path, ok, init, REQUEST_TIMEOUT_MS);
}

function post(body: unknown): RequestInit {
	return { method: 'POST', body: JSON.stringify(body) };
}

/** Account calls run argon2 on the server, so they get longer than a score read. */
export const ACCOUNT_TIMEOUT_MS = 20_000;

export interface AccountCall {
	method?: 'GET' | 'POST' | 'PATCH' | 'DELETE';
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

/** A command-like send: 200 and 409 conflict or locked are answers; a 5xx is retryable. */
async function answerTo(path: string, init: RequestInit): Promise<{ status: number; body: unknown }> {
	let answer: { status: number; body: unknown };
	try {
		answer = await call(path, init, REQUEST_TIMEOUT_MS);
	} catch (e) {
		// A server error page on a command: whatever it was, a resend with the same client_id
		// is safe, so it is retryable like any other lost answer.
		if (e instanceof ApiError && e.status >= 500) throw unreachable(e.status);
		throw e;
	}
	if (answer.status >= 500) throw unreachable(answer.status);
	return answer;
}

/** The view a 200, a 409 conflict or a 409 locked carries; null for any other answer. */
function viewOutcome(status: number, body: unknown): Exclude<CommandOutcome, { status: 'rejected' }> | null {
	const b = body as ApiErrorBody;
	if (status === 200) return { status: 'applied', view: body as MatchView };
	if (status === 409 && b.error === 'conflict') return { status: 'conflict', view: b.view };
	if (status === 409 && b.error === 'locked') return { status: 'locked', view: b.view };
	return null;
}

async function outcome(path: string, req: unknown): Promise<CommandOutcome> {
	const { status, body } = await answerTo(path, post(req));
	const known = viewOutcome(status, body);
	if (known) return known;
	const b = body as ApiErrorBody;
	if (status === 422 && b.error === 'rejected') {
		return { status: 'rejected', kind: b.kind, index: b.index };
	}
	throw failure(status, body);
}

/** A typed final score's answer, from a scoring link or the coordinator. */
export function finalOutcome(status: number, body: unknown): FinalOutcome | null {
	const known = viewOutcome(status, body);
	if (known) return known;
	const b = body as ApiErrorBody;
	if (status === 422 && b.error === 'invalid_score') return { status: 'invalid_score', reason: b.reason };
	return null;
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

/**
 * Type the final score from a scoring link: the server's events become exactly that score.
 * `base_revision` is the view the score was typed on; a log that moved since is a conflict.
 */
export async function postFinal(token: string, req: FinalRequest): Promise<FinalOutcome> {
	const { status, body } = await answerTo(`/api/score/${enc(token)}/final`, post(req));
	const out = finalOutcome(status, body);
	if (out) return out;
	throw failure(status, body);
}

/**
 * Choose who serves first, before the first point. A match already started throws an
 * ApiError `already_started`.
 */
export async function putStart(token: string, start: Start): Promise<StartOutcome> {
	const { status, body } = await answerTo(`/api/score/${enc(token)}/start`, {
		method: 'PUT',
		body: JSON.stringify({ start })
	});
	const out = viewOutcome(status, body);
	if (out && out.status !== 'conflict') return out;
	throw failure(status, body);
}

/** A court link's API path: `/api/court/{token}`, then `rest` (say `/advance`). */
export function courtUrl(token: string, rest = ''): string {
	return `/api/court/${enc(token)}${rest}`;
}

/** The SSE stream of `state` events (each a MatchView) for a scoring or viewing token. */
export function streamUrl(kind: 'score' | 'view', token: string): string {
	return `/api/${kind}/${enc(token)}/stream`;
}
