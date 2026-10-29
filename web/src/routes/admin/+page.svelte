<script lang="ts">
	import BackLink from '#lib/components/BackLink.svelte';
	import { onMount } from 'svelte';
	import { authMessage, createResetLink, disableUser, enableUser, listUsers } from '#lib/auth.ts';
	import LinkCode from '#lib/components/LinkCode.svelte';
	import { requireSignedIn } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { AdminUser, Me } from '#lib/types.ts';

	const t = strings.auth;
	const a = t.adminPage;

	let me = $state<Me | null>(null);
	let phase = $state<'loading' | 'ready' | 'not_admin' | 'failed'>('loading');
	let users = $state<AdminUser[]>([]);
	let failure = $state<string | null>(null);
	let busy = $state(false);
	let origin = $state('');
	/** The reset link just made: shown once, never stored. */
	let reset = $state<{ name: string; url: string } | null>(null);

	onMount(() => {
		origin = location.origin;
		void load();
	});

	async function load() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') {
			failure = strings.networkError;
			phase = 'failed';
		}
		if (s.status !== 'signed_in') return;
		me = s.me;
		if (!s.me.is_admin) {
			phase = 'not_admin';
			return;
		}
		try {
			users = await listUsers();
			phase = 'ready';
		} catch (e) {
			failure = authMessage(e);
			phase = 'failed';
		}
	}

	async function act(run: () => Promise<void>) {
		if (busy) return;
		busy = true;
		failure = null;
		try {
			await run();
		} catch (e) {
			failure = authMessage(e);
		} finally {
			busy = false;
		}
	}

	const toggle = (user: AdminUser) =>
		act(async () => {
			await (user.disabled ? enableUser(user.id) : disableUser(user.id));
			users = await listUsers();
		});

	const makeReset = (user: AdminUser) =>
		act(async () => {
			reset = null;
			const link = await createResetLink(user.id);
			reset = { name: user.display_name, url: origin + link.url };
		});
</script>

<main class="acct page-wide-desk">
	<BackLink />
	<h1>{a.title}</h1>
	{#if phase === 'loading'}
		<p class="hint">{t.loading}</p>
	{:else if phase === 'not_admin'}
		<p class="error" role="alert">{a.notAdmin}</p>
	{:else}
		{#if failure}<p class="error" role="alert">{failure}</p>{/if}
		{#if reset}
			<LinkCode url={reset.url} title={a.resetTitle(reset.name)} hint={a.resetHint} />
		{/if}
		{#if phase === 'ready'}
			<ul class="list grid-auto" style="--min: 360px" aria-label={a.title}>
				{#each users as user (user.id)}
					{@const self = user.id === me?.id}
					<li class="row">
						<span class="what">
							<strong>
								{user.display_name}
								{#if user.is_admin}<span class="badge">{a.adminBadge}</span>{/if}
								{#if user.disabled}<span class="badge off">{a.disabledBadge}</span>{/if}
								{#if self}<span class="badge">{a.you}</span>{/if}
							</strong>
							<span class="hint">{user.email}</span>
							<span class="hint">{user.invited_by_name ? a.invitedBy(user.invited_by_name) : a.firstUser}</span>
						</span>
						{#if !self}
							<span class="actions">
								<button
									class={user.disabled ? 'secondary' : 'danger'}
									type="button"
									disabled={busy}
									aria-label={user.disabled ? a.enableUser(user.display_name) : a.disableUser(user.display_name)}
									onclick={() => toggle(user)}>{user.disabled ? a.enable : a.disable}</button
								>
								<button
									class="secondary"
									type="button"
									disabled={busy}
									aria-label={a.resetLinkFor(user.display_name)}
									onclick={() => makeReset(user)}>{a.resetLink}</button
								>
							</span>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	{/if}
</main>
