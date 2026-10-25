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

/** A request that failed: `status` 0 means the server could not be reached. */
export class ApiError extends Error {
	constructor(
		readonly status: number,
		readonly code: ApiErrorBody['error'] | 'network',
		message: string
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

async function call(path: string, init?: RequestInit): Promise<{ status: number; body: unknown }> {
	let res: Response;
	try {
		res = await fetch(path, {
			...init,
			headers: init?.body ? { 'content-type': 'application/json' } : undefined
		});
	} catch {
		throw new ApiError(0, 'network', strings.networkError);
	}
	let body: unknown;
	try {
		body = await res.json();
	} catch {
		throw new ApiError(res.status, 'internal', strings.serverError(res.status));
	}
	return { status: res.status, body };
}

function failure(status: number, body: unknown): ApiError {
	const b = body as Partial<{ error: ApiErrorBody['error']; message: string }> | null;
	const code = typeof b?.error === 'string' ? b.error : 'internal';
	const message = typeof b?.message === 'string' ? b.message : strings.serverError(status);
	return new ApiError(status, code, message);
}

async function request<T>(path: string, ok: number, init?: RequestInit): Promise<T> {
	const { status, body } = await call(path, init);
	if (status !== ok) throw failure(status, body);
	return body as T;
}

function post(body: unknown): RequestInit {
	return { method: 'POST', body: JSON.stringify(body) };
}

async function outcome(path: string, req: unknown): Promise<CommandOutcome> {
	const { status, body } = await call(path, post(req));
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
	return request(`/api/score/${enc(token)}`, 200);
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
