// The court phone (/c/{token}): which screen it shows, and the small pure pieces around it.
// The page itself only wires these to the court stream, the match's view and the fence.

import { ApiError, type FinalOutcome } from './api';
import { liveScoreText, teamText } from './events';
import { scoreText } from './players';
import type { StorageLike } from './setup';
import { strings } from './strings';
import type { CourtMatch, CourtState, EventMatch, EventTeam, MatchView } from './types';

/**
 * - `join`: nobody has tapped Join on this phone yet (the tap unlocks audio and the wake lock);
 * - `waiting`: no current match on the court;
 * - `loading`: there is one, but its view hasn't arrived yet;
 * - `serve`: nothing scored yet, so choose who serves first;
 * - `live` / `finished`: the fence, scoring or showing the final score;
 * - `gone`: no court any more (its link was replaced, or the event finished).
 */
export type CourtScreen = 'join' | 'waiting' | 'loading' | 'serve' | 'live' | 'finished' | 'gone';

/**
 * The screen for a court `state` (null once the link is dead), the current match's `view`, and
 * whether this phone `joined`. `startedHere` is the match this phone already chose the server
 * for: choosing writes no event, so the empty log alone would keep asking.
 */
export function courtScreen(
	state: CourtState | null,
	view: MatchView | null,
	joined: boolean,
	startedHere: string | null = null
): CourtScreen {
	if (state === null) return 'gone';
	if (!joined) return 'join';
	const current = state.current;
	if (current === null) return 'waiting';
	if (view === null || view.id !== current.id) return 'loading';
	if (typeof view.score.status === 'object') return 'finished';
	if (view.head_seq === 0 && startedHere !== current.id) return 'serve';
	return 'live';
}

/** "Ana / Marta v Rui / João". */
export function courtMatchText(m: Pick<CourtMatch, 'teams'>): string {
	return strings.events.versus(strings.teamName(...m.teams.a), strings.teamName(...m.teams.b));
}

/** Why a court link stopped working: 404 replaced (a new scoring code), 410 the event finished. */
export function goneReason(e: unknown): 'replaced' | 'finished' | null {
	if (!(e instanceof ApiError)) return null;
	if (e.status === 404) return 'replaced';
	if (e.status === 410) return 'finished';
	return null;
}

/**
 * A typed final's refusal as the error the typed score sheet explains in place, or null when
 * it was applied. `postFinal` answers with outcomes; the sheet wants a throw.
 */
export function finalRefusal(out: FinalOutcome): ApiError | null {
	switch (out.status) {
		case 'applied':
			return null;
		case 'conflict':
			return new ApiError(409, 'conflict', '');
		case 'locked':
			return new ApiError(409, 'locked', '');
		case 'invalid_score':
			return new ApiError(422, 'invalid_score', '', false, { reason: out.reason });
	}
}

/** Joining is remembered per court link for the browser tab (sessionStorage). */
export const JOINED_KEY_PREFIX = 'padel-score:court-joined:';

export function loadJoined(storage: StorageLike, token: string): boolean {
	try {
		return storage.getItem(JOINED_KEY_PREFIX + token) === '1';
	} catch {
		return false;
	}
}

export function saveJoined(storage: StorageLike, token: string): void {
	try {
		storage.setItem(JOINED_KEY_PREFIX + token, '1');
	} catch {
		// Not remembered: a reload asks for the tap again.
	}
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;

/** A loose check of a court stream frame. */
export function isCourtState(v: unknown): v is CourtState {
	return (
		isObject(v) &&
		isObject(v.event) &&
		typeof v.event.name === 'string' &&
		isObject(v.court) &&
		typeof v.court.name === 'string' &&
		(v.current === null || isObject(v.current)) &&
		(v.next === null || isObject(v.next))
	);
}

/**
 * A match's line on the event page: "6-4 · Winner: Ana / Marta" once finished, the live score
 * while it is played, else null.
 */
export function matchResultText(
	m: Pick<EventMatch, 'state' | 'result' | 'winner' | 'live' | 'team_a' | 'team_b'>,
	teams: readonly EventTeam[]
): string | null {
	if (m.state === 'finished' && m.result) {
		const score = scoreText(m.result);
		if (m.winner === null) return `${score} · ${strings.events.draw}`;
		const winner = teams.find((x) => x.id === (m.winner === 'a' ? m.team_a : m.team_b));
		return winner ? `${score} · ${strings.events.sheet.winner(teamText(winner))}` : score;
	}
	if (m.state === 'live' && m.live) return liveScoreText(m.live);
	return null;
}
