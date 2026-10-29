// The New chooser: the four things a person can start, and the old /new link that moved.

import { strings } from './strings';

export type ChoiceKey = 'match' | 'later' | 'board' | 'event';
export interface Choice {
	key: ChoiceKey;
	href: string;
	title: string;
	line: string;
}

const ORDER: ChoiceKey[] = ['match', 'later', 'board', 'event'];

/** The ways to start something, in the order the chooser shows them. */
export function choices(): Choice[] {
	return ORDER.map((key) => {
		const [title, line] = strings.create.choices[key];
		return { key, href: `/new/${key}`, title, line };
	});
}

/** Where an old `/new?from=…` link goes now (the query kept); null for a plain /new. */
export function legacyNewTarget(search: URLSearchParams): string | null {
	return search.has('from') ? `/new/match?${search}` : null;
}
