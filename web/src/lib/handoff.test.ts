import { afterEach, describe, expect, it, vi } from 'vitest';
import { defaultSetup } from './setup';
import { parseFrom, putStartHandoff, takeStartHandoff, withStartPlayers } from './handoff';
import type { StartPlayers } from './types';

const started: StartPlayers = {
	players: [
		{ player_id: 'p1', name: 'Ana' },
		{ player_id: 'p2', name: 'Rui' },
		{ player_id: 'p3', name: 'Marta' },
		{ player_id: 'p4', name: 'Joana' }
	],
	format: null,
	from: { kind: 'game', id: 'g1' }
};

function fakeSession() {
	const data = new Map<string, string>();
	vi.stubGlobal('sessionStorage', {
		getItem: (k: string) => data.get(k) ?? null,
		setItem: (k: string, v: string) => void data.set(k, v),
		removeItem: (k: string) => void data.delete(k)
	});
	return data;
}

afterEach(() => vi.unstubAllGlobals());

describe('start hand-off', () => {
	it('a take returns the value once', () => {
		const data = fakeSession();
		putStartHandoff(started);
		expect(data.has('padel-score:start:game:g1')).toBe(true);
		expect(takeStartHandoff('game:g1')).toEqual(started);
		expect(takeStartHandoff('game:g1')).toBeNull();
	});

	it('is keyed by the source', () => {
		fakeSession();
		putStartHandoff(started);
		expect(takeStartHandoff('game:g2')).toBeNull();
		expect(takeStartHandoff('scheduled:g1')).toBeNull();
	});

	it('ignores a value that is not a hand-off', () => {
		const data = fakeSession();
		data.set('padel-score:start:game:g1', '{"players":[]}');
		expect(takeStartHandoff('game:g1')).toBeNull();
		data.set('padel-score:start:game:g1', 'nope');
		expect(takeStartHandoff('game:g1')).toBeNull();
	});

	it('survives storage that throws', () => {
		vi.stubGlobal('sessionStorage', {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('full');
			},
			removeItem: () => {}
		});
		expect(() => putStartHandoff(started)).not.toThrow();
		expect(takeStartHandoff('game:g1')).toBeNull();
	});

	it('fills seats 0-1 into team A and 2-3 into team B', () => {
		const s = withStartPlayers(defaultSetup(), started);
		expect(s.teams).toEqual({ a: ['Ana', 'Rui'], b: ['Marta', 'Joana'] });
		expect(s.players).toEqual({ a: ['p1', 'p2'], b: ['p3', 'p4'] });
		expect(s.deuce).toBe(defaultSetup().deuce);
	});

	it('reads the from query value', () => {
		expect(parseFrom('game:g1')).toEqual({ kind: 'game', id: 'g1' });
		expect(parseFrom('nope')).toBeNull();
		expect(parseFrom(null)).toBeNull();
	});
});
