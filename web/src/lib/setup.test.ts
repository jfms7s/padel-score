import { describe, expect, it } from 'vitest';
import {
	LAST_SETUP_KEY,
	RECENT_KEY,
	addRecent,
	defaultSetup,
	listRecent,
	loadLastSetup,
	matchLabel,
	saveLastSetup,
	serveChoices,
	setSets,
	toCreateRequest,
	type Setup,
	type StorageLike
} from './setup';

function fakeStorage(initial: Record<string, string> = {}): StorageLike & { data: Record<string, string> } {
	const data = { ...initial };
	return {
		data,
		getItem: (k) => (k in data ? data[k] : null),
		setItem: (k, v) => {
			data[k] = v;
		}
	};
}

const throwing: StorageLike = {
	getItem: () => {
		throw new Error('denied');
	},
	setItem: () => {
		throw new Error('denied');
	}
};

const named = (over: Partial<Setup> = {}): Setup => ({
	...defaultSetup(),
	teams: { a: ['Ana', 'Rui'], b: ['João', 'Marta'] },
	...over
});

describe('setup', () => {
	it('default setup is golden point one set', () => {
		expect(defaultSetup()).toEqual({
			deuce: 'golden_point',
			sets: 'one',
			deciding: 'full_set',
			teams: { a: ['', ''], b: ['', ''] },
			firstServer: { team: 'a', index: 0 },
			otherFirstServer: 0
		});
	});

	it('one set forces full deciding set', () => {
		const s = named({ sets: 'one', deciding: 'super_tie_break' });
		expect(toCreateRequest(s).format).toEqual({
			sets: { deuce: 'golden_point', sets: 'one', deciding: 'full_set' }
		});
		expect(setSets(named({ sets: 'best_of_3', deciding: 'super_tie_break' }), 'one').deciding).toBe(
			'full_set'
		);
		expect(setSets(named({ deciding: 'super_tie_break' }), 'best_of_3').deciding).toBe(
			'super_tie_break'
		);
	});

	it('corrupt saved setup falls back to default', () => {
		for (const bad of ['{nope', 'null', '7', '{"deuce":"bogus"}', '{"deuce":"advantage"}']) {
			expect(loadLastSetup(fakeStorage({ [LAST_SETUP_KEY]: bad }))).toEqual(defaultSetup());
		}
		expect(loadLastSetup(fakeStorage())).toEqual(defaultSetup());
		expect(loadLastSetup(throwing)).toEqual(defaultSetup());
	});

	it('saves and restores the rules and names', () => {
		const st = fakeStorage();
		const s = named({ deuce: 'star_point', sets: 'best_of_3', deciding: 'super_tie_break' });
		saveLastSetup(st, s);
		expect(loadLastSetup(st)).toEqual(s);
		expect(() => saveLastSetup(throwing, s)).not.toThrow();
	});

	it('request carries both serve choices', () => {
		const s = named({
			deuce: 'advantage',
			sets: 'best_of_3',
			deciding: 'super_tie_break',
			firstServer: { team: 'b', index: 1 },
			otherFirstServer: 1
		});
		expect(toCreateRequest(s)).toEqual({
			format: { sets: { deuce: 'advantage', sets: 'best_of_3', deciding: 'super_tie_break' } },
			start: { first_server: { team: 'b', index: 1 }, other_first_server: 1 },
			teams: { a: ['Ana', 'Rui'], b: ['João', 'Marta'] }
		});
	});

	it('blank names become Player placeholders in the request', () => {
		const s = { ...defaultSetup(), teams: { a: [' Ana ', ''] as [string, string], b: ['', ''] as [string, string] } };
		expect(toCreateRequest(s).teams).toEqual({
			a: ['Ana', 'Player 2'],
			b: ['Player 3', 'Player 4']
		});
	});

	it('serve choices fall back to Player 1/2 placeholders and follow the first server', () => {
		const blank = defaultSetup();
		expect(serveChoices(blank).first.map((c) => c.label)).toEqual([
			'Player 1',
			'Player 2',
			'Player 3',
			'Player 4'
		]);
		expect(serveChoices(blank).other.map((c) => c.label)).toEqual(['Player 3', 'Player 4']);
		const s = named({ firstServer: { team: 'b', index: 0 } });
		expect(serveChoices(s).other.map((c) => c.label)).toEqual(['Ana', 'Rui']);
	});
});

describe('recent matches', () => {
	it('adds newest first, caps at 10, dedupes by token', () => {
		const st = fakeStorage();
		for (let i = 0; i < 12; i++) addRecent(st, { scorer_token: `t${i}`, label: `m${i}`, created_at: i });
		const list = listRecent(st);
		expect(list).toHaveLength(10);
		expect(list[0].scorer_token).toBe('t11');
		addRecent(st, { scorer_token: 't5', label: 'again', created_at: 99 });
		expect(listRecent(st)[0].label).toBe('again');
		expect(listRecent(st).filter((r) => r.scorer_token === 't5')).toHaveLength(1);
	});

	it('survives corrupt or unavailable storage', () => {
		expect(listRecent(fakeStorage({ [RECENT_KEY]: '{x' }))).toEqual([]);
		expect(listRecent(fakeStorage({ [RECENT_KEY]: '[{"bad":1},{"scorer_token":"a","label":"l","created_at":1}]' }))).toHaveLength(1);
		expect(listRecent(throwing)).toEqual([]);
		expect(() => addRecent(throwing, { scorer_token: 'a', label: 'l', created_at: 1 })).not.toThrow();
	});

	it('labels a match from the team names', () => {
		expect(matchLabel({ a: ['Ana', 'Rui'], b: ['João', 'Marta'] })).toBe('Ana / Rui v João / Marta');
	});
});
