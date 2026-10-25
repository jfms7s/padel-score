import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { beforeAll, describe, expect, it } from 'vitest';
import {
	ballPosition,
	changeEndsLine,
	LAYOUT_KEY,
	latestView,
	loadLayout,
	nextLayout,
	optimisticView,
	saveLayout,
	setsSummary,
	shareLinks,
	spectatorFromHash
} from './fence';
import { initEngineSync, replayLocal } from './engine';
import { fenceModel } from './view';
import { asView as v } from './fixtures/load';
import type { MatchView } from './types';
import type { StorageLike } from './setup';
import newMatch from './fixtures/view-new.json';
import onePoint from './fixtures/view-one-point.json';
import goldenPoint from './fixtures/view-golden-point-40-40.json';
import changeEnds from './fixtures/view-change-ends.json';
import thirdGame from './fixtures/view-third-game.json';
import finished from './fixtures/view-finished.json';
import tiebreak from './fixtures/view-tiebreak.json';
import timed from './fixtures/view-timed.json';
import superTiebreakFinished from './fixtures/view-super-tiebreak-finished.json';

beforeAll(() => {
	const wasm = fileURLToPath(new URL('./wasm/padel_scoring_wasm_bg.wasm', import.meta.url));
	initEngineSync(readFileSync(wasm));
});

const withRevision = (view: MatchView, revision: number): MatchView => ({ ...view, revision });

function fakeStorage(init: Record<string, string> = {}): StorageLike & { data: Record<string, string> } {
	const data = { ...init };
	return { data, getItem: (k) => data[k] ?? null, setItem: (k, val) => void (data[k] = val) };
}

describe('latestView', () => {
	const base = v(onePoint);

	it('takes the first view', () => {
		expect(latestView(null, base)).toBe(base);
	});

	it('takes a newer revision and ignores an older one', () => {
		const newer = withRevision(base, base.revision + 1);
		expect(latestView(base, newer)).toBe(newer);
		expect(latestView(newer, base)).toBe(newer);
	});

	it('keeps the view in hand for the same revision', () => {
		const same = withRevision(base, base.revision);
		expect(latestView(base, same)).toBe(base);
	});
});

describe('ballPosition', () => {
	it('puts the ball on the server side of the serving team', () => {
		expect(ballPosition(fenceModel(v(newMatch)))).toEqual({ team: 'a', side: 'right' });
		expect(ballPosition(fenceModel(v(onePoint)))).toEqual({ team: 'a', side: 'left' });
		expect(ballPosition(fenceModel(v(changeEnds)))).toEqual({ team: 'b', side: 'right' });
	});

	it('centres the ball when the receiver chooses', () => {
		expect(ballPosition(fenceModel(v(goldenPoint)))).toEqual({ team: 'a', side: 'centre' });
	});

	it('has no ball once the match is over', () => {
		expect(ballPosition(fenceModel(v(finished)))).toBeNull();
	});
});

describe('changeEndsLine', () => {
	it('says who leads in the set', () => {
		const m = fenceModel(v(changeEnds));
		expect(changeEndsLine(m)).toBe('Change ends. Ana / Rui lead 1–0 in set 1.');
	});

	it('says who took the set when a new one starts', () => {
		const m = fenceModel(v(changeEnds));
		m.a.sets = [3, 0];
		m.b.sets = [6, 0];
		m.a.games = 0;
		m.b.games = 0;
		expect(changeEndsLine(m)).toBe('Change ends. João / Marta took set 1 6–3.');
	});

	it('uses the tie-break points in a tie-break', () => {
		const m = fenceModel(v(tiebreak));
		expect(changeEndsLine(m)).toBe('Change ends. Ana / Rui lead 2–1 in the tie-break.');
		m.b.points = '2';
		expect(changeEndsLine(m)).toBe('Change ends. 2–2 in the tie-break.');
	});

	it('has no set number in a timed match', () => {
		const m = fenceModel(v(timed));
		m.b.games = 2;
		expect(changeEndsLine(m)).toBe('Change ends. João / Marta lead 2–1.');
	});
});

describe('setsSummary', () => {
	it('lists the set scores with tie-break and super tie-break points', () => {
		expect(setsSummary(fenceModel(v(finished)))).toBe('6–0');
		expect(setsSummary(fenceModel(v(superTiebreakFinished)))).toBe('6–0, 0–6, 10–0');
		const tb = v(finished);
		const won: MatchView = {
			...tb,
			score: { ...tb.score, sets: [{ a: 6, b: 7, tiebreak: [5, 7] }, { a: 6, b: 2, tiebreak: null }] }
		};
		expect(setsSummary(fenceModel(won))).toBe('6–7 (5), 6–2');
	});

	it('is empty for a timed match', () => {
		expect(setsSummary(fenceModel(v(timed)))).toBe('');
	});
});

describe('layout', () => {
	it('defaults to auto and remembers a choice', () => {
		const st = fakeStorage();
		expect(loadLayout(st)).toBe('auto');
		saveLayout(st, 'board');
		expect(st.data[LAYOUT_KEY]).toBe('board');
		expect(loadLayout(st)).toBe('board');
		expect(loadLayout(fakeStorage({ [LAYOUT_KEY]: 'sideways' }))).toBe('auto');
	});

	it('cycles auto, court, board', () => {
		expect(nextLayout('auto')).toBe('court');
		expect(nextLayout('court')).toBe('board');
		expect(nextLayout('board')).toBe('auto');
	});

	it('survives a store that throws', () => {
		const bad: StorageLike = {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('full');
			}
		};
		expect(loadLayout(bad)).toBe('auto');
		expect(() => saveLayout(bad, 'court')).not.toThrow();
	});
});

describe('share links', () => {
	it('reads the spectator token from the fragment', () => {
		expect(spectatorFromHash('#v=abc_-9')).toBe('abc_-9');
		expect(spectatorFromHash('')).toBeNull();
		expect(spectatorFromHash('#x=1')).toBeNull();
		expect(spectatorFromHash('#v=')).toBeNull();
	});

	it('builds absolute scoring and viewing links', () => {
		expect(shareLinks('https://p.example', 'sc/1', 'sp')).toEqual({
			scoring: 'https://p.example/s/sc%2F1#v=sp',
			viewing: 'https://p.example/v/sp'
		});
		expect(shareLinks('https://p.example', 'sc', null)).toEqual({
			scoring: 'https://p.example/s/sc',
			viewing: null
		});
	});
});

describe('optimisticView', () => {
	it('adds pending points to the server events', () => {
		const out = optimisticView(v(newMatch), ['a', 'a', 'b'], replayLocal);
		expect(out?.score.current).toEqual({
			game: { a: 'thirty', b: 'fifteen', deuces: 0, deciding_point: false }
		});
		expect(out?.revision).toBe(v(newMatch).revision);
	});

	it('gives up when the engine refuses the points', () => {
		expect(optimisticView(v(finished), ['a'], replayLocal)).toBeNull();
	});

	it('matches the server for the same points', () => {
		const third = v(thirdGame);
		const before: MatchView = { ...third, events: third.events.slice(0, -1) };
		const point = third.events.at(-1)?.event;
		if (!point || point === 'time_up') throw new Error('fixture');
		expect(optimisticView(before, [point.point], replayLocal)?.score).toEqual(third.score);
	});
});
