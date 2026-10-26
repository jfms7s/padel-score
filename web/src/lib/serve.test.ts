import { describe, expect, it } from 'vitest';
import { defaultServeOrder, fromStart, toStart, type ServeOrder } from './serve';
import type { Start } from './types';

const ORDERS: ServeOrder[] = (['a', 'b'] as const).flatMap((team) =>
	([0, 1] as const).flatMap((a) => ([0, 1] as const).map((b): ServeOrder => ({ team, first: { a, b } })))
);

describe('serve order', () => {
	it('defaults to team A serving, the second player of each team first', () => {
		expect(defaultServeOrder()).toEqual({ team: 'a', first: { a: 1, b: 1 } });
		expect(toStart(defaultServeOrder())).toEqual({ first_server: { team: 'a', index: 1 }, other_first_server: 1 });
	});

	it('maps to the start the rotation reads', () => {
		expect(toStart({ team: 'b', first: { a: 0, b: 1 } })).toEqual({
			first_server: { team: 'b', index: 1 },
			other_first_server: 0
		});
	});

	it('round-trips all 8 orders', () => {
		expect(ORDERS).toHaveLength(8);
		for (const order of ORDERS) expect(fromStart(toStart(order))).toEqual(order);
	});

	it('round-trips every valid start', () => {
		for (const team of ['a', 'b'] as const)
			for (const index of [0, 1])
				for (const other of [0, 1]) {
					const start: Start = { first_server: { team, index }, other_first_server: other };
					expect(toStart(fromStart(start)!)).toEqual(start);
				}
	});

	it('rejects an invalid start', () => {
		expect(fromStart({ first_server: { team: 'a', index: 2 }, other_first_server: 0 })).toBeNull();
		expect(fromStart({ first_server: { team: 'a', index: 0 }, other_first_server: 2 })).toBeNull();
		expect(fromStart({ first_server: { team: 'c' as 'a', index: 0 }, other_first_server: 0 })).toBeNull();
	});
});
