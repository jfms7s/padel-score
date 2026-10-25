import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BACKOFF_MS, backoffDelay, openStream, sseParser, WATCHDOG_MS, type StreamStatus } from './stream';
import type { MatchView } from './types';
import { asView } from './fixtures/load';
import onePoint from './fixtures/view-one-point.json';

const view = asView(onePoint);
const frame = (v: MatchView) => `event: state\ndata: ${JSON.stringify(v)}\n\n`;
const KEEP_ALIVE = ':\n\n';

interface Conn {
	url: string;
	aborted: boolean;
	push(text: string): void;
	end(): void;
	drop(): void;
}

/** A fake fetch whose connections the test drives: each call opens one streaming response. */
function fakeServer() {
	const conns: Conn[] = [];
	const enc = new TextEncoder();
	const server = {
		conns,
		/** Fail the next connects as a network error would. */
		refuse: false,
		status: 200,
		fetch: vi.fn(async (url: string, init?: RequestInit): Promise<Response> => {
			if (server.refuse) throw new TypeError('Failed to fetch');
			let ctrl!: ReadableStreamDefaultController<Uint8Array>;
			const body = new ReadableStream<Uint8Array>({
				start(c) {
					ctrl = c;
				}
			});
			const conn: Conn = {
				url,
				aborted: false,
				push: (text) => ctrl.enqueue(enc.encode(text)),
				end: () => ctrl.close(),
				drop: () => ctrl.error(new TypeError('network error'))
			};
			init?.signal?.addEventListener('abort', () => {
				conn.aborted = true;
				try {
					ctrl.error(new DOMException('aborted', 'AbortError'));
				} catch {
					// already closed
				}
			});
			conns.push(conn);
			return new Response(body, { status: server.status });
		})
	};
	return server;
}

function open(server: ReturnType<typeof fakeServer>) {
	const states: MatchView[] = [];
	const statuses: StreamStatus[] = [];
	const stream = openStream(
		'/api/score/t/stream',
		(v) => states.push(v),
		(s) => statuses.push(s),
		{
			fetch: server.fetch as unknown as typeof fetch,
			setTimeout: (f, ms) => setTimeout(f, ms),
			clearTimeout: (id) => clearTimeout(id)
		}
	);
	return { states, statuses, stream };
}

const tick = (ms = 0) => vi.advanceTimersByTimeAsync(ms);

beforeEach(() => {
	vi.useFakeTimers();
});
afterEach(() => {
	vi.useRealTimers();
});

describe('sseParser', () => {
	it('parses frames split anywhere, CRLF, multi-line data and comments', () => {
		const events: [string, string][] = [];
		let comments = 0;
		const p = sseParser(
			(type, data) => events.push([type, data]),
			() => comments++
		);
		p.feed('event: st');
		p.feed('ate\r\ndata: {"a":\r\ndata:1}\r\n\r\n: keep-alive\n\n');
		p.feed('data: plain\n\nevent: other\ndata: x\n');
		p.feed('\n');
		// A frame with no data dispatches nothing.
		p.feed('event: state\n\n');
		expect(events).toEqual([
			['state', '{"a":\n1}'],
			['message', 'plain'],
			['other', 'x']
		]);
		expect(comments).toBe(1);
	});
});

