import { redirect } from '@sveltejs/kit';

// Starting a board moved under New. Old links keep their query.
export function load({ url }: { url: URL }) {
	redirect(307, '/new/board' + url.search);
}
