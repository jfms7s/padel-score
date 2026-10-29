import { describe, expect, it } from 'vitest';
import { choices, legacyNewTarget } from './create';

describe('choices', () => {
	it('lists the four ways to start, in order', () => {
		const list = choices();
		expect(list.map((c) => c.key)).toEqual(['match', 'later', 'board', 'event']);
		expect(list.map((c) => c.href)).toEqual(['/new/match', '/new/later', '/new/board', '/new/event']);
	});
	it('words each choice from the strings', () => {
		const [match] = choices();
		expect(match?.title).toBe('Score a match now');
		expect(match?.line).toBe('On the court, four players, start tapping');
	});
});

describe('legacyNewTarget', () => {
	it('sends a link made from a game to the score page, query kept', () => {
		expect(legacyNewTarget(new URLSearchParams('from=game%3Ag1'))).toBe('/new/match?from=game%3Ag1');
	});
	it('leaves a plain /new alone', () => {
		expect(legacyNewTarget(new URLSearchParams(''))).toBeNull();
		expect(legacyNewTarget(new URLSearchParams('x=1'))).toBeNull();
	});
});
