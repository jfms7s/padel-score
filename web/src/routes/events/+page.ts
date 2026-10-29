import { redirect } from '@sveltejs/kit';

// The events list moved into Organise. Old links keep their query.
export function load({ url }: { url: URL }) {
	redirect(307, '/organise' + url.search);
}
