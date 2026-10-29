<script lang="ts">
	// Home: the matches being scored, what waits for an answer, and the week ahead.
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import { authMessage, listMyMatches } from '#lib/auth.ts';
	import { answerRequest, boardMessage } from '#lib/boards.ts';
	import {
		dismissPushPrompt,
		getCalendar,
		itemStart,
		pushPromptDismissed,
		startOfToday,
		weekStart
	} from '#lib/calendar.ts';
	import Agenda from '#lib/components/home/Agenda.svelte';
	import AgendaWeek from '#lib/components/home/AgendaWeek.svelte';
	import BoardsStrip from '#lib/components/home/BoardsStrip.svelte';
	import DetailPane from '#lib/components/home/DetailPane.svelte';
	import LiveNow from '#lib/components/home/LiveNow.svelte';
	import OpenToJoin from '#lib/components/home/OpenToJoin.svelte';
	import Waiting from '#lib/components/home/Waiting.svelte';
	import WeekStrip from '#lib/components/home/WeekStrip.svelte';
	import DeleteMatchSheet from '#lib/components/DeleteMatchSheet.svelte';
	import { addDays, browserTimeZone, localDate } from '#lib/dates.ts';
	import { isCancelled, itemEnd, itemKey, liveNow, offlineRows, type LiveRow } from '#lib/home.ts';
	import { claimBadgeCount } from '#lib/linking.ts';
	import { listPlayers } from '#lib/players.ts';
	import { currentPushEnv, devicePushOn, enablePush, pushSupport } from '#lib/push.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { shareLink } from '#lib/share.ts';
	import { partnerLink } from '#lib/signups.ts';
	import { listRecent, removeRecent } from '#lib/setup.ts';
	import { safeStorage } from '#lib/storage.ts';
	import { strings } from '#lib/strings.ts';
	import type { Calendar, MyMatch, WaitingItem } from '#lib/types.ts';

	const zone = browserTimeZone();

	// Home is for a signed-in user: signed out, the browser goes to sign-in. With the server out
	// of reach it still shows what this device knows.
	$effect(() => {
		void requireSignedIn();
	});
	const signedIn = $derived($session.status === 'signed_in');
	const offline = $derived($session.status === 'unavailable');
	const shown = $derived(signedIn || offline);

	// Signed in, the server's list of live matches this user started, on any device.
	let mine = $state<MyMatch[] | null>(null);
	let mineFailure = $state<string | null>(null);
	let recent = $state(listRecent(safeStorage()));
	// The clock the live list is split by; moves on whenever the page comes back.
	let now = $state(Date.now() / 1000);
	const live = $derived(mine ? liveNow(mine, now, zone) : { fresh: [], older: [] });
	const offlineList = $derived(offlineRows(recent, zone));

	$effect(() => {
		if (!signedIn) return;
		let alive = true;
		listMyMatches().then(
			(list) => {
				if (alive) mine = list;
			},
			(e: unknown) => {
				if (alive) mineFailure = authMessage(e);
			}
		);
		return () => {
			alive = false;
		};
	});

	// Claims waiting on the coordinator, summed over the player list. Best effort: none shown on failure.
	let claimCount = $state(0);
	$effect(() => {
		if (!signedIn) return;
		let alive = true;
		listPlayers({ quiet: true }).then(
			(list) => {
				if (alive) claimCount = claimBadgeCount(list);
			},
			() => {}
		);
		return () => {
			alive = false;
		};
	});

	// The calendar: what waits for an answer, the week and the agenda, the boards and what is open.
	let cal = $state<Calendar | null>(null);
	let failure = $state<string | null>(null);
	let actionError = $state<string | null>(null);
	let working = $state(false);
	let note = $state<string | null>(null);

	// Recomputed whenever the page comes back, so a page left open past midnight moves on.
	let today = $state(localDate(Date.now() / 1000, zone));
	const thisWeek = $derived(weekStart(today));
	let weekFirst = $state(weekStart(localDate(Date.now() / 1000, zone)));

	// The notifications card: shown once, to someone whose browser can and who has not said no.
	let pushCard = $state(false);
	let pushFailure = $state<string | null>(null);

	onMount(() => {
		// Coming back to the page: what waits for an answer may be gone, and the day may have changed.
		const wake = () => {
			if (document.visibilityState === 'hidden' || cal === null) return;
			void refresh();
		};
		document.addEventListener('visibilitychange', wake);
		window.addEventListener('focus', wake);
		return () => {
			document.removeEventListener('visibilitychange', wake);
			window.removeEventListener('focus', wake);
		};
	});

	let started = false;
	$effect(() => {
		if (!signedIn || started) return;
		started = true;
		void refresh().then(checkPush);
	});

	async function refresh() {
		try {
			const secs = Date.now() / 1000;
			now = secs;
			const fresh = localDate(secs, zone);
			if (fresh !== today) {
				// The day rolled over: today moves, and the week view follows if it showed the old week.
				if (weekFirst === thisWeek) weekFirst = weekStart(fresh);
				today = fresh;
			}
			cal = await getCalendar(startOfToday(zone, secs));
			failure = null;
		} catch (e) {
			failure = boardMessage(e);
		}
	}

	async function checkPush() {
		if (pushSupport(currentPushEnv()) !== 'available') return;
		if (pushPromptDismissed(safeStorage())) return;
		try {
			pushCard = !(await devicePushOn());
		} catch {
			pushCard = false;
		}
	}

	async function turnOn() {
		const result = await enablePush();
		if (result === 'on') pushCard = false;
		else pushFailure =
				result === 'denied'
					? strings.push.denied
					: result === 'dismissed'
						? strings.push.dismissed
						: strings.push.failed;
	}

	function notNow() {
		dismissPushPrompt(safeStorage());
		pushCard = false;
	}

	async function answer(id: string, accept: boolean) {
		working = true;
		actionError = null;
		try {
			await answerRequest(id, accept);
		} catch (e) {
			actionError = boardMessage(e);
		} finally {
			working = false;
		}
		await refresh();
	}

	async function resend(w: Extract<WaitingItem, { kind: 'partner' }>) {
		const result = await shareLink(
			partnerLink(location.origin, w.join_token, w.partner_token),
			strings.calendar.partnerShareTitle(w.event_name)
		);
		note =
			result === 'copied' ? strings.calendar.linkCopied : result === 'failed' ? strings.calendar.linkFailed : null;
	}

	// The week grid's selection lives in ?item=. With none (or a stale one) the first item that
	// ends after now and isn't cancelled is selected, in the week the grid shows if there is one.
	const chosen = $derived.by(() => {
		const items = cal?.items ?? [];
		const wanted = page.url.searchParams.get('item');
		const coming = items.filter((it) => !isCancelled(it) && itemEnd(it) > now);
		const inWeek = coming.find((it) => {
			const date = localDate(itemStart(it), zone);
			return date >= weekFirst && date <= addDays(weekFirst, 6);
		});
		return items.find((it) => itemKey(it) === wanted) ?? inWeek ?? coming[0] ?? null;
	});
	const chosenKey = $derived(chosen ? itemKey(chosen) : null);

	/** Select in place: no new history entry, and scroll and focus stay put. */
	function select(key: string): Promise<void> {
		const params = new URLSearchParams(page.url.searchParams.toString());
		params.set('item', key);
		return goto(`?${params}`, { replace: true, reset: false });
	}

	function toDay(date: string) {
		document.getElementById(`day-${date}`)?.scrollIntoView({ block: 'start' });
	}

	// The match whose delete sheet is open. Once it is gone (or kept as finished) it leaves the list.
	let deleting = $state<LiveRow | null>(null);
	function closeDelete(removed: boolean) {
		const token = deleting?.scorerToken;
		deleting = null;
		if (!removed || token === undefined) return;
		const storage = safeStorage();
		removeRecent(storage, token);
		recent = listRecent(storage);
		if (mine) mine = mine.filter((m) => m.scorer_token !== token);
	}
