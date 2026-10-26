import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { beforeAll, describe, expect, it } from 'vitest';
import { eventsOf, initEngineSync, replayLocal, validateFinal } from './engine';
import type { Event, Format, Start } from './types';
import { asView } from './fixtures/load';
import tiebreak from './fixtures/view-tiebreak.json';
import timedFinished from './fixtures/view-timed-finished.json';
import replayInvalidRules from './fixtures/replay-err-invalid-rules.json';

const SETS: Format = { sets: { deuce: 'golden_point', sets: 'one', deciding: 'full_set' } };
const START: Start = { first_server: { team: 'a', index: 0 }, other_first_server: 0 };

beforeAll(() => {
	const wasm = fileURLToPath(new URL('./wasm/padel_scoring_wasm_bg.wasm', import.meta.url));
	initEngineSync(readFileSync(wasm));
});

describe('replayLocal', () => {
	it('replayLocal matches a known score', () => {
		const events: Event[] = [{ point: 'a' }, { point: 'a' }, { point: 'a' }];
		const out = replayLocal(SETS, START, events);
		if (!('ok' in out)) throw new Error(JSON.stringify(out));
		expect(out.ok.current).toEqual({
			game: { a: 'forty', b: 'love', deuces: 0, deciding_point: false }
		});
	});

	it('agrees with the server on recorded matches', () => {
		for (const json of [tiebreak, timedFinished]) {
			const view = asView(json);
			expect(replayLocal(view.format, view.start, eventsOf(view))).toEqual({ ok: view.score });
		}
	});

	it('reports the engine error and its index', () => {
		expect(replayLocal(SETS, START, [{ point: 'a' }, 'time_up'])).toEqual({
			err: { kind: 'not_timed', index: 1 }
		});
	});

	it('reports invalid rules as the engine serializes them', () => {
		const bad: Format = { sets: { deuce: 'advantage', sets: 'one', deciding: 'super_tie_break' } };
		expect(replayLocal(bad, START, [])).toEqual(replayInvalidRules);
	});
});

describe('validateFinal', () => {
	it('judges a typed score as the server does', () => {
		expect(validateFinal(SETS, { sets: [{ a: 6, b: 4, tiebreak: null }] })).toEqual({ winner: 'a' });
		expect(validateFinal(SETS, { sets: [{ a: 6, b: 5, tiebreak: null }] })).toEqual({
			error: { impossible_set: { index: 0 } }
		});
		expect(validateFinal(SETS, { sets: [] })).toEqual({ error: 'match_not_finished' });
		const timed: Format = { timed: { deuce: 'advantage', draws_allowed: false } };
		expect(validateFinal(timed, { timed: { a: 3, b: 3, overtime_winner: null } })).toEqual({
			error: 'draw_not_allowed'
		});
		expect(validateFinal(timed, { timed: { a: 3, b: 3, overtime_winner: 'b' } })).toEqual({ winner: 'b' });
		expect(validateFinal(timed, { sets: [] })).toEqual({ error: 'wrong_format' });
	});
});
