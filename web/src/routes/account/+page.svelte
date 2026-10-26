<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import {
		authMessage,
		changePassword,
		createInvite,
		listDevices,
		listInvites,
		logout,
		revokeDevice,
		revokeInvite,
		shortDate
	} from '#lib/auth.ts';
	import LinkCode from '#lib/components/LinkCode.svelte';
	import { requireSignedIn, session, signedOut } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';
	import type { CreatedInvite, Device, Invite } from '#lib/types.ts';

	const t = strings.auth;
	const a = t.accountPage;

	let ready = $state(false);
	let loadFailure = $state<string | null>(null);

	let invites = $state<Invite[]>([]);
	let devices = $state<Device[]>([]);
	let fresh = $state<CreatedInvite | null>(null);
	let inviteFailure = $state<string | null>(null);
	let deviceFailure = $state<string | null>(null);
	/** Whether each list has loaded: the empty state is for a list that did, never for an error. */
	let invitesLoaded = $state(false);
	let devicesLoaded = $state(false);

	let current = $state('');
	let next = $state('');
	let pwBusy = $state(false);
	let pwFailure = $state<string | null>(null);
	let pwDone = $state(false);

	let busy = $state(false);
	let origin = $state('');

	onMount(() => {
		origin = location.origin;
		void load();
	});

	async function load() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') loadFailure = strings.networkError;
		if (s.status !== 'signed_in') return;
		// Each section loads on its own, so one failing leaves the rest usable.
		const [inv, dev] = await Promise.allSettled([listInvites(), listDevices()]);
		if (inv.status === 'fulfilled') {
			invites = inv.value;
			invitesLoaded = true;
		}
		else inviteFailure = authMessage(inv.reason);
		if (dev.status === 'fulfilled') {
			devices = dev.value;
			devicesLoaded = true;
		}
		else deviceFailure = authMessage(dev.reason);
		ready = true;
	}

	async function submitPassword(event: SubmitEvent) {
		event.preventDefault();
		if (pwBusy) return;
		pwBusy = true;
		pwFailure = null;
		pwDone = false;
		try {
			await changePassword(current, next);
			current = '';
			next = '';
			pwDone = true;
		} catch (e) {
			pwFailure = authMessage(e);
		} finally {
			pwBusy = false;
		}
		// The change signed every watch out.
		if (pwDone) void listDevices().then(
				(list) => {
					devices = list;
					devicesLoaded = true;
				},
				() => {}
			);
	}

	async function act(run: () => Promise<void>, onFail: (m: string) => void) {
		if (busy) return;
		busy = true;
		try {
			await run();
		} catch (e) {
			onFail(authMessage(e));
		} finally {
			busy = false;
		}
	}

	const newInvite = () =>
		act(
			async () => {
				inviteFailure = null;
				fresh = await createInvite();
				invites = await listInvites();
				invitesLoaded = true;
			},
			(m) => (inviteFailure = m)
		);

	const dropInvite = (id: string) =>
		act(
			async () => {
				inviteFailure = null;
				await revokeInvite(id);
				if (fresh?.id === id) fresh = null;
				invites = await listInvites();
				invitesLoaded = true;
			},
			(m) => (inviteFailure = m)
		);

	const dropDevice = (id: string) =>
		act(
			async () => {
				deviceFailure = null;
				await revokeDevice(id);
				devices = await listDevices();
				devicesLoaded = true;
			},
			(m) => (deviceFailure = m)
		);

	const signOut = () =>
		act(
			async () => {
				await logout();
				signedOut();
				await goto('/');
			},
			(m) => (loadFailure = m)
		);
</script>

<main class="acct page-wide-desk">
	<h1>{a.title}</h1>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{/if}
	{#if $session.status === 'signed_in' && ready}
		{@const me = $session.me}
		<p>{a.signedInAs(me.display_name, me.email)}</p>
		<div class="actions">
			<a class="secondary" href="/profile">{strings.linking.profile.link}</a>
			<a class="secondary" href="/link">{a.link}</a>
			{#if me.is_admin}<a class="secondary" href="/admin">{a.admin}</a>{/if}
		</div>

		<div class="secs">
		<section>
		<h2>{a.invites}</h2>
		<p class="hint">{a.invitesHint}</p>
		<button class="primary" type="button" onclick={newInvite} disabled={busy}>{a.createInvite}</button>
		{#if fresh}
			<LinkCode url={origin + fresh.url} title={a.newInvite} hint={a.newInviteHint} />
		{/if}
		{#if inviteFailure}<p class="error" role="alert">{inviteFailure}</p>{/if}
		{#if !invitesLoaded}
			<!-- The error above says why. -->
		{:else if invites.length === 0}
			<p class="hint">{a.noInvites}</p>
		{:else}
			<ul class="list" aria-label={a.invites}>
				{#each invites as invite (invite.id)}
					<li class="row">
						<span class="what">
							<strong>{a.inviteStatus[invite.status]}</strong>
							<span class="hint">
								{invite.used_by_name ? a.usedBy(invite.used_by_name) : a.madeOn(shortDate(invite.created_at))}
							</span>
						</span>
						{#if invite.status === 'active'}
							<button
								class="danger"
								type="button"
								disabled={busy}
								aria-label={a.revokeInvite(shortDate(invite.created_at))}
								onclick={() => dropInvite(invite.id)}>{a.revoke}</button
							>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
		</section>

		<section>
		<h2>{a.devices}</h2>
		<p class="hint">{a.devicesHint}</p>
		{#if deviceFailure}<p class="error" role="alert">{deviceFailure}</p>{/if}
		{#if !devicesLoaded}
			<!-- The error above says why. -->
		{:else if devices.length === 0}
			<p class="hint">{a.noDevices}</p>
		{:else}
			<ul class="list" aria-label={a.devices}>
				{#each devices as device (device.id)}
					<li class="row">
						<span class="what">
							<strong>{device.name}</strong>
							<span class="hint">
								{device.prefix}… · {device.last_used_at ? a.lastUsed(shortDate(device.last_used_at)) : a.neverUsed}
							</span>
						</span>
						<button
							class="danger"
							type="button"
							disabled={busy}
							aria-label={a.revokeDevice(device.name)}
							onclick={() => dropDevice(device.id)}>{a.revoke}</button
						>
					</li>
				{/each}
			</ul>
		{/if}
		</section>

		<section>
		<h2>{a.password}</h2>
		<form onsubmit={submitPassword}>
			<label>
				{a.currentPassword}
				<input type="password" bind:value={current} autocomplete="current-password" required />
			</label>
			<label>
				{a.newPassword}
				<input type="password" bind:value={next} autocomplete="new-password" minlength="8" maxlength="128" required aria-describedby="pw-hint" />
				<span class="hint" id="pw-hint">{t.passwordHint}</span>
			</label>
			{#if pwFailure}<p class="error" role="alert">{pwFailure}</p>{/if}
			{#if pwDone}<p class="ok" role="status">{a.passwordChanged}</p>{/if}
			<button class="primary" type="submit" disabled={pwBusy}>{pwBusy ? t.saving : a.password}</button>
		</form>
		</section>

		<section>
		<h2>{t.signOut}</h2>
		<button class="danger" type="button" onclick={signOut} disabled={busy}>{t.signOut}</button>
		</section>
		</div>
	{:else if !loadFailure}
		<p class="hint">{t.loading}</p>
	{/if}
</main>

<style>
	@media (min-width: 1100px) {
		.secs {
			display: grid;
			grid-template-columns: repeat(2, minmax(0, 1fr));
			gap: 0 32px;
			align-items: start;
		}
	}
</style>