</script>

<main class="page-wide">
	{#if shown}
		<h1>{strings.home.title}</h1>

		<div class="cols">
			<div class="live">
				{#if offline}
					<LiveNow fresh={offlineList} older={[]} offline={true} ondelete={(row) => (deleting = row)} />
					<p class="hint">{strings.networkError}</p>
				{:else if mineFailure}
					<p class="hint" role="alert">{mineFailure}</p>
				{:else if mine === null}
					<p class="hint">{strings.auth.loading}</p>
				{:else}
					<LiveNow fresh={live.fresh} older={live.older} offline={false} ondelete={(row) => (deleting = row)} />
				{/if}
			</div>
			<div class="waiting">
				{#if signedIn && cal}
					<Waiting
						{cal}
						claims={claimCount}
						{pushCard}
						onanswer={answer}
						onresend={resend}
						onpush={turnOn}
						onnotnow={notNow}
						busy={working}
						error={actionError}
						{pushFailure}
						{note}
					/>
				{/if}
			</div>
			<div class="week">
				{#if signedIn}
					{#if failure}
						<p class="error" role="alert">{failure}</p>
					{:else if cal === null}
						<p class="hint">{strings.calendar.loading}</p>
					{/if}
					{#if cal}
						<div class="below">
							<WeekStrip
								items={cal.items}
								first={weekFirst}
								{today}
								timeZone={zone}
								onweek={(first) => (weekFirst = first)}
								onday={toDay}
							/>
							<Agenda items={cal.items} timeZone={zone} />
						</div>
						<div class="above">
							<AgendaWeek
								items={cal.items}
								open={cal.open}
								first={weekFirst}
								{today}
								timeZone={zone}
								selected={chosenKey}
								onselect={select}
								onweek={(first) => (weekFirst = first)}
							/>
						</div>
					{/if}
				{/if}
			</div>
			<div class="detail above">
				{#if signedIn && cal}
					<h2>{strings.home.selected}</h2>
					<DetailPane item={chosen} timeZone={zone} />
				{/if}
			</div>
			<div class="boards">
				{#if signedIn && cal}<BoardsStrip boards={cal.boards} />{/if}
			</div>
			<div class="open below">
				{#if signedIn && cal}<OpenToJoin open={cal.open} />{/if}
			</div>
		</div>
	{/if}
</main>

{#if deleting}
	<DeleteMatchSheet label={deleting.label} scorerToken={deleting.scorerToken} onclose={closeDelete} />
{/if}

<style>
	main {
		padding-top: 16px;
		padding-bottom: 48px;
	}
	/* A phone column below 700; the two-column layout is wide at every width from there. */
	.cols,
	h1 {
		max-width: 28rem;
	}
	h1 {
		margin: 0 0 12px;
		font-size: 32px;
		font-weight: 800;
		font-variation-settings: 'wdth' 75;
	}
	.hint {
		margin: 0 0 8px;
		color: var(--muted);
	}
	.error {
		color: var(--warn);
	}
	h2 {
		margin: 24px 0 8px;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	/* The week grid and its detail are for 1100 and up. */
	.above {
		display: none;
	}
	/* Below 700 one column in reading order: live, waiting, week, agenda, boards, open. */
	.cols {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}
	.cols > div {
		min-width: 0;
	}
	/* From 700 the week and agenda stand on the left, the rest stack on the right. */
	@media (min-width: 700px) {
		.cols,
		h1 {
			max-width: none;
		}
		.cols {
			display: grid;
			grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr);
			column-gap: 32px;
			/* A spare 1fr row soaks up the tall week column, so the right sections stack tightly. */
			grid-template-rows: repeat(4, auto) 1fr;
			align-items: start;
		}
		.cols > div {
			grid-column: 2;
		}
		.cols > .week {
			grid-column: 1;
			grid-row: 1 / span 5;
		}
	}
	/* From 1100 the week is a grid with a detail pane; the strip, agenda and open list give way. */
	@media (min-width: 1100px) {
		.above {
			display: block;
		}
		.below {
			display: none;
		}
	}
</style>
