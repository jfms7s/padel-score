import { redirect } from '@sveltejs/kit';

// My results moved to Results. Old links keep their query.
export function load({ url }: { url: URL }) {
	redirect(307, '/results' + url.search);
}
