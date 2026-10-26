import { describe, expect, it } from 'vitest';
import { courtTile, deskTabs, groupMatches, isJumpKey, moveRow, nextUnfinished, resolveTab } from './desk';
import type { EventMatch, EventTeam, FinalScore, Score } from './types';

function team(id: string, name: string): EventTeam {
	return { id, name, custom_name: null, players: [{ name: 'x' }, { name: 'y' }] } as unknown as EventTeam;
}

const teams = [team('t1', 'Ana / Marta'), team('t2', 'Rui / Joao')];

function match(id: string, extra: Partial<EventMatch> = {}): EventMatch {
	return {
		id,
		label: `Match ${id}`,
		ordinal: 1,
		team_a: 't1',
		team_b: 't2',
		court_id: null,
		state: 'waiting',
		locked: false,
		winner: null,
		result: null,
		live: null,
		scorer_token: '',
		spectator_token: '',
		...extra
	};
}

function liveScore(games: [number, number], current: Score['current'], sets_won: [number, number] = [0, 0]): Score {
	return {
		sets: [{ a: games[0], b: games[1], tiebreak: null }],
		sets_won,
		games,
		regulation_games: games,
		current,
		server: { team: 'a', index: 0 },
		side: 'right',
		change_ends: false,
		overtime: false,
		time_up: false,
		status: 'in_progress'
	};
}

const final: FinalScore = {
	sets: [
		{ a: 6, b: 4, tiebreak: null },
		{ a: 3, b: 6, tiebreak: null },
		{ a: 6, b: 2, tiebreak: null }
	],
	games: [6, 2],
	regulation_games: [6, 2],
	overtime: false,
	timed: false
};

describe('jump key', () => {
	const field = { closest: (sel: string) => (sel.includes('input') ? {} : null) };
	const base = {
		key: '/',
		ctrlKey: false,
		metaKey: false,
		altKey: false,
		defaultPrevented: false,
		target: {} as EventTarget
	};
	it('takes a bare slash outside fields', () => {
		expect(isJumpKey({ ...base, target: { closest: () => null } as unknown as EventTarget })).toBe(true);
	});
	it('ignores fields, modifiers, other keys and handled presses', () => {
		expect(isJumpKey({ ...base, target: field as unknown as EventTarget })).toBe(false);
		expect(isJumpKey({ ...base, ctrlKey: true })).toBe(false);
		expect(isJumpKey({ ...base, key: 'a' })).toBe(false);
		expect(isJumpKey({ ...base, defaultPrevented: true })).toBe(false);
	});
});

describe('tabs', () => {
	it('lists the tabs per status', () => {
		expect(deskTabs('setup')).toEqual(['teams', 'courts', 'kind']);
		expect(deskTabs('running')).toEqual(['live', 'matches', 'kind', 'teams', 'courts', 'finish']);
		expect(deskTabs('finished')).toEqual(['live', 'matches', 'kind', 'teams']);
	});

	it('resolves the tab param', () => {
		expect(resolveTab('kind', 'running')).toBe('kind');
		expect(resolveTab('finish', 'setup')).toBe('teams');
		expect(resolveTab('nonsense', 'running')).toBe('live');
		expect(resolveTab(null, 'finished')).toBe('live');
	});
});

describe('groupMatches', () => {
	it('groups and orders by ordinal', () => {
		const ms = [
			match('w2', { ordinal: 5 }),
			match('f', { ordinal: 1, state: 'finished', court_id: 'c1' }),
			match('q', { ordinal: 4 }),
			match('l', { ordinal: 3, state: 'live', court_id: 'c2' }),
			match('w1', { ordinal: 2 })
		];
		const g = groupMatches(ms, [
			{ id: 'c1', current: 'f' },
			{ id: 'c2', current: null },
			{ id: 'c3', current: 'q' }
		]);
		expect(g.onCourt.map((m) => m.id)).toEqual(['l', 'q']);
		expect(g.finished.map((m) => m.id)).toEqual(['f']);
		expect(g.waiting.map((m) => m.id)).toEqual(['w1', 'w2']);
	});
});

describe('keyboard rows', () => {
	it('nextUnfinished', () => {
		const ids = ['a', 'b', 'c'];
		expect(nextUnfinished(ids, new Set(['a', 'c']), 'a', true)).toBe('c');
		expect(nextUnfinished(ids, new Set(['a', 'c']), 'a', false)).toBe('a');
		expect(nextUnfinished(['a'], new Set(), 'a', true)).toBeNull();
		expect(nextUnfinished(ids, new Set(['a']), 'c', true)).toBe('a');
		expect(nextUnfinished(ids, new Set(['b']), null, true)).toBe('b');
	});

	it('moveRow clamps', () => {
		expect(moveRow(['a', 'b'], 'b', 1)).toBe('b');
		expect(moveRow(['a', 'b'], 'b', -1)).toBe('a');
		expect(moveRow(['a', 'b'], 'a', -1)).toBe('a');
		expect(moveRow(['a', 'b'], null, -1)).toBe('b');
		expect(moveRow(['a', 'b'], null, 1)).toBe('a');
		expect(moveRow([], null, 1)).toBeNull();
	});
});

