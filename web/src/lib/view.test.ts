import { describe, expect, it } from 'vitest';
import { fenceModel, pageMode, pointLabel } from './view';
import { asView as v } from './fixtures/load';
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
import overtime from './fixtures/view-timed-overtime.json';
import overtimeFinished from './fixtures/view-timed-overtime-finished.json';
import superTiebreakFinished from './fixtures/view-super-tiebreak-finished.json';

// The fixtures are real responses from the server (serde's actual output); `v` checks each
// against the wire types before use.

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

	it('names the serving player only when a name was entered', () => {
		const unnamed = v(newMatch);
		unnamed.teams = { a: ['Player 1', 'Player 2'], b: ['Player 3', 'Player 4'] };
		const m = fenceModel(unnamed);
		expect(m.a.serving).toBe(true);
		expect(m.servingPlayerName).toBeNull();

		// A placeholder in another slot is a name like any other; only the slot's own is left out.
		const server = unnamed.score.server;
		const mixed = v(newMatch);
		mixed.teams = { a: ['Player 3', 'Player 4'], b: ['Player 1', 'Player 2'] };
		expect(fenceModel(mixed).servingPlayerName).toBe(mixed.teams[server.team][server.index]);

		const partly = v(newMatch);
		partly.teams = { a: ['Ana', 'Player 2'], b: ['Player 3', 'Player 4'] };
		partly.teams[server.team][server.index] = 'Rita';
		expect(fenceModel(partly).servingPlayerName).toBe('Rita');
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
		expect(m.serveSide).toBeNull();
		expect(m.servingPlayerName).toBeNull();
		expect(m.a.points).toBe('');
	});

	it('reports a super tie-break win', () => {
		const m = fenceModel(v(superTiebreakFinished));
		expect(m.finished).toEqual({ winner: 'a' });
		expect(m.inTieBreak).toBe(false);
		expect(m.superTieBreak).toBe(false);
		expect(m.a.sets).toEqual([6, 0, 1]);
		expect(m.b.sets).toEqual([0, 6, 0]);
		expect(m.a.setsWon).toBe(2);
		expect(m.serveSide).toBeNull();
		expect(m.servingPlayerName).toBeNull();
		expect(m.a.serving || m.b.serving).toBe(false);
		expect(m.rulesLine).toBe('Match over');
	});

	it('shows overtime in a timed match, then its winner', () => {
		const m = fenceModel(v(overtime));
		expect(m.rulesLine).toBe('Overtime, golden point');
		expect(m.a.games).toBe(1);
		expect(m.b.games).toBe(1);
		expect(m.a.points).toBe('30');
		expect(m.serveSide).not.toBeNull();
		expect(m.servingPlayerName).not.toBeNull();

		const done = fenceModel(v(overtimeFinished));
		// The overtime game counts in the games shown.
		expect(done.a.games).toBe(2);
		expect(done.b.games).toBe(1);
		expect(done.finished).toEqual({ winner: 'a' });
		expect(done.serveSide).toBeNull();
		expect(done.servingPlayerName).toBeNull();
		expect(done.rulesLine).toBe('Match over');
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

	it('details finished sets, with tie-break points for the loser', () => {
		const base = v(finished);
		const view = {
			...base,
			score: {
				...base.score,
				sets: [
					{ a: 7, b: 6, tiebreak: [7, 5] as [number, number] },
					{ a: 4, b: 6, tiebreak: null },
					{ a: 6, b: 7, tiebreak: [9, 11] as [number, number] }
				]
			}
		};
		const m = fenceModel(view);
		expect(m.a.setDetails).toEqual([
			{ games: 7, tiebreak: null, superTieBreak: false, won: true },
			{ games: 4, tiebreak: null, superTieBreak: false, won: false },
			{ games: 6, tiebreak: 9, superTieBreak: false, won: false }
		]);
		expect(m.b.setDetails).toEqual([
			{ games: 6, tiebreak: 5, superTieBreak: false, won: false },
			{ games: 6, tiebreak: null, superTieBreak: false, won: true },
			{ games: 7, tiebreak: null, superTieBreak: false, won: true }
		]);
	});

	it('details a super tie-break set by its points', () => {
		const m = fenceModel(v(superTiebreakFinished));
		expect(m.a.setDetails.at(-1)).toEqual({ games: 1, tiebreak: 10, superTieBreak: true, won: true });
		expect(m.b.setDetails.at(-1)).toEqual({ games: 0, tiebreak: 0, superTieBreak: true, won: false });
		expect(m.a.setDetails[0]).toEqual({ games: 6, tiebreak: null, superTieBreak: false, won: true });
	});

	it('leaves the set in progress undecided, even in its tie-break', () => {
		const m = fenceModel(v(tiebreak));
		expect(m.a.setDetails).toEqual([{ games: 6, tiebreak: null, superTieBreak: false, won: null }]);
		const sup = fenceModel(v(superTiebreak));
		expect(sup.a.setDetails.at(-1)).toEqual({ games: 0, tiebreak: null, superTieBreak: true, won: null });
		expect(sup.b.setDetails[1]).toEqual({ games: 6, tiebreak: null, superTieBreak: false, won: true });
		expect(fenceModel(v(timed)).a.setDetails).toEqual([]);
	});

	it('only the deciding set of a super tie-break format is a super tie-break', () => {
		const one = v(finished);
		const m = fenceModel({
			...one,
			format: { sets: { deuce: 'golden_point', sets: 'one', deciding: 'super_tie_break' } }
		});
		expect(m.a.setDetails[0]?.superTieBreak).toBe(false);
		const sup = fenceModel(v(superTiebreakFinished));
		expect(sup.a.setDetails.map((d) => d.superTieBreak)).toEqual([false, false, true]);
	});
});

describe('pageMode', () => {
	it('readonly fence model hides controls on the spectator route', () => {
		expect(pageMode('/v/abc123')).toEqual({ controls: false });
		expect(pageMode('/v/abc123/')).toEqual({ controls: false });
	});
	it('gives the scorer route its controls', () => {
		expect(pageMode('/s/abc123')).toEqual({ controls: true });
		expect(pageMode('/s/abc123/')).toEqual({ controls: true });
	});
	it('defaults to read-only for anything that is not the scorer route', () => {
		expect(pageMode('/')).toEqual({ controls: false });
		expect(pageMode('/v/s/x')).toEqual({ controls: false });
		expect(pageMode('/new')).toEqual({ controls: false });
	});
});
