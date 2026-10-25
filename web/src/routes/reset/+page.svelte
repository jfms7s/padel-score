<script lang="ts">
	import { onMount } from 'svelte';
	import { replaceState } from '$app/navigation';
	import { page } from '$app/state';
	import { authMessage, resetPassword, takeFragmentCode } from '#lib/auth.ts';
	import { refreshSession } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';

	const t = strings.auth;

	let code: string | null = null;
	let phase = $state<'loading' | 'open' | 'done' | 'invalid'>('loading');
	let password = $state('');
	let busy = $state(false);
	let failure = $state<string | null>(null);

	onMount(() => {
		// As on /invite: read the code from the fragment, then take it out of the address bar.
		code = takeFragmentCode(location, {
			state: page.state,
			replaceState: (_state, _unused, url) => replaceState(url, page.state)
		});
		phase = code === null ? 'invalid' : 'open';
	});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy || code === null) return;
		busy = true;
		failure = null;
		try {
			await resetPassword(code, password);
			// The reset signs that user out everywhere: this browser too, if it was theirs.
			void refreshSession();
			phase = 'done';
		} catch (e) {
			failure = authMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct">
	<h1>{t.reset.title}</h1>
	{#if phase === 'loading'}
		<p class="hint">{t.loading}</p>
	{:else if phase === 'invalid'}
		<p class="error" role="alert">{t.reset.noCode}</p>
	{:else if phase === 'done'}
		<p class="ok" role="status">{t.reset.done}</p>
		<p><a class="primary" href="/login">{t.signIn}</a></p>
	{:else}
		<form onsubmit={submit}>
			<label>
				{t.reset.newPassword}
				<input type="password" bind:value={password} autocomplete="new-password" minlength="8" maxlength="128" required aria-describedby="pw-hint" />
				<span class="hint" id="pw-hint">{t.passwordHint}</span>
			</label>
			{#if failure}<p class="error" role="alert">{failure}</p>{/if}
			<button class="primary" type="submit" disabled={busy}>{busy ? t.saving : t.reset.submit}</button>
		</form>
	{/if}
</main>
