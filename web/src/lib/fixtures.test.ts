// Every recorded server and engine sample must match the TS wire types exactly.

import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { isEvent, isFormat, isMatchView, isScore } from './guards';
import { asView } from './fixtures/load';

const all = import.meta.glob<{ default: unknown }>('./fixtures/*.json', { eager: true });
const fixture = (name: string) => all[`./fixtures/${name}.json`]!.default;
const views = Object.keys(all).filter((p) => /\/view-.*\.json$/.test(p));

describe('fixtures match the wire types', () => {
	it('has the recorded views', () => {
		expect(views.length).toBeGreaterThanOrEqual(19);
	});

	it.each(views)('%s is a MatchView', (path) => {
		expect(isMatchView(all[path]!.default)).toBe(true);
	});

	it('the conflict, replace and SSE bodies carry MatchViews', () => {
		expect(isMatchView((fixture('commands-409-conflict') as { view: unknown }).view)).toBe(true);
		expect(isMatchView(fixture('replace-200'))).toBe(true);
		const frame = readFileSync(new URL('./fixtures/stream-state.txt', import.meta.url), 'utf8');
		const data = frame.split('\n').find((l) => l.startsWith('data: '))!.slice(6);
		expect(isMatchView(JSON.parse(data))).toBe(true);
	});

	it('the engine sample is a Score', () => {
		expect(isScore((fixture('replay-ok-three-points') as { ok: unknown }).ok)).toBe(true);
	});

	it('asView refuses a sample that drifted', () => {
		const good = structuredClone(fixture('view-new')) as Record<string, unknown>;
		expect(() => asView(good)).not.toThrow();
		const renamed = { ...good, headSeq: good.head_seq };
		delete (renamed as Record<string, unknown>).head_seq;
		expect(() => asView(renamed)).toThrow();
	});
});

describe('guards', () => {
	it('accept every variant and reject unknown ones', () => {
		expect(isEvent({ point: 'a' })).toBe(true);
		expect(isEvent('time_up')).toBe(true);
		expect(isEvent({ point: 'c' })).toBe(false);
		expect(isEvent({ time_up: null })).toBe(false);
		expect(isFormat({ timed: { deuce: 'star_point', draws_allowed: true } })).toBe(true);
		expect(isFormat({ timed: { deuce: 'star_point' } })).toBe(false);
		expect(isFormat({ sets: { deuce: 'advantage', sets: 'best_of_5', deciding: 'full_set' } })).toBe(false);
	});

	it('reject an extra or mistyped field', () => {
		const score = structuredClone((fixture('view-tiebreak') as { score: Record<string, unknown> }).score);
		expect(isScore(score)).toBe(true);
		expect(isScore({ ...score, extra: 1 })).toBe(false);
		expect(isScore({ ...score, games: [6, '6'] })).toBe(false);
		expect(isScore({ ...score, status: { finished: {} } })).toBe(false);
	});
});
