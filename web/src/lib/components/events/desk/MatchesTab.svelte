<!--
	The desk's Matches tab: the grouped match list with the court select, typed results and View,
	and the New match form while the event runs (beside the list on a desktop).
-->
<script lang="ts">
	import MatchList from '#lib/components/events/MatchList.svelte';
	import { teamText } from '#lib/events.ts';
	import { MAX_NAME_LENGTH } from '#lib/setup.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, EventMatch, NewEventMatch, TypedScore } from '#lib/types.ts';

	let {
		detail,
		matches,
		busy,
		failure,
		onfail,
		oncreate,
		onmove,
		onsave
	}: {
		detail: EventDetail;
		/** The matches in their order. */
		matches: EventMatch[];
		busy: boolean;
		failure: string | undefined;
		onfail: (message: string) => void;
		oncreate: (body: NewEventMatch) => Promise<boolean>;
		onmove: (m: EventMatch, courtId: string) => void;
		/** Throws to refuse; the list explains it in place. */
		onsave: (m: EventMatch, score: TypedScore, clientId: string) => Promise<void>;
	} = $props();

	const t = strings.events;

	const ev = $derived(detail.event);
	const status = $derived(ev.status);
	const teams = $derived(detail.teams);
	const courts = $derived(detail.courts);

	let teamA = $state('');
	let teamB = $state('');
	let matchCourt = $state('');
	let matchLabel = $state('');
	const running = $derived(status === 'running');

	async function submitMatch(event: SubmitEvent) {
		event.preventDefault();
		if (teamA === '' || teamB === '') {
			onfail(t.pickTeam);
			return;
		}
		if (teamA === teamB) {
			onfail(t.sameTeam);
			return;
		}
		const label = matchLabel.trim();
		const made = await oncreate({
			team_a: teamA,
			team_b: teamB,
			...(matchCourt === '' ? {} : { court_id: matchCourt }),
			...(label === '' ? {} : { label })
		});
		if (made) {
			teamA = '';
			teamB = '';
			matchCourt = '';
			matchLabel = '';
		}
	}
</script>

<section aria-labelledby="h-matches">
	<h2 id="h-matches">{t.matches}</h2>
	<div class="layout" class:with-form={running}>
		<div class="list-col">
			{#if matches.length === 0}
				<p class="hint">{t.noMatches}</p>
			{:else}
				<div class="panel">
					<MatchList
						{matches}
						{courts}
						{teams}
						format={ev.format}
						mode="manage"
						onsave={running ? onsave : undefined}
						onmove={running ? onmove : undefined}
						{busy}
					/>
				</div>
			{/if}
		</div>
		{#if running}
			<div class="form-col">
				<h3>{t.newMatch}</h3>
				<form class="new-match" onsubmit={submitMatch}>
					<label>
						{t.teamA}
						<select bind:value={teamA} required>
							<option value="" disabled>{t.pickTeam}</option>
							{#each teams as tm (tm.id)}<option value={tm.id}>{teamText(tm)}</option>{/each}
						</select>
					</label>
					<label>
						{t.teamB}
						<select bind:value={teamB} required>
							<option value="" disabled>{t.pickTeam}</option>
							{#each teams as tm (tm.id)}<option value={tm.id}>{teamText(tm)}</option>{/each}
						</select>
					</label>
					<label>
						{t.court}
						<select bind:value={matchCourt}>
							<option value="">{t.notOnCourt}</option>
							{#each courts as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
						</select>
					</label>
					<label>
						{t.label}
						<input bind:value={matchLabel} autocomplete="off" maxlength={MAX_NAME_LENGTH} />
					</label>
					<button class="primary" type="submit" disabled={busy}>{t.create_match}</button>
				</form>
			</div>
		{/if}
	</div>
	{#if failure}<p class="error" role="alert">{failure}</p>{/if}
</section>

<style>
	section {
		margin-top: 8px;
	}
	h3 {
		margin: 20px 0 8px;
		font-size: 16px;
		font-weight: 700;
	}
	.panel {
		border: 1px solid var(--line);
		border-radius: 16px;
		background: var(--panel);
		overflow: hidden;
	}
	.layout {
		display: grid;
		gap: 8px 24px;
		align-items: start;
	}
	.layout > * {
		min-width: 0;
	}
	.new-match {
		max-width: 28rem;
	}
	select {
		min-height: var(--tap);
		width: 100%;
		padding: 0 10px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		color: var(--ink);
		font-size: 16px;
	}
	@media (min-width: 1100px) {
		.layout.with-form {
			grid-template-columns: minmax(0, 1fr) 22rem;
		}
		.form-col h3 {
			margin-top: 0;
		}
	}
	p.error {
		margin-top: 12px;
	}
</style>
