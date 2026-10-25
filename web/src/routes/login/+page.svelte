<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { authMessage, getSetup, login } from '#lib/auth.ts';
	import { safeNext } from '#lib/next.ts';
	import { signedIn } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';

	const t = strings.auth;

	let email = $state('');
	let password = $state('');
	let busy = $state(false);
	let failure = $state<string | null>(null);
	let needsSetup = $state(false);

	$effect(() => {
		getSetup().then(
			(s) => (needsSetup = s.needs_setup),
			() => {
				// Only a hint; the form works either way.
			}
		);
	});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		busy = true;
		failure = null;
		try {
			const me = await login(email, password);
			signedIn(me);
			await goto(safeNext(page.url.searchParams.get('next')));
		} catch (e) {
			failure = authMessage(e);
			password = '';
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct">
	<h1>{t.login.title}</h1>
	{#if needsSetup}
		<p class="ok">{t.login.setupFirst} <a href="/setup">{t.login.setupLink}</a></p>
	{/if}
	<form onsubmit={submit}>
		<label>
			{t.email}
			<input type="email" bind:value={email} autocomplete="username" required />
		</label>
		<label>
			{t.password}
			<input type="password" bind:value={password} autocomplete="current-password" required />
		</label>
		{#if failure}<p class="error" role="alert">{failure}</p>{/if}
		<button class="primary" type="submit" disabled={busy}>{busy ? t.signingIn : t.signIn}</button>
	</form>
</main>
