// The live score stream: the server's SSE `state` events (each a MatchView) for one match.
//
// Why fetch and not EventSource: the server sends a keep-alive comment (`:`) every 10 s, and
// that comment is the only sign of life while nobody scores. EventSource swallows comments, so
// a silent but healthy stream and a dead one (a phone that slept, a proxy holding a dead TCP
// connection) look the same to it. Reading the body with fetch and parsing the frames here sees
// every line, so a watchdog can declare the stream offline after 20 s of silence, drop the
// connection and reconnect with backoff.

import { isMatchView } from './guards';
import type { MatchView } from './types';

/** `gone`: the server says the match doesn't exist (purged or never was); the stream stops. */
export type StreamStatus = 'live' | 'offline' | 'gone';

/** No frame (state or keep-alive) for this long means the stream is gone. */
export const WATCHDOG_MS = 20_000;
/** A connection must stay up this long before the backoff starts over, so a flapping link
 *  keeps backing off instead of reconnecting every second. */
export const STABLE_MS = 30_000;
/** Reconnect delays, the last one repeating. Also used to resend commands. */
export const BACKOFF_MS = [1000, 2000, 5000, 10_000] as const;

export function backoffDelay(attempt: number): number {
	return BACKOFF_MS[Math.min(attempt, BACKOFF_MS.length - 1)] ?? 10_000;
}

type Timer = ReturnType<typeof setTimeout>;

export interface StreamDeps {
	fetch: typeof fetch;
	setTimeout: (f: () => void, ms: number) => Timer;
	clearTimeout: (t: Timer) => void;
	now?: () => number;
}

const reconnectListeners = new Set<() => void>();

/**
 * Hear any stream on the page come back: `'live'` after `'offline'` (not its first `'live'`).
 * The non-stop's server clock re-syncs on it, as a phone that slept may have drifted. Returns
 * the unsubscribe.
 */
export function onStreamReconnect(listener: () => void): () => void {
	reconnectListeners.add(listener);
	return () => reconnectListeners.delete(listener);
}

const defaultDeps = (): StreamDeps => ({
	fetch: (...args) => globalThis.fetch(...args),
	setTimeout: (f, ms) => globalThis.setTimeout(f, ms),
	clearTimeout: (t) => globalThis.clearTimeout(t)
});

/**
 * An SSE parser (the WHATWG event-stream rules, minus `id` and `retry`, which the server
 * never sends): feed it decoded text in any chunks. `onEvent` gets each dispatched event's
 * type (default `message`) and data; `onComment` gets each comment line.
 */
export function sseParser(onEvent: (type: string, data: string) => void, onComment: () => void) {
	let buffer = '';
	let type = '';
	let data: string[] = [];

	function line(l: string) {
		if (l === '') {
			if (data.length > 0) onEvent(type || 'message', data.join('\n'));
			type = '';
			data = [];
			return;
		}
		if (l.startsWith(':')) {
			onComment();
			return;
		}
		const colon = l.indexOf(':');
		const field = colon < 0 ? l : l.slice(0, colon);
		let value = colon < 0 ? '' : l.slice(colon + 1);
		if (value.startsWith(' ')) value = value.slice(1);
		if (field === 'event') type = value;
		else if (field === 'data') data.push(value);
	}

	return {
		feed(text: string) {
			buffer += text;
			// A lone CR at the end may be half of a CRLF: wait for the next chunk.
			const re = /\r\n|\r(?!$)|\n/g;
			let start = 0;
			for (let m = re.exec(buffer); m; m = re.exec(buffer)) {
				line(buffer.slice(start, m.index));
				start = m.index + m[0].length;
			}
			buffer = buffer.slice(start);
		}
	};
}

/**
 * Follow `url`'s `state` events until `close()`. `onStatus` hears `'offline'` on a failed
 * connect, an error, the end of the stream or 20 s of silence, and `'live'` on the first frame
 * after that (or the first frame at all). Each is reported once per change. Reconnects back off
 * 1, 2, 5, then every 10 s; the backoff starts over only after a connection stayed up 30 s.
 * A 404 reports `'gone'` and stops for good.
 */
