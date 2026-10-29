// Row data for the Results page: matches I played (`/api/me/results`) and matches I recorded
// (`/api/me/history`) reduced to the same shape, so one row component renders both.
import { longDate } from '#lib/dates.ts';
import { scoreText } from '#lib/players.ts';
import { strings } from '#lib/strings.ts';
import type { HistoryEntry, LinkedEntry, Outcome, Team } from '#lib/types.ts';

export type ResultsView = 'played' | 'recorded';

/** `recorded` only for `?view=recorded`; anything else is `played`. */
export function viewFromQuery(search: URLSearchParams): ResultsView {
	return search.get('view') === 'recorded' ? 'recorded' : 'played';
}

export interface ResultRowData {
	id: string;
	date: string;
	a: string;
	b: string;
	winner: Team | null;
	score: string;
	chip: 'won' | 'lost' | 'draw' | 'recorded';
	event: string | null;
	href: string | null;
}

const CHIPS: Record<Outcome, ResultRowData['chip']> = { win: 'won', loss: 'lost', draw: 'draw' };

const names = (team: readonly string[]) => team.join(strings.teamNameSeparator);
const finalView = (token: string) => `/v/${encodeURIComponent(token)}`;
const other = (side: Team): Team => (side === 'a' ? 'b' : 'a');

/** The winning side from the player's: their own side on a win, the other on a loss. */
function winnerOf(e: LinkedEntry): Team | null {
	if (e.outcome === 'draw') return null;
	return e.outcome === 'win' ? e.side : other(e.side);
}

/** A match the player played. Another coordinator's match is redacted: no names, no link. */
export function playedRow(e: LinkedEntry, timeZone?: string): ResultRowData {
	const base = {
		id: e.match_id,
		date: longDate(e.finished_at, timeZone),
		winner: winnerOf(e),
		score: scoreText(e.score),
		chip: CHIPS[e.outcome]
	};
	if (e.redacted) {
		return {
			...base,
			a: strings.results.hidden,
			b: strings.results.hidden,
			event: e.event ? strings.events.kinds[e.event.kind] : null,
			href: null
		};
	}
	return {
		...base,
		a: names(e.teams.a),
		b: names(e.teams.b),
		event: e.event?.label ?? null,
		href: finalView(e.spectator_token)
	};
}

/** A match the player recorded, whoever played it. */
export function recordedRow(e: HistoryEntry, timeZone?: string): ResultRowData {
	return {
		id: e.match_id,
		date: longDate(e.finished_at, timeZone),
		a: names(e.teams.a),
		b: names(e.teams.b),
		winner: e.winner,
		score: scoreText(e.score),
		chip: 'recorded',
		event: e.event?.label ?? null,
		href: finalView(e.spectator_token)
	};
}

/** Nothing played yet but matches recorded: prompt to link yourself to a player. */
export function showFindYourself(played: number, recorded: number): boolean {
	return played === 0 && recorded > 0;
}
