<script lang="ts">
	// An event's sign-up page (/j/{token}), and the partner link page (?partner=): anyone with
	// the link can look; a signed-in visitor signs up, answers a partner's invitation or leaves.
	// The same link can be a scheduled match: its places, the line and the organiser's tools.
	// There is no stream for people who joined, so the page is read again after the user's own
	// actions, when it comes back into view and every 30 seconds while it is in view.
	import { untrack } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { ApiError, redirectToSignIn } from '#lib/api.ts';
	import JoinEvent from '#lib/components/events/JoinEvent.svelte';
	import JoinMatch from '#lib/components/JoinMatch.svelte';
	import { putStartHandoff, fromParam } from '#lib/handoff.ts';
	import {
		cancelScheduled,
		joinMatch,
		leaveMatch,
		removeFromScheduled,
		scheduledMessage,
		startScheduled,
		updateScheduled
	} from '#lib/scheduled.ts';
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
	import type { JoinAs, JoinView, PartnerInvite, ScheduledChange } from '#lib/types.ts';

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
	let inviteFailed = $state(false);

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
		inviteFailed = false;
		if (forPartner === null) return;
		try {
			const found = await getPartnerInvite(forPartner);
			if (forPartner === partnerToken) invite = found;
		} catch (e) {
			if (forPartner !== partnerToken) return;
			if (e instanceof ApiError && e.code === 'not_found') inviteGone = true;
			else inviteFailed = true;
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

	/** One action at a time; what the server has is shown again afterwards. True when it worked. */
	async function act(run: () => Promise<void>, message: (e: unknown) => string = signupMessage): Promise<boolean> {
		if (working) return false;
		working = true;
		actionError = null;
		note = null;
		let ok = false;
		try {
			await run();
			ok = true;
		} catch (e) {
			actionError = message(e);
		} finally {
			working = false;
			await load(token, true);
		}
		return ok;
	}

	const join = (as: JoinAs) => act(async () => void (await joinEvent(token, as)));
	const leave = () => act(() => leaveJoin(token));

	// A scheduled match: the owner's routes are by id, joining and leaving by the link.
	const matchAct = (run: () => Promise<unknown>) => act(async () => void (await run()), scheduledMessage);
	const matchId = $derived(view?.kind === 'match' ? view.view.id : '');

	const joinTheMatch = () => matchAct(() => joinMatch(token));
	const leaveTheMatch = () => matchAct(() => leaveMatch(token));
	const cancelTheMatch = () => matchAct(() => cancelScheduled(matchId));
	const removeFromMatch = (slot: number) => matchAct(() => removeFromScheduled(matchId, slot));
	const editTheMatch = (change: ScheduledChange) => matchAct(() => updateScheduled(matchId, change));

	/** The four players wait in this tab for New match, which sets teams and serve. */
	const startTheMatch = () =>
		matchAct(async () => {
			const started = await startScheduled(matchId);
			putStartHandoff(started);
			await goto(`/new?from=${encodeURIComponent(fromParam(started.from))}`);
		});

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
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{:else if view === null}
		<p class="hint">{t.loading}</p>
	{:else if view.kind === 'match'}
		<JoinMatch
			view={view.view}
			{origin}
			{signedIn}
			{working}
			error={actionError}
			{note}
			onjoin={joinTheMatch}
			onleave={leaveTheMatch}
			onsignin={redirectToSignIn}
			onshare={share}
			onstart={startTheMatch}
			oncancel={cancelTheMatch}
			onremove={removeFromMatch}
			onedit={editTheMatch}
		/>
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
			{inviteFailed}
			onretry={() => void loadInvite(partnerToken)}
			onjoin={join}
			onleave={leave}
			onsignin={redirectToSignIn}
			onshare={share}
			onaccept={accept}
			ondecline={decline}
		/>
	{/if}
</main>
