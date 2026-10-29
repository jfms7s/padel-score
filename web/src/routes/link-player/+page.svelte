<script lang="ts">
	// What a coordinator's link QR code opens: /link-player#<code>. The signed-in player sees which
	// entry it is and confirms "Yes, that's me". Signed out, the code is kept across sign-in.
	import { onMount } from 'svelte';
	import { goto, replaceState } from '$app/navigation';
	import { page } from '$app/state';
	import { redirectToSignIn } from '#lib/api.ts';
	import { takeFragmentCode } from '#lib/auth.ts';
	import {
		acceptLinkCode,
		linkMessage,
		lookupLinkCode,
		previewText,
		stashLinkCode,
		takeStashedLinkCode
	} from '#lib/linking.ts';
	import { bootSession } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { ClaimPreview } from '#lib/types.ts';

	const t = strings.linking.page;

	let code: string | null = null;
	let phase = $state<'loading' | 'open' | 'invalid'>('loading');
	let preview = $state<ClaimPreview | null>(null);
	let busy = $state(false);
	let failure = $state<string | null>(null);

	onMount(() => {
		// The code lives only in the fragment, which never reaches the server or a referrer, and is
		// taken out of the address bar as soon as it is read.
		code = takeFragmentCode(location, {
			state: page.state,
			replaceState: (_state, _unused, url) => replaceState(url, page.state)
		});
		void start();
	});

	async function start() {
		const s = await bootSession();
		if (s.status === 'signed_out' || s.status === 'unavailable') {
			// Keep the scanned code across sign-in (or until the server is back).
			if (code !== null) stashLinkCode(code);
			if (s.status === 'signed_out') redirectToSignIn();
			else fail(strings.networkError);
			return;
		}
		// Back from sign-in there is no fragment: take the stashed code.
		code ??= takeStashedLinkCode();
		if (code === null) return fail(t.noCode);
		try {
			preview = await lookupLinkCode(code);
			phase = 'open';
		} catch (e) {
			fail(linkMessage(e));
		}
	}

	function fail(message: string) {
		failure = message;
		phase = 'invalid';
	}

	async function confirm() {
		if (busy || code === null) return;
		busy = true;
		failure = null;
		try {
			await acceptLinkCode(code);
			await goto('/results');
		} catch (e) {
			failure = linkMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct">
	<h1>{t.title}</h1>
	{#if phase === 'loading'}
		<p class="hint">{strings.auth.loading}</p>
	{:else if phase === 'invalid'}
		<p class="error" role="alert">{failure}</p>
	{:else if preview}
		<p><strong>{t.is(preview.name, preview.coordinator_name)}</strong></p>
		<p class="hint">{previewText(preview)}</p>
		{#if failure}<p class="error" role="alert">{failure}</p>{/if}
		<button class="primary" type="button" disabled={busy} onclick={confirm}>{busy ? t.confirming : t.yes}</button>
	{/if}
</main>
