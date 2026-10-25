import { describe, expect, it } from 'vitest';
import { isEvent, isFormat, isMatchView, isScore } from './guards';
import { loadView } from './lastview';
import { MemoryStorage } from './fixtures/queue-harness';
import onePoint from './fixtures/view-one-point.json';

const withExtras = () => {
	const v = structuredClone(onePoint) as Record<string, unknown> & { score: Record<string, unknown>; events: Record<string, unknown>[] };
	v.created_at = '2026-10-25T00:00:00Z';
	v.score.extra = 1;
	v.events[0]!.at = 3;
	return v;
};

describe('runtime guards', () => {
	it('runtime_guard_accepts_unknown_extra_keys', () => {
		const v = withExtras();
		expect(isMatchView(v)).toBe(true);
		expect(isScore(v.score)).toBe(true);
		expect(isFormat({ timed: { deuce: 'advantage', draws_allowed: true, new_field: 'x' } })).toBe(true);
	});

	it('strict mode refuses unknown keys, as the fixture tests need', () => {
		expect(isMatchView(onePoint, { strict: true })).toBe(true);
		expect(isMatchView(withExtras(), { strict: true })).toBe(false);
		expect(isFormat({ timed: { deuce: 'advantage', draws_allowed: true, new_field: 'x' } }, { strict: true })).toBe(false);
	});

	it('lenient mode still needs every required key with its type', () => {
		const { revision: _r, ...missing } = onePoint;
		expect(isMatchView(missing)).toBe(false);
		expect(isMatchView({ ...onePoint, revision: 'one' })).toBe(false);
		// A variant it doesn't know is still not one it can score.
		expect(isEvent({ point: 'a', lost: true })).toBe(false);
		expect(isEvent({ ace: 'a' })).toBe(false);
	});

	it('a stored view with an extra key still loads', () => {
		const storage = new MemoryStorage();
		storage.setItem('padel-score:view:tok', JSON.stringify(withExtras()));
		expect(loadView(storage, 'tok')?.revision).toBe(onePoint.revision);
	});
});
