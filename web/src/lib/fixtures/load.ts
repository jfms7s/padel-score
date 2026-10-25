// Test helper: a recorded fixture as a MatchView, checked against the wire types first.

import { isMatchView } from '../guards';
import type { MatchView } from '../types';

export function asView(json: unknown): MatchView {
	if (!isMatchView(json)) throw new Error(`fixture is not a MatchView: ${JSON.stringify(json).slice(0, 200)}`);
	return json;
}