describe('courtTile', () => {
	it('shows a live game', () => {
		const m = match('m1', {
			state: 'live',
			live: liveScore([4, 3], { game: { a: 'forty', b: 'thirty', deuces: 0, deciding_point: false } }, [1, 0])
		});
		const t = courtTile({ current: 'm1', queue: [] }, [m], teams);
		expect(t).toMatchObject({
			state: 'live',
			server: 'a',
			a: { point: '40', games: 4, sets: 1, names: 'Ana / Marta' },
			b: { point: '30', games: 3, sets: 0 }
		});
	});

	it('shows tie-break points and timed games', () => {
		const m = match('m1', {
			state: 'live',
			live: liveScore([6, 6], { tie_break: { a: 3, b: 5, super_tiebreak: false } })
		});
		const t = courtTile({ current: 'm1', queue: [] }, [m], teams);
		expect(t).toMatchObject({ state: 'live', a: { point: '3' }, b: { point: '5' } });
	});

	it('has no point when current is none, and null sets when timed', () => {
		const s = liveScore([2, 1], 'none');
		const m = match('m1', { state: 'live', live: s });
		const t = courtTile({ current: 'm1', queue: [] }, [m], teams);
		expect(t).toMatchObject({ a: { point: '' }, b: { point: '' } });
		const timed = courtTile(
			{ current: 'm1', queue: [] },
			[{ ...m, live: { ...s, sets_won: [0, 0], sets: [] } }],
			teams
		);
		expect(timed).toMatchObject({ a: { sets: null } });
	});

	it('shows a finished match with no queue', () => {
		const m = match('m1', { state: 'finished', winner: 'a', result: final });
		const t = courtTile({ current: 'm1', queue: [] }, [m], teams);
		expect(t).toMatchObject({
			state: 'finished',
			winner: 'a',
			nextLabel: null,
			score: '6-4 3-6 6-2',
			a: { games: 6, sets: 2, setGames: [6, 3, 6] },
			b: { games: 2, sets: 1, setGames: [4, 6, 2] }
		});
	});

	it('shows every set of a finished match, a super tie-break by its points', () => {
		const result: FinalScore = {
			sets: [
				{ a: 6, b: 4, tiebreak: null },
				{ a: 6, b: 7, tiebreak: [5, 7] },
				{ a: 1, b: 0, tiebreak: [10, 8] }
			],
			games: [1, 0],
			regulation_games: [1, 0],
			overtime: false,
			timed: false
		};
		const t = courtTile({ current: 'm1', queue: [] }, [match('m1', { state: 'finished', winner: 'a', result })], teams);
		expect(t).toMatchObject({
			score: '6-4 6-7(5) 10-8',
			a: { sets: 2, setGames: [6, 6, 10] },
			b: { sets: 1, setGames: [4, 7, 8] }
		});
	});

	it('shows a finished timed match by its games', () => {
		const result: FinalScore = { sets: [], games: [9, 7], regulation_games: [9, 7], overtime: false, timed: true };
		const t = courtTile({ current: 'm1', queue: [] }, [match('m1', { state: 'finished', winner: 'a', result })], teams);
		expect(t).toMatchObject({
			score: '9-7',
			a: { games: 9, sets: null, setGames: [] },
			b: { games: 7, sets: null, setGames: [] }
		});
	});

	it('is ready with a waiting match on court', () => {
		const t = courtTile({ current: 'm1', queue: ['m2'] }, [match('m1'), match('m2')], teams);
		expect(t).toEqual({ state: 'ready', label: 'Match m1', a: 'Ana / Marta', b: 'Rui / Joao', nextLabel: 'Match m2' });
	});

	it('is ready with a live match that has no score yet', () => {
		const t = courtTile({ current: 'm1', queue: [] }, [match('m1', { state: 'live', live: null })], teams);
		expect(t).toEqual({ state: 'ready', label: 'Match m1', a: 'Ana / Marta', b: 'Rui / Joao', nextLabel: null });
	});

	it('is idle with a queued match', () => {
		const t = courtTile({ current: null, queue: ['m2'] }, [match('m2')], teams);
		expect(t).toEqual({ state: 'idle', nextLabel: 'Match m2' });
	});
});
