// Who serves first, as the pickers ask it: which team, then each team's first server. The
// engine's `Start` is the same choice from the rotation's side (first server, then the
// receiving team's first server), and the two map onto each other without loss.

import type { Start, Team } from './types';

export interface ServeOrder {
	/** The team that serves first. */
	team: Team;
	/** Each team's first server (the index of the player), whichever team serves first. */
	first: { a: 0 | 1; b: 0 | 1 };
}

/** Team A serves first; both teams' second player serves first for the team. */
export function defaultServeOrder(): ServeOrder {
	return { team: 'a', first: { a: 1, b: 1 } };
}

export function toStart(order: ServeOrder): Start {
	const other: Team = order.team === 'a' ? 'b' : 'a';
	return {
		first_server: { team: order.team, index: order.first[order.team] },
		other_first_server: order.first[other]
	};
}

/** The order for a stored `Start`; null if it isn't a valid one. */
export function fromStart(start: Start): ServeOrder | null {
	const { first_server: server, other_first_server: other } = start;
	if (server.team !== 'a' && server.team !== 'b') return null;
	if ((server.index !== 0 && server.index !== 1) || (other !== 0 && other !== 1)) return null;
	return server.team === 'a'
		? { team: 'a', first: { a: server.index, b: other } }
		: { team: 'b', first: { a: other, b: server.index } };
}
