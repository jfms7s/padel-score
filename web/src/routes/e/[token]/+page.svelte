<script lang="ts">
	// The event for spectators: each court with its live match, the matches with their results,
	// the final positions once finished, and the kind's own panel. Read only, from the event's
	// viewing link; it never holds a scoring token. A signed-in viewer also gets "That's me" on
	// the teams' players, to claim their entry.
	import { page } from '$app/state';
	import CourtTile from '#lib/components/events/CourtTile.svelte';
	import KindPanel from '#lib/components/events/KindPanel.svelte';
	import { matchResultText } from '#lib/court.ts';
	import { eventViewStreamUrl, getEventView, isEventDetail, matchText, matchViewPath, rulesText, teamText } from '#lib/events.ts';
	import { ApiError } from '#lib/api.ts';
	import { getEventSlots } from '#lib/linking.ts';
	import { session } from '#lib/session.ts';
	import ClaimSheet from '#lib/components/ClaimSheet.svelte';
	import { openSseStream, type StreamStatus } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventSlotState, EventView, EventViewMatch } from '#lib/types.ts';

	const t = strings.court;
	const te = strings.events;
	const token = $derived(page.params.token ?? '');

	let detail = $state.raw<EventView | null>(null);
	let loadFailure = $state<string | null>(null);
	let streamStatus = $state<StreamStatus | null>(null);

	const ev = $derived(detail?.event ?? null);
	const teams = $derived(detail?.teams ?? []);
	const courts = $derived(detail?.courts ?? []);
	const matches = $derived([...(detail?.matches ?? [])].sort((x, y) => x.ordinal - y.ordinal));
	const places = $derived([...(detail?.places ?? [])].sort((x, y) => x.place - y.place));

	const matchById = (mid: string | null): EventViewMatch | undefined =>
		mid === null ? undefined : matches.find((m) => m.id === mid);
	const courtName = (cid: string | null) => courts.find((c) => c.id === cid)?.name ?? null;
	const teamName = (tid: string) => {
		const tm = teams.find((x) => x.id === tid);
		return tm ? teamText(tm) : te.unknownTeam;
	};

	$effect(() => {
		const tk = token;
		detail = null;
		loadFailure = null;
		streamStatus = null;
		let live = true;
		getEventView(tk).then(
			(d) => {
				if (live) detail = d;
			},
			(e: unknown) => {
				if (!live) return;
				loadFailure = e instanceof ApiError && e.status === 404 ? t.eventGone : t.loadFailed;
			}
		);
		const s = openSseStream(
			eventViewStreamUrl(tk),
			// The view has the detail's shape, minus the scoring tokens and phone counts.
			{ type: 'event', accept: (v: unknown): v is EventView => isEventDetail(v) },
			(d) => {
				if (live) detail = d;
			},
			(st) => {
				if (live) streamStatus = st;
			}
		);
		return () => {
			live = false;
			s.close();
		};
	});

	// "That's me", only when a session is known. Signed out, or on any failure, there are no
	// slots and the page is unchanged.
	let slots = $state.raw<EventSlotState[]>([]);
	let claiming = $state<EventSlotState | null>(null);
	const signedIn = $derived($session.status === 'signed_in');
	const claimable = $derived(slots.filter((x) => x.state !== 'linked' && x.state !== 'declined'));

	async function loadSlots(tk: string, live: () => boolean) {
		try {
			const list = await getEventSlots(tk);
			if (live()) slots = list;
		} catch {
			if (live()) slots = [];
		}
	}

	$effect(() => {
		const tk = token;
		slots = [];
		claiming = null;
		if (!signedIn) return;
		let live = true;
		void loadSlots(tk, () => live);
		return () => {
			live = false;
		};
	});
</script>

<svelte:head>
	<title>{ev ? ev.name : strings.appName}</title>
</svelte:head>

