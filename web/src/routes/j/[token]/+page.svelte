<script lang="ts">
	// An event's sign-up page (/j/{token}), and the partner link page (?partner=): anyone with
	// the link can look; a signed-in visitor signs up, answers a partner's invitation or leaves.
	// There is no stream for people who joined, so the page is read again after the user's own
	// actions, when it comes back into view and every 30 seconds while it is in view.
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { ApiError, redirectToSignIn } from '#lib/api.ts';
	import JoinEvent from '#lib/components/events/JoinEvent.svelte';
	import { session } from '#lib/session.ts';
	import { shareLink } from '#lib/share.ts';
	import {
		acceptPartner,
		declinePartner,
		getJoin,
		getPartnerInvite,
		joinEvent,
		leaveJoin,
		signupMessage
	} from '#lib/signups.ts';
	import { strings } from '#lib/strings.ts';
	import type { JoinAs, JoinView, PartnerInvite } from '#lib/types.ts';

	const t = strings.signups;
	const REFRESH_MS = 30_000;

	const token = $derived(page.params.token ?? '');
	const partnerToken = $derived(page.url.searchParams.get('partner'));
	const origin = $derived(page.url.origin);

	let view = $state<JoinView | null>(null);
	let loadFailure = $state<string | null>(null);
	let actionError = $state<string | null>(null);
	let note = $state<string | null>(null);
	let working = $state(false);
	let invite = $state<PartnerInvite | null>(null);
	let inviteGone = $state(false);

	const signedIn = $derived($session.status === 'signed_in');

	// A response only counts if nothing newer was asked for since.
	let latest = 0;

	async function load(forToken: string, quiet = false) {
		const mine = ++latest;
		try {
			const next = await getJoin(forToken);
			if (mine !== latest) return;
			view = next;
			loadFailure = null;
		} catch (e) {
			if (mine !== latest || quiet) return;
			loadFailure = e instanceof ApiError && e.code === 'not_found' ? t.notFound : signupMessage(e);
		}
	}

	async function loadInvite(forPartner: string | null) {
		invite = null;
		inviteGone = false;
		if (forPartner === null) return;
		try {
			const found = await getPartnerInvite(forPartner);
			if (forPartner === partnerToken) invite = found;
		} catch (e) {
			if (forPartner === partnerToken) inviteGone = e instanceof ApiError && e.code === 'not_found';
		}
	}

	$effect(() => {
		const current = token;
		untrack(() => {
			view = null;
			loadFailure = null;
			void load(current);
		});
		const again = () => {
			if (document.visibilityState === 'visible' && !working) void load(current, true);
		};
		const timer = setInterval(again, REFRESH_MS);
		document.addEventListener('visibilitychange', again);
		window.addEventListener('focus', again);
		return () => {
			clearInterval(timer);
			document.removeEventListener('visibilitychange', again);
			window.removeEventListener('focus', again);
		};
	});

	$effect(() => {
		const current = partnerToken;
		untrack(() => void loadInvite(current));
	});

	/** One action at a time; what the server has is shown again afterwards. */
	async function act(run: () => Promise<void>) {
		if (working) return;
		working = true;
		actionError = null;
		note = null;
		try {
			await run();
		} catch (e) {
			actionError = signupMessage(e);
		} finally {
			working = false;
			await load(token, true);
		}
	}

	const join = (as: JoinAs) => act(async () => void (await joinEvent(token, as)));
	const leave = () => act(() => leaveJoin(token));

	/** The invitation is spent either way: drop `?partner=` so a reload does not ask again. */
	const leaveInvite = () => goto(page.url.pathname, { replaceState: true });

	const accept = () =>
		act(async () => {
			if (partnerToken === null) return;
			await acceptPartner(partnerToken);
			await leaveInvite();
		});

	const decline = () =>
		act(async () => {
			if (partnerToken === null) return;
			await declinePartner(partnerToken);
			await leaveInvite();
			note = t.declined;
		});

	async function share(url: string, title: string) {
		const result = await shareLink(url, title);
		note = result === 'copied' ? strings.share.linkCopied : result === 'failed' ? strings.share.linkFailed : null;
	}
</script>

<main class="acct page-narrow">
	<a class="back" href="/">{strings.appName}</a>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{:else if view === null}
		<p class="hint">{t.loading}</p>
	{:else}
		<JoinEvent
			{view}
			{token}
			{origin}
			{signedIn}
			{working}
			error={actionError}
			{note}
			{invite}
			{inviteGone}
			onjoin={join}
			onleave={leave}
			onsignin={redirectToSignIn}
			onshare={share}
			onaccept={accept}
			ondecline={decline}
		/>
	{/if}
</main>

<style>
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
</style>