describe('openStream', () => {
	it('delivers state frames and reports live', async () => {
		const server = fakeServer();
		const { states, statuses } = open(server);
		await tick();
		expect(server.fetch).toHaveBeenCalledTimes(1);
		expect(server.conns[0]?.url).toBe('/api/score/t/stream');
		server.conns[0]?.push(frame(view));
		await tick();
		expect(states).toEqual([view]);
		expect(statuses).toEqual(['live']);
	});

	it('stream_drop_marks_offline', async () => {
		const server = fakeServer();
		const { statuses } = open(server);
		await tick();
		server.conns[0]?.push(frame(view));
		await tick(WATCHDOG_MS - 1);
		expect(statuses).toEqual(['live']);
		await tick(1);
		expect(statuses).toEqual(['live', 'offline']);
		expect(server.conns[0]?.aborted).toBe(true);
	});

	it('treats keep-alive comments as liveness', async () => {
		const server = fakeServer();
		const { statuses } = open(server);
		await tick();
		server.conns[0]?.push(frame(view));
		for (let i = 0; i < 6; i++) {
			await tick(15_000);
			server.conns[0]?.push(KEEP_ALIVE);
		}
		await tick(15_000);
		expect(statuses).toEqual(['live']);
		expect(server.fetch).toHaveBeenCalledTimes(1);
	});

	it('marks offline when the connection errors or the server ends it', async () => {
		const server = fakeServer();
		const { statuses } = open(server);
		await tick();
		server.conns[0]?.push(frame(view));
		await tick();
		server.conns[0]?.drop();
		await tick();
		expect(statuses).toEqual(['live', 'offline']);
		await tick(BACKOFF_MS[0]);
		server.conns[1]?.push(frame(view));
		await tick();
		server.conns[1]?.end();
		await tick();
		expect(statuses).toEqual(['live', 'offline', 'live', 'offline']);
	});

	it('stream reconnects with backoff', async () => {
		const server = fakeServer();
		server.refuse = true;
		const { statuses } = open(server);
		await tick();
		expect(server.fetch).toHaveBeenCalledTimes(1);
		expect(statuses).toEqual(['offline']);
		const expected = [1000, 2000, 5000, 10_000, 10_000, 10_000];
		for (const [i, ms] of expected.entries()) {
			await tick(ms - 1);
			expect(server.fetch).toHaveBeenCalledTimes(i + 1);
			await tick(1);
			expect(server.fetch).toHaveBeenCalledTimes(i + 2);
		}
		// Offline is reported once, not on every failed attempt.
		expect(statuses).toEqual(['offline']);
	});

	it('reconnects after a watchdog timeout and an error status', async () => {
		const server = fakeServer();
		server.status = 502;
		const { statuses } = open(server);
		await tick();
		expect(statuses).toEqual(['offline']);
		server.status = 200;
		await tick(1000);
		expect(server.fetch).toHaveBeenCalledTimes(2);
		// Connected but silent: the watchdog gives up after 20 s and backs off 2 s.
		await tick(WATCHDOG_MS);
		expect(server.conns[1]?.aborted).toBe(true);
		expect(statuses).toEqual(['offline']);
		await tick(2000);
		expect(server.fetch).toHaveBeenCalledTimes(3);
	});

	it('frame after reconnect marks live', async () => {
		const server = fakeServer();
		server.refuse = true;
		const { statuses, states } = open(server);
		await tick();
		await tick(1000);
		await tick(2000);
		server.refuse = false;
		await tick(5000);
		expect(server.fetch).toHaveBeenCalledTimes(4);
		expect(statuses).toEqual(['offline']);
		server.conns[0]?.push(KEEP_ALIVE);
		await tick();
		expect(statuses).toEqual(['offline', 'live']);
		server.conns[0]?.push(frame(view));
		await tick();
		expect(states).toHaveLength(1);
		// Up for less than 30 s: a flapping link keeps backing off (the next step is 10 s).
		server.conns[0]?.drop();
		await tick(9999);
		expect(server.fetch).toHaveBeenCalledTimes(4);
		await tick(1);
		expect(server.fetch).toHaveBeenCalledTimes(5);
	});

	it('resets the backoff once the stream has stayed up 30 s', async () => {
		const server = fakeServer();
		server.refuse = true;
		open(server);
		await tick();
		await tick(1000);
		await tick(2000);
		server.refuse = false;
		await tick(5000);
		const conn = server.conns[0];
		conn?.push(frame(view));
		for (let i = 0; i < 2; i++) {
			await tick(15_000);
			conn?.push(KEEP_ALIVE);
		}
		conn?.drop();
		await tick(999);
		expect(server.fetch).toHaveBeenCalledTimes(4);
		await tick(1);
		expect(server.fetch).toHaveBeenCalledTimes(5);
	});

	it('reports gone and stops on a 404', async () => {
		const server = fakeServer();
		server.status = 404;
		const { statuses } = open(server);
		await tick();
		expect(statuses).toEqual(['gone']);
		await tick(60_000);
		expect(server.fetch).toHaveBeenCalledTimes(1);
	});

	it('ignores frames that are not state or not JSON', async () => {
		const server = fakeServer();
		const { states, statuses } = open(server);
		await tick();
		server.conns[0]?.push('event: state\ndata: {oops\n\nevent: hello\ndata: {}\n\n');
		await tick();
		expect(states).toEqual([]);
		expect(statuses).toEqual(['live']);
	});

	it('close stops reading and reconnecting', async () => {
		const server = fakeServer();
		const { stream, statuses } = open(server);
		await tick();
		stream.close();
		await tick();
		expect(server.conns[0]?.aborted).toBe(true);
		await tick(60_000);
		expect(server.fetch).toHaveBeenCalledTimes(1);
		expect(statuses).toEqual([]);
	});
});

describe('backoffDelay', () => {
	it('steps 1, 2, 5, 10 s and stays at 10', () => {
		expect([0, 1, 2, 3, 4, 50].map(backoffDelay)).toEqual([1000, 2000, 5000, 10_000, 10_000, 10_000]);
	});
});
