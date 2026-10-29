import { redirect } from '@sveltejs/kit';

// Matches I recorded are Results' Recorded view. Old links keep their other params.
export function load({ url }: { url: URL }) {
	const query = new URLSearchParams(url.search);
	query.set('view', 'recorded');
	redirect(307, `/results?${query}`);
}