export function openStream(
	url: string,
	onState: (view: MatchView) => void,
	onStatus: (status: StreamStatus) => void,
	deps: StreamDeps = defaultDeps()
): { close(): void } {
	return openSseStream(url, { type: 'state', accept: isMatchView }, onState, onStatus, deps);
}

/** Which frames a stream follows: their SSE event type, and the check each payload must pass. */
export interface FrameKind<T> {
	type: string;
	accept: (v: unknown) => v is T;
}

/**
 * `openStream` for any frame type: follow `url`'s `kind.type` events, handing each payload that
 * passes `kind.accept` to `onFrame`. Status, watchdog and reconnects are as `openStream`'s.
 */
export function openSseStream<T>(
	url: string,
	kind: FrameKind<T>,
	onFrame: (frame: T) => void,
	onStatus: (status: StreamStatus) => void,
	deps: StreamDeps = defaultDeps()
): { close(): void } {
	let closed = false;
	let status: StreamStatus | null = null;
	let attempt = 0;
	let watchdog: Timer | null = null;
	let retry: Timer | null = null;
	let abort: AbortController | null = null;
	/** When the current connection delivered its first frame. */
	let upSince: number | null = null;
	const now = deps.now ?? (() => Date.now());

	const report = (s: StreamStatus) => {
		if (closed || status === s) return;
		const back = s === 'live' && status === 'offline';
		status = s;
		onStatus(s);
		if (back) for (const listener of reconnectListeners) listener();
	};

	const clearWatchdog = () => {
		if (watchdog !== null) deps.clearTimeout(watchdog);
		watchdog = null;
	};

	const armWatchdog = (ctrl: AbortController) => {
		clearWatchdog();
		watchdog = deps.setTimeout(() => {
			watchdog = null;
			ctrl.abort();
		}, WATCHDOG_MS);
	};

	function alive(ctrl: AbortController) {
		if (ctrl !== abort) return;
		upSince ??= now();
		armWatchdog(ctrl);
		report('live');
	}

	function lost(ctrl: AbortController) {
		if (closed || ctrl !== abort) return;
		clearWatchdog();
		ctrl.abort();
		abort = null;
		if (upSince !== null && now() - upSince >= STABLE_MS) attempt = 0;
		upSince = null;
		report('offline');
		const delay = backoffDelay(attempt);
		attempt++;
		retry = deps.setTimeout(() => {
			retry = null;
			void connect();
		}, delay);
	}

	function gone(ctrl: AbortController) {
		if (closed || ctrl !== abort) return;
		clearWatchdog();
		report('gone');
		closed = true;
		ctrl.abort();
		abort = null;
	}

	async function connect() {
		if (closed) return;
		const ctrl = new AbortController();
		abort = ctrl;
		armWatchdog(ctrl);
		const parser = sseParser(
			(type, data) => {
				alive(ctrl);
				if (type !== kind.type) return;
				let payload: unknown;
				try {
					payload = JSON.parse(data);
				} catch {
					console.error(`stream: dropped an unreadable ${kind.type} frame`);
					return;
				}
				if (!kind.accept(payload)) {
					console.error(`stream: dropped a ${kind.type} frame of the wrong shape`);
					return;
				}
				if (!closed) onFrame(payload);
			},
			() => alive(ctrl)
		);
		try {
			const res = await deps.fetch(url, {
				headers: { accept: 'text/event-stream' },
				cache: 'no-store',
				signal: ctrl.signal
			});
			if (res.status === 404) {
				void res.body?.cancel().catch(() => {});
				gone(ctrl);
				return;
			}
			if (!res.ok || !res.body) {
				void res.body?.cancel().catch(() => {});
				throw new Error(`stream answered ${res.status}`);
			}
			const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
			for (;;) {
				const { done, value } = await reader.read();
				if (done || ctrl !== abort) break;
				parser.feed(value);
			}
		} catch {
			// Aborted, refused or dropped: all the same to the fence.
		}
		lost(ctrl);
	}

	void connect();

	return {
		close() {
			closed = true;
			clearWatchdog();
			if (retry !== null) deps.clearTimeout(retry);
			retry = null;
			abort?.abort();
			abort = null;
		}
	};
}
