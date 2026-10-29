import { redirect } from '@sveltejs/kit';
import { legacyNewTarget } from '#lib/create.ts';

// Scoring a match moved to /new/match; an old `/new?from=…` link keeps its query.
export function load({ url }: { url: URL }) {
	const target = legacyNewTarget(url.searchParams);
	if (target !== null) redirect(307, target);
}
