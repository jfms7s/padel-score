<script lang="ts">
	import { goto } from '$app/navigation';
	import { ApiError } from '#lib/api.ts';
	import { authMessage, getSetup, setup } from '#lib/auth.ts';
	import { signedIn } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';

	const t = strings.auth;

	let phase = $state<'loading' | 'open' | 'done' | 'failed'>('loading');
	let code = $state('');
	let name = $state('');
	let email = $state('');
	let password = $state('');
	let busy = $state(false);
	let failure = $state<string | null>(null);

	$effect(() => {
		getSetup().then(
			(s) => (phase = s.needs_setup ? 'open' : 'done'),
			(e: unknown) => {
				failure = authMessage(e);
				phase = 'failed';
			}
		);
	});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		busy = true;
		failure = null;
		try {
			const me = await setup({ setup_code: code, email, display_name: name, password });
			signedIn(me);
			await goto('/account');
		} catch (e) {
			if (e instanceof ApiError && e.code === 'setup_done') phase = 'done';
			failure = e instanceof ApiError && e.code === 'forbidden' ? t.errors.setup_code : authMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct">
	<h1>{t.setup.title}</h1>
	{#if phase === 'loading'}
		<p class="hint">{t.loading}</p>
	{:else if phase === 'done'}
		<p class="ok">{t.setup.done} <a href="/login">{t.signIn}</a></p>
	{:else if phase === 'failed'}
		<p class="error" role="alert">{failure}</p>
	{:else}
		<p>{t.setup.intro}</p>
		<form onsubmit={submit}>
			<label>
				{t.setup.code}
				<input bind:value={code} autocomplete="off" autocapitalize="characters" spellcheck="false" required aria-describedby="code-hint" />
				<span class="hint" id="code-hint">{t.setup.codeHint}</span>
			</label>
			<label>
				{t.name}
				<input bind:value={name} autocomplete="name" maxlength="40" required />
			</label>
			<label>
				{t.email}
				<input type="email" bind:value={email} autocomplete="username" required />
			</label>
			<label>
				{t.password}
				<input type="password" bind:value={password} autocomplete="new-password" minlength="8" maxlength="128" required aria-describedby="pw-hint" />
				<span class="hint" id="pw-hint">{t.passwordHint}</span>
			</label>
			{#if failure}<p class="error" role="alert">{failure}</p>{/if}
			<button class="primary" type="submit" disabled={busy}>{busy ? t.saving : t.setup.submit}</button>
		</form>
	{/if}
</main>
