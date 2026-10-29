import { redirect } from '@sveltejs/kit';

// The calendar moved into Home. Old links and push taps keep their query.
export function load({ url }: { url: URL }) {
	redirect(307, '/' + url.search);
}
