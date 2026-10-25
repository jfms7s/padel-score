// The match API (crates/server/src/api.rs). Paths are relative: in production the same binary
// serves the app and the API, and in dev vite proxies /api to it.

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

/**
 * A request that failed. `retryable` means the server may never have seen or answered it (no
 * connection, a gateway error while it restarts, a dropped body), so a command should be queued
 * and resent; the server's `client_id` idempotency makes a resend safe. Such failures have
 * `code: 'network'` (`status` 0 when there was no response at all).
 */
export class ApiError extends Error {
	constructor(
		readonly status: number,
		readonly code: ApiErrorBody['error'] | 'network',
		message: string,
		readonly retryable = false
	) {
		super(message);
		this.name = 'ApiError';
	}
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

async function call(
	path: string,
	init?: RequestInit,
	timeoutMs?: number
): Promise<{ status: number; body: unknown }> {
	const limit = timeoutMs === undefined ? null : deadline(timeoutMs);
	try {
		return await exchange(path, { ...init, signal: limit?.signal });
	} finally {
		limit?.done();
	}
}

/** One request; a timeout aborts it like a dropped connection, so it is retryable. */
async function exchange(path: string, init: RequestInit): Promise<{ status: number; body: unknown }> {
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
	try {
		return { status: res.status, body: JSON.parse(text) };
	} catch {
		// The server only sends JSON, so an unreadable success was cut short or rewritten on the
		// way; an unreadable error page came from something in between.
		if (res.ok) throw unreachable(res.status);
		throw new ApiError(res.status, 'internal', strings.serverError(res.status));
	}
}

function failure(status: number, body: unknown): ApiError {
	const b = body as Partial<{ error: ApiErrorBody['error']; message: string }> | null;
	const code = typeof b?.error === 'string' ? b.error : 'internal';
	const message = typeof b?.message === 'string' ? b.message : strings.serverError(status);
	return new ApiError(status, code, message);
}

async function request<T>(path: string, ok: number, init?: RequestInit, timeoutMs?: number): Promise<T> {
	const { status, body } = await call(path, init, timeoutMs);
	if (status !== ok) throw failure(status, body);
	return body as T;
}

function post(body: unknown): RequestInit {
	return { method: 'POST', body: JSON.stringify(body) };
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
	return request(`/api/view/${enc(token)}`, 200);
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