<main class="acct">
	{#if loadFailure && !detail}
		<p class="error" role="alert">{loadFailure}</p>
	{:else if detail && ev}
		<header class="head">
			<h1>{ev.name}</h1>
			<p class="meta">
				<span>{te.kinds[ev.kind]}</span>
				<span class="badge" class:off={ev.status === 'finished'}>{te.status[ev.status]}</span>
			</p>
			<p class="hint">{rulesText(ev.format)}</p>
		</header>

		{#if streamStatus === 'offline' || streamStatus === 'gone'}
			<div class="lost" role="status">
				<span>{te.streamLost}</span>
				<button class="secondary" type="button" onclick={() => location.reload()}>{te.reload}</button>
			</div>
		{/if}

		{#if ev.status === 'finished' && places.length > 0}
			<section aria-labelledby="h-places">
				<h2 id="h-places">{t.finalPositions}</h2>
				<ol class="list final">
					{#each places as p (p.team_id)}
						<li class="row">
							<span class="pos">{te.place(p.place, p.place_to)}</span>
							<span class="what"><strong>{teamName(p.team_id)}</strong></span>
						</li>
					{/each}
				</ol>
			</section>
		{/if}

		<!-- Once finished the courts are empty: only the positions and the results matter. -->
		{#if courts.length > 0 && ev.status !== 'finished'}
			<section aria-labelledby="h-courts">
				<h2 id="h-courts">{t.courts}</h2>
				<!-- Temporary: Task 6 places the tiles for good. -->
				<div class="tiles">
					{#each courts as court (court.id)}
						<CourtTile {court} {matches} {teams} />
					{/each}
				</div>
				<ul class="list" aria-labelledby="h-courts">
					{#each courts as court (court.id)}
						{@const m = matchById(court.current)}
						<li class="row">
							<span class="what">
								<strong>{court.name}</strong>
								{#if m}
									<span>{m.label}: {matchText(m, teams)}</span>
									{@const line = matchResultText(m, teams)}
									{#if line}<span class="score">{line}</span>{/if}
								{:else}
									<span class="hint">{t.courtIdle}</span>
								{/if}
							</span>
							<div class="actions">
								<a class="secondary" href="/cv/{encodeURIComponent(court.spectator_token)}" aria-label={t.watchLabel(court.name)}>{t.watch}</a>
							</div>
						</li>
					{/each}
				</ul>
			</section>
		{/if}

		<section aria-labelledby="h-matches">
			<h2 id="h-matches">{t.matches}</h2>
			{#if matches.length === 0}
				<p class="hint">{t.noMatches}</p>
			{:else}
				<ul class="list" aria-labelledby="h-matches">
					{#each matches as m (m.id)}
						{@const text = matchText(m, teams)}
						{@const line = matchResultText(m, teams)}
						{@const where = courtName(m.court_id)}
						<li class="row">
							<span class="what">
								<strong>{m.label}</strong>
								<span>{text}</span>
								<span class="hint">{te.matchState[m.state]}{#if where}{' · '}{where}{/if}{#if line}{' · '}{line}{/if}</span>
							</span>
							<div class="actions">
								<a class="secondary" href={matchViewPath(m.spectator_token)} aria-label={te.viewLabel(text)}>{te.view}</a>
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<KindPanel event={detail} audience="spectator" />

		{#if claimable.length > 0}
			<section aria-labelledby="h-claims">
				<h2 id="h-claims">{strings.linking.thatsMe}</h2>
				<ul class="list" aria-labelledby="h-claims">
					{#each claimable as slot (`${slot.team_id}${slot.position}`)}
						<li class="row">
							<span class="what">
								<strong>{slot.name}</strong>
								<span class="hint">{teamName(slot.team_id)}</span>
							</span>
							<div class="actions">
								{#if slot.state === 'claimable'}
									<button
										class="secondary"
										type="button"
										aria-label={strings.linking.thatsMeLabel(slot.name)}
										onclick={() => (claiming = slot)}>{strings.linking.thatsMe}</button
									>
								{:else if slot.state === 'pending'}
									<span class="hint">{strings.linking.claimSentShort}</span>
								{:else if slot.state === 'yours'}
									<span class="hint">{strings.linking.you}</span>
								{/if}
							</div>
						</li>
					{/each}
				</ul>
			</section>
		{/if}
	{:else}
		<p class="hint">{t.eventLoading}</p>
	{/if}
</main>

{#if claiming}
	<ClaimSheet
		target={{ kind: 'event_team_slot', event_view_token: token, team_id: claiming.team_id, position: claiming.position }}
		onclose={() => {
			claiming = null;
			void loadSlots(token, () => true);
		}}
	/>
{/if}

<style>
	.tiles {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(min(100%, 22rem), 1fr));
		gap: 14px;
	}
	.head h1 {
		margin-bottom: 4px;
		overflow-wrap: anywhere;
	}
	.meta {
		display: flex;
		align-items: center;
		gap: 4px;
		margin: 0 0 4px;
	}
	.meta .badge {
		margin-left: 6px;
	}
	section {
		margin-top: 8px;
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
	.row .what {
		flex: 1 1 10rem;
	}
	.score {
		font-weight: 700;
		font-feature-settings: 'tnum';
	}
	.pos {
		flex: none;
		min-width: 4.5rem;
		font-weight: 750;
		font-feature-settings: 'tnum';
	}
	.final {
		margin: 0;
	}
</style>
