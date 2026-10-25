// The live score stream: the server's SSE `state` events (each a MatchView) for one match.
//
// Why fetch and not EventSource: the server sends a keep-alive comment (`:`) every 15 s, and
// that comment is the only sign of life while nobody scores. EventSource swallows comments, so
// a silent but healthy stream and a dead one (a phone that slept, a proxy holding a dead TCP
// connection) look the same to it. Reading the body with fetch and parsing the frames here sees
// every line, so a watchdog can declare the stream offline after 20 s of silence, drop the
// connection and reconnect with backoff.

import type { MatchView } from './types';

export type StreamStatus = 'live' | 'offline';

/** No frame (state or keep-alive) for this long means the stream is gone. */
export const WATCHDOG_MS = 20_000;
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
 * 1, 2, 5, then every 10 s; any frame resets the backoff.
 */
export function openStream(
	url: string,
	onState: (view: MatchView) => void,
	onStatus: (status: StreamStatus) => void,
	deps: StreamDeps = defaultDeps()
): { close(): void } {
	let closed = false;
	let status: StreamStatus | null = null;
	let attempt = 0;
	let watchdog: Timer | null = null;
	let retry: Timer | null = null;
	let abort: AbortController | null = null;

	const report = (s: StreamStatus) => {
		if (closed || status === s) return;
		status = s;
		onStatus(s);
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
		attempt = 0;
		armWatchdog(ctrl);
		report('live');
	}

	function lost(ctrl: AbortController) {
		if (closed || ctrl !== abort) return;
		clearWatchdog();
		ctrl.abort();
		abort = null;
		report('offline');
		const delay = backoffDelay(attempt);
		attempt++;
		retry = deps.setTimeout(() => {
			retry = null;
			void connect();
		}, delay);
	}

	async function connect() {
		if (closed) return;
		const ctrl = new AbortController();
		abort = ctrl;
		armWatchdog(ctrl);
		const parser = sseParser(
			(type, data) => {
				alive(ctrl);
				if (type !== 'state') return;
				let view: MatchView;
				try {
					view = JSON.parse(data) as MatchView;
				} catch {
					return;
				}
				if (!closed) onState(view);
			},
			() => alive(ctrl)
		);
		try {
			const res = await deps.fetch(url, {
				headers: { accept: 'text/event-stream' },
				cache: 'no-store',
				signal: ctrl.signal
			});
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
