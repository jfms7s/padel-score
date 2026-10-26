<script lang="ts">
	// The event for spectators: each court with its live match, the matches with their results,
	// the final positions once finished, and the kind's own panel. Read only, from the event's
	// viewing link; it never holds a scoring token. A signed-in viewer also gets "That's me" on
	// the teams' players, to claim their entry.
	import { page } from '$app/state';
	import { MediaQuery } from 'svelte/reactivity';
	import CourtGrid from '#lib/components/events/CourtGrid.svelte';
	import CourtTile from '#lib/components/events/CourtTile.svelte';
	import KindPanel from '#lib/components/events/KindPanel.svelte';
	import MatchList from '#lib/components/events/MatchList.svelte';
	import { eventViewStreamUrl, getEventView, isEventDetail, rulesText, teamText } from '#lib/events.ts';
	import { ApiError } from '#lib/api.ts';
	import { getEventSlots } from '#lib/linking.ts';
	import { session } from '#lib/session.ts';
	import ClaimSheet from '#lib/components/ClaimSheet.svelte';
	import { openSseStream, type StreamStatus } from '#lib/stream.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventSlotState, EventView } from '#lib/types.ts';

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

	// The tiles are compact below the desktop width, as on the desk's Live tab.
	const desk = new MediaQuery('(min-width: 1100px)');
	const compact = $derived(!desk.current);

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

<main class="acct page-wide">
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
				<CourtGrid count={courts.length}>
					{#each courts as court (court.id)}
						<CourtTile {court} {matches} {teams} {compact} />
					{/each}
				</CourtGrid>
			</section>
		{/if}

		<div class="split">
			<div class="kind">
				<KindPanel event={detail} audience="spectator" parts="all" />
			</div>
			<section class="panel" aria-labelledby="h-matches">
				<h2 id="h-matches">{t.matches}</h2>
				{#if matches.length === 0}
					<p class="hint empty">{t.noMatches}</p>
				{:else}
					<MatchList {matches} {courts} {teams} format={ev.format} mode="view" />
				{/if}
			</section>
		</div>

		{#if claimable.length > 0}
			<section class="page-narrow claims" aria-labelledby="h-claims">
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
	.split {
		display: grid;
		gap: 16px;
		align-items: start;
		margin-top: 16px;
	}
	.split > * {
		min-width: 0;
	}
	/* The kind's own sections bring their top margin; the column starts flush. */
	.kind :global(section:first-child),
	.kind :global(h2:first-child),
	.kind :global(h3:first-child) {
		margin-top: 0;
	}
	.panel {
		margin-top: 0;
		border: 1px solid var(--line);
		border-radius: 16px;
		background: var(--panel);
		overflow: hidden;
	}
	.panel h2 {
		margin: 0;
		padding: 12px 14px;
		font-size: 16px;
		border-bottom: 1px solid var(--line);
	}
	.empty {
		margin: 0;
		padding: 12px 14px;
	}
	/* Claims keep the phone column's width, flush left under the wide page. */
	.claims {
		margin-left: 0;
		padding-left: 0;
		padding-right: 0;
	}
	.row .what {
		flex: 1 1 10rem;
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
	@media (min-width: 1100px) {
		.split {
			grid-template-columns: minmax(0, 1fr) 380px;
		}
	}
</style>
