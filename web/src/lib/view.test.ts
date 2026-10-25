import { describe, expect, it } from 'vitest';
import { fenceModel, pointLabel } from './view';
import type { MatchView } from './types';
import { strings } from './strings';
import goldenPoint from './fixtures/view-golden-point-40-40.json';
import starPoint from './fixtures/view-star-point.json';
import advantage from './fixtures/view-advantage.json';
import newMatch from './fixtures/view-new.json';
import onePoint from './fixtures/view-one-point.json';
import changeEnds from './fixtures/view-change-ends.json';
import thirdGame from './fixtures/view-third-game.json';
import finished from './fixtures/view-finished.json';
import tiebreak from './fixtures/view-tiebreak.json';
import superTiebreak from './fixtures/view-super-tiebreak.json';
import timed from './fixtures/view-timed.json';
import timedFinished from './fixtures/view-timed-finished.json';
import timedDraw from './fixtures/view-timed-draw.json';

// The fixtures are real responses from the server (and so serde's actual output); JSON imports
// widen string unions to string, hence the cast.
const v = (json: unknown) => json as MatchView;

describe('pointLabel', () => {
	it('pointLabel maps calls', () => {
		expect(pointLabel('love')).toBe('0');
		expect(pointLabel('fifteen')).toBe('15');
		expect(pointLabel('thirty')).toBe('30');
		expect(pointLabel('forty')).toBe('40');
		expect(pointLabel('advantage')).toBe('AD');
	});
});

describe('fenceModel', () => {
	it('fenceModel shows golden point callout at 40-40', () => {
		const m = fenceModel(v(goldenPoint));
		expect(m.a.points).toBe('40');
		expect(m.b.points).toBe('40');
		expect(m.callout).toBe(strings.goldenPoint);
		expect(m.callout).toBe('Golden point');
		expect(m.serveSide).toBe('receiver_chooses');
	});

	it('shows the star point callout only at the deciding deuce', () => {
		expect(fenceModel(v(starPoint)).callout).toBe('Star point');
		const adv = fenceModel(v(advantage));
		expect(adv.callout).toBeNull();
		expect(adv.a.points).toBe('AD');
		expect(adv.b.points).toBe('40');
	});

	it('has no callout before 40-40', () => {
		expect(fenceModel(v(onePoint)).callout).toBeNull();
	});

	it('fenceModel marks server and side', () => {
		const start = fenceModel(v(newMatch));
		expect(start.a.serving).toBe(true);
		expect(start.b.serving).toBe(false);
		expect(start.serveSide).toBe('right');
		expect(start.servingPlayerName).toBe('Ana');

		const after = fenceModel(v(onePoint));
		expect(after.a.points).toBe('15');
		expect(after.b.points).toBe('0');
		expect(after.serveSide).toBe('left');

		// After the first game the receiving team's chosen first server serves...
		const next = fenceModel(v(changeEnds));
		expect(next.a.serving).toBe(false);
		expect(next.b.serving).toBe(true);
		expect(next.servingPlayerName).toBe('João');
		expect(next.serveSide).toBe('right');

		// ...and after the second, the first server's partner (one point into the third game).
		const third = fenceModel(v(thirdGame));
		expect(third.a.serving).toBe(true);
		expect(third.servingPlayerName).toBe('Rui');
		expect(third.serveSide).toBe('left');
	});

	it('names teams and counts games and sets', () => {
		const m = fenceModel(v(thirdGame));
		expect(m.a.name).toBe('Ana / Rui');
		expect(m.b.name).toBe('João / Marta');
		expect(m.a.games).toBe(1);
		expect(m.b.games).toBe(1);
		expect(m.a.sets).toEqual([1]);
		expect(m.b.sets).toEqual([1]);
		expect(m.rulesLine).toBe('Set 1, golden point');
		expect(m.finished).toBeNull();
	});

	it('flags a change of ends right after an odd game', () => {
		expect(fenceModel(v(changeEnds)).changeEnds).toBe(true);
		expect(fenceModel(v(thirdGame)).changeEnds).toBe(false);
	});

	it('fenceModel reports winner when finished', () => {
		const m = fenceModel(v(finished));
		expect(m.finished).toEqual({ winner: 'a' });
		expect(m.a.serving).toBe(false);
		expect(m.b.serving).toBe(false);
		expect(m.a.sets).toEqual([6]);
		expect(m.b.sets).toEqual([0]);
		expect(m.a.setsWon).toBe(1);
		expect(m.callout).toBeNull();
		expect(m.changeEnds).toBe(false);
		expect(m.rulesLine).toBe(strings.matchOver);
	});

	it('reports a timed draw with no winner', () => {
		expect(fenceModel(v(timedDraw)).finished).toEqual({ winner: null });
		expect(fenceModel(v(timedFinished)).finished).toEqual({ winner: 'a' });
	});

	it('fenceModel uses tie-break points during a tie-break', () => {
		const m = fenceModel(v(tiebreak));
		expect(m.inTieBreak).toBe(true);
		expect(m.superTieBreak).toBe(false);
		expect(m.a.points).toBe('2');
		expect(m.b.points).toBe('1');
		expect(m.a.games).toBe(6);
		expect(m.b.games).toBe(6);
		expect(m.callout).toBeNull();
		expect(m.rulesLine).toBe('Set 1, tie-break');
	});

	it('marks a super tie-break', () => {
		const m = fenceModel(v(superTiebreak));
		expect(m.inTieBreak).toBe(true);
		expect(m.superTieBreak).toBe(true);
		expect(m.a.points).toBe('2');
		expect(m.b.points).toBe('1');
		expect(m.a.sets).toEqual([6, 0, 0]);
		expect(m.b.sets).toEqual([0, 6, 0]);
		expect(m.a.setsWon).toBe(1);
		expect(m.b.setsWon).toBe(1);
		expect(m.rulesLine).toBe('Super tie-break');
	});

	it('shows a timed match with no sets', () => {
		const m = fenceModel(v(timed));
		expect(m.a.sets).toEqual([]);
		expect(m.a.games).toBe(1);
		expect(m.b.points).toBe('15');
		expect(m.rulesLine).toBe('Non-stop, golden point');
	});
});
