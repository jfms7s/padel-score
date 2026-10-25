<script lang="ts">
	import { onMount } from 'svelte';
	import { goto, replaceState } from '$app/navigation';
	import { page } from '$app/state';
	import { acceptInvite, authMessage, lookupInvite, shortDate, takeFragmentCode } from '#lib/auth.ts';
	import { session, signedIn } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { InviteLookup } from '#lib/types.ts';

	const t = strings.auth;

	let code: string | null = null;
	let phase = $state<'loading' | 'open' | 'invalid'>('loading');
	let invite = $state<InviteLookup | null>(null);
	let name = $state('');
	let email = $state('');
	let password = $state('');
	let busy = $state(false);
	let failure = $state<string | null>(null);

	onMount(() => {
		// The code lives only in the fragment, which never reaches the server or a referrer. It is
		// taken out of the address bar as soon as it is read.
		code = takeFragmentCode(location, {
			state: page.state,
			replaceState: (_state, _unused, url) => replaceState(url, page.state)
		});
		if (code === null) {
			failure = t.invite.noCode;
			phase = 'invalid';
			return;
		}
		lookupInvite(code).then(
			(found) => {
				invite = found;
				phase = 'open';
			},
			(e: unknown) => {
				failure = authMessage(e);
				phase = 'invalid';
			}
		);
	});

	async function submit(event: SubmitEvent) {
		event.preventDefault();
		if (busy || code === null) return;
		busy = true;
		failure = null;
		try {
			const me = await acceptInvite(code, { email, display_name: name, password });
			signedIn(me);
			await goto('/');
		} catch (e) {
			failure = authMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<main class="acct">
	<h1>{t.invite.title}</h1>
	{#if phase === 'loading'}
		<p class="hint">{t.loading}</p>
	{:else if phase === 'invalid'}
		<p class="error" role="alert">{failure}</p>
	{:else if invite}
		<p>{t.invite.invitedBy(invite.invited_by_name)} {t.invite.expires(shortDate(invite.expires_at))}</p>
		{#if $session.status === 'signed_in'}
			<p class="hint">{t.invite.signedInNote}</p>
		{/if}
		<form onsubmit={submit}>
			<label>
				{t.name}
				<input bind:value={name} autocomplete="name" maxlength="40" required aria-describedby="name-hint" />
				<span class="hint" id="name-hint">{t.nameHint}</span>
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
			<button class="primary" type="submit" disabled={busy}>{busy ? t.saving : t.invite.submit}</button>
		</form>
	{/if}
</main>
