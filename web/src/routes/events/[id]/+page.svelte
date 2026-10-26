<script lang="ts">
	// The coordinator's event desk: a header, a tab bar and the open tab (see desk.ts). The page
	// loads the event, follows its stream (offering a reload when it is lost) and runs every
	// call; the tabs only show and ask.
	import { onDestroy, onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import KindPanel from '#lib/components/events/KindPanel.svelte';
	import CourtsTab from '#lib/components/events/desk/CourtsTab.svelte';
	import DeskHeader from '#lib/components/events/desk/DeskHeader.svelte';
	import DeskTabs from '#lib/components/events/desk/DeskTabs.svelte';
	import FinishTab from '#lib/components/events/desk/FinishTab.svelte';
	import LiveTab from '#lib/components/events/desk/LiveTab.svelte';
	import MatchesTab from '#lib/components/events/desk/MatchesTab.svelte';
	import TeamsTab from '#lib/components/events/desk/TeamsTab.svelte';
	import { defaultTab, deskTabs, resolveTab, type DeskTab } from '#lib/desk.ts';
	import {
		addCourt,
		addTeam,
		advanceCourt,
		assignMatch,
		createEventMatch,
		deleteTeam,
		eventLink,
		eventMessage,
		eventStreamUrl,
		finishEvent,
		getEvent,
		isEventDetail,
		postEventResult,
		rotateCourt,
		startEvent,
		updateTeam
	} from '#lib/events.ts';
	import { listPlayers } from '#lib/players.ts';
	import { requireSignedIn, session } from '#lib/session.ts';
	import { openSseStream, type StreamStatus } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { CourtOut, EventDetail, EventMatch, EventTeam, NewEventMatch, Place, Player, TypedScore } from '#lib/types.ts';

	const t = strings.events;

	const id = $derived(page.params.id ?? '');
	const origin = $derived(page.url.origin);

	let detail = $state<EventDetail | null>(null);
	let loadFailure = $state<string | null>(null);
	let streamStatus = $state<StreamStatus | null>(null);
	let stream: { close(): void } | null = null;

	/** The signed-in user's players, for adding teams; null until loaded or if unreadable. */
	let players = $state<Player[] | null>(null);
	let playersFailed = $state(false);

	type Section = 'teams' | 'courts' | 'matches' | 'lifecycle';
	let busy = $state(false);
	let failures = $state<Partial<Record<Section, string>>>({});

	const ev = $derived(detail?.event ?? null);
	const status = $derived(ev?.status ?? 'setup');
	const teams = $derived(detail?.teams ?? []);
	const courts = $derived(detail?.courts ?? []);
	const matches = $derived([...(detail?.matches ?? [])].sort((x, y) => x.ordinal - y.ordinal));

	onMount(() => {
		void load();
	});
	onDestroy(() => stream?.close());

	async function load() {
		const s = await requireSignedIn();
		if (s.status === 'unavailable') loadFailure = strings.networkError;
		if (s.status !== 'signed_in') return;
		try {
			detail = await getEvent(id);
		} catch (e) {
			loadFailure = eventMessage(e);
			return;
		}
		stream = openSseStream(
			eventStreamUrl(id),
			{ type: 'event', accept: isEventDetail },
			(d) => (detail = d),
			(st) => (streamStatus = st)
		);
		try {
			players = await listPlayers({ quiet: true });
		} catch {
			playersFailed = true;
		}
	}

	async function refresh() {
		detail = await getEvent(id);
	}

	/** Run one action at a time; its failure shows in its own section. */
	async function act(section: Section, run: () => Promise<void>): Promise<boolean> {
		if (busy) return false;
		busy = true;
		failures = {};
		try {
			await run();
			return true;
		} catch (e) {
			failures = { [section]: eventMessage(e) };
			return false;
		} finally {
			busy = false;
		}
	}

	// ---- copying links ---------------------------------------------------------------------

	let copied = $state<string | null>(null);
	let copyFailed = $state(false);

	async function copy(key: string, link: string) {
		copyFailed = false;
		try {
			await navigator.clipboard.writeText(link);
			copied = key;
			setTimeout(() => {
				if (copied === key) copied = null;
			}, 2000);
		} catch {
			copyFailed = true;
		}
	}

	/** Show a message in one section, or clear them all with null. */
	function fail(section: Section, message: string | null) {
		failures = message === null ? {} : { [section]: message };
	}

	// ---- tabs ------------------------------------------------------------------------------

	/** A non-stop has nothing to show in its kind tab until it starts. */
	const tabs = $derived(deskTabs(status).filter((tab) => !(tab === 'kind' && ev?.kind === 'non_stop' && status === 'setup')));
	const active = $derived.by(() => {
		const tab = resolveTab(page.url.searchParams.get('tab'), status);
		return tabs.includes(tab) ? tab : defaultTab(status);
	});

	/** Swap the tab in place: no new history entry, and scroll and focus stay put. */
	function select(tab: DeskTab) {
		void goto(`?tab=${tab}`, { replace: true, reset: false });
	}

	// ---- teams -----------------------------------------------------------------------------

	function added(player: Player) {
		if (!players) return;
		players = [...players.filter((p) => p.id !== player.id), player];
	}

	function newTeam(pair: [string, string], name: string): Promise<boolean> {
		return act('teams', async () => {
			await addTeam(id, name === '' ? { players: pair } : { players: pair, name });
			await refresh();
		});
	}

	function renameTeam(team: EventTeam, name: string): Promise<boolean> {
		return act('teams', async () => {
			await updateTeam(id, team.id, { name: name === '' ? null : name });
			await refresh();
		});
	}

	function removeTeam(team: EventTeam) {
		void act('teams', async () => {
			await deleteTeam(id, team.id);
			await refresh();
		});
	}

	// ---- courts ----------------------------------------------------------------------------

	function rotate(court: CourtOut): Promise<boolean> {
		return act('courts', async () => {
			await rotateCourt(id, court.id);
			await refresh();
		});
	}

	function nextMatch(court: CourtOut) {
		void act('courts', async () => {
			detail = await advanceCourt(id, court.id);
		});
	}

	function newCourt() {
		void act('courts', async () => {
			await addCourt(id);
			await refresh();
		});
	}

	// ---- matches ---------------------------------------------------------------------------

	function newMatch(body: NewEventMatch): Promise<boolean> {
		return act('matches', async () => {
			await createEventMatch(id, body);
			await refresh();
		});
	}

	function move(match: EventMatch, courtId: string) {
		void act('matches', async () => {
			await assignMatch(id, match.id, courtId === '' ? null : courtId);
			await refresh();
		});
	}

	async function saveResult(match: EventMatch, score: TypedScore, clientId: string) {
		// Throws to the caller, which explains a refusal in place.
		await postEventResult(id, match.id, { client_id: clientId, score });
		await refresh().catch(() => {
			// The stream brings the change too.
		});
	}

	// ---- start and finish ------------------------------------------------------------------

	function start() {
		void act('lifecycle', async () => {
			detail = await startEvent(id);
		});
	}

	/** With the places, or with none when the kind computes them (see KindFinish). */
	function finish(places?: Place[]) {
		void act('lifecycle', async () => {
			detail = await finishEvent(id, places);
		});
	}
</script>

<main class="acct page-wide">
	<a class="back" href="/events">{t.allEvents}</a>
	{#if loadFailure}
		<p class="error" role="alert">{loadFailure}</p>
	{:else if $session.status === 'signed_in' && detail && ev}
		<DeskHeader
			{detail}
			{busy}
			{copied}
			{copyFailed}
			oncopy={() => copy('spectator', eventLink(origin, detail?.spectator_token ?? ''))}
			onstart={start}
			onfinish={() => select('finish')}
		/>
		{#if failures.lifecycle && active !== 'finish'}<p class="error" role="alert">{failures.lifecycle}</p>{/if}

		{#if streamStatus === 'offline' || streamStatus === 'gone'}
			<div class="lost" role="status">
				<span>{t.streamLost}</span>
				<button class="secondary" type="button" onclick={() => location.reload()}>{t.reload}</button>
			</div>
		{/if}

		<DeskTabs {tabs} {active} kind={ev.kind} onselect={select} />

		<div id="desk-panel" role="tabpanel" aria-labelledby="tab-{active}">
			{#if active === 'teams'}
				<TeamsTab
					{detail}
					{players}
					{playersFailed}
					{busy}
					failure={failures.teams}
					onfail={(message) => fail('teams', message)}
					onadd={newTeam}
					onrename={renameTeam}
					onremove={removeTeam}
					onplayer={added}
				/>
			{:else if active === 'courts'}
				<CourtsTab
					{detail}
					{matches}
					{origin}
					{busy}
					failure={failures.courts}
					{copied}
					{copyFailed}
					oncopy={copy}
					onrotate={rotate}
					onnext={nextMatch}
					onaddcourt={newCourt}
				/>
			{:else if active === 'matches'}
				<MatchesTab
					{detail}
					{matches}
					{busy}
					failure={failures.matches}
					onfail={(message) => fail('matches', message)}
					oncreate={newMatch}
					onmove={move}
					onsave={saveResult}
				/>
			{:else if active === 'finish'}
				<FinishTab {detail} {matches} {busy} failure={failures.lifecycle} onopen={() => fail('lifecycle', null)} onfinish={finish} />
			{:else if active === 'kind'}
				<KindPanel event={detail} audience="coordinator" parts={ev.kind === 'non_stop' ? 'tables' : 'all'} />
			{:else}
				<LiveTab {detail} {matches} {busy} failure={failures.courts} onnext={nextMatch} onsave={saveResult} />
			{/if}
		</div>
	{:else}
		<p class="hint">{t.loading}</p>
	{/if}
</main>

<style>
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
	.lost {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		margin-top: 12px;
		padding: 8px 12px;
		border: 1px solid var(--warn);
		border-radius: 12px;
		color: var(--warn);
	}
	p.error {
		margin-top: 12px;
	}
</style>
