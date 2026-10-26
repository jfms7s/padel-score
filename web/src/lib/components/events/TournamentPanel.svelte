<!--
	Owned by phase 7 (the tournament). The tournament's part of the event screen, for the coordinator or
	a spectator: in setup the "Losers keep playing" switch and the team counts; once running, the live
	bracket, and for the coordinator the redraw, fill-courts and held-result controls.
-->
<script lang="ts">
	import { eventMessage, updateEvent } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import {
		bracketSections,
		canFillCourts,
		decideBracket,
		fillCourts,
		heldText,
		redrawBracket,
		teamCountHint,
		tournamentData
	} from '#lib/tournament.ts';
	import type { EventDetail, EventView } from '#lib/types.ts';
	import BracketMatch from './BracketMatch.svelte';

	let { event, audience }: { event: EventDetail | EventView; audience: 'coordinator' | 'spectator' } = $props();

	const t = strings.tournament;
	const ev = strings.events;

	const coordinator = $derived(audience === 'coordinator');
	const id = $derived(event.event.id);
	const status = $derived(event.event.status);
	const data = $derived(tournamentData(event.kind_data));
	const sections = $derived(data ? bracketSections(data) : []);
	const held = $derived(data ? data.nodes.filter((n) => n.held) : []);
	const fullPlacement = $derived(event.event.settings.full_placement === true);
	const hint = $derived(teamCountHint(event.teams.length));
	const fillable = $derived(data ? canFillCourts(data, event.matches, event.courts) : false);

	let busy = $state<string | null>(null);
	let confirming = $state(false);
	let failure = $state<string | null>(null);

	// The page follows the event stream, so a reply's detail is not needed: only a failure is.
	async function run(key: string, call: () => Promise<unknown>) {
		busy = key;
		failure = null;
		try {
			await call();
			confirming = false;
		} catch (e) {
			failure = eventMessage(e);
		} finally {
			busy = null;
		}
	}
</script>

{#if status === 'setup'}
	{#if coordinator}
		<section aria-labelledby="h-tournament">
			<h2 id="h-tournament">{t.settingsHeading}</h2>
			<div class="row">
				<span class="what"><strong>{t.fullPlacementState(fullPlacement)}</strong></span>
				<button
					class="secondary"
					type="button"
					disabled={busy !== null}
					onclick={() => run('settings', () => updateEvent(id, { settings: { full_placement: !fullPlacement } }))}
				>
					{busy === 'settings' ? t.saving : t.change}
				</button>
			</div>
			<p class="hint">{fullPlacement ? t.fullPlacementOn : t.fullPlacementOff}</p>
			<p class="hint">{hint ?? t.teamCounts}</p>
			{#if failure}<p class="error" role="alert">{failure}</p>{/if}
		</section>
	{/if}
{:else if data}
	<section aria-labelledby="h-bracket">
		<h2 id="h-bracket">{t.bracket}</h2>
		<p class="hint">{t.teamsAndCourts(data.teams, event.courts.length)}</p>

		{#if coordinator}
			{#each held as node (`${node.place_from}-${node.size}-${node.idx}`)}
				{@const text = heldText(data, node, status === 'finished', event.matches)}
				<div class="held" role="group" aria-label={node.label}>
					<p class="warn">{text.warning}</p>
					{#if text.finishedNote}<p class="hint">{text.finishedNote}</p>{/if}
					{#if node.match_id !== null}
						{@const matchId = node.match_id}
						<div class="decision">
							{#if text.replay}
								<div>
									<button
										class="secondary"
										type="button"
										disabled={busy !== null}
										onclick={() => run(`replay-${matchId}`, () => decideBracket(id, matchId, 'replay'))}
									>
										{t.replay}
									</button>
									<p class="hint">{text.replay}</p>
								</div>
							{/if}
							<div>
								<button
									class="secondary"
									type="button"
									disabled={busy !== null}
									onclick={() => run(`keep-${matchId}`, () => decideBracket(id, matchId, 'keep'))}
								>
									{t.keep}
								</button>
								<p class="hint">{text.keep}</p>
							</div>
						</div>
					{/if}
				</div>
			{/each}
		{/if}

		{#each sections as section, i (i)}
			<!-- svelte-ignore a11y_no_noninteractive_tabindex (a box that scrolls sideways must be reachable by keyboard) -->
			<div class="scroller" class:placement={section.kind === 'placement'} role="region" aria-label={section.title ?? t.bracket} tabindex="0">
				{#if section.title}<h3>{section.title}</h3>{/if}
				<div class="bracket" style:--cols={section.columns.length}>
					{#each section.columns as column (column.title)}
						<div class="round">
							<h4>{column.title}</h4>
							<div class="stack">
								{#each column.nodes as node (`${node.place_from}-${node.size}-${node.idx}`)}
									<BracketMatch {data} {node} {event} />
								{/each}
							</div>
						</div>
					{/each}
				</div>
			</div>
		{/each}

		{#if coordinator && status === 'running'}
			<div class="actions">
				{#if fillable}
					<button class="secondary" type="button" disabled={busy !== null} onclick={() => run('fill', () => fillCourts(id))}>
						{busy === 'fill' ? t.filling : t.fillCourts}
					</button>
				{/if}
				{#if data.can_redraw}
					{#if confirming}
						<div class="confirm" role="group" aria-label={t.redraw}>
							<p>{t.redrawConfirm}</p>
							<div class="actions">
								<button class="danger" type="button" disabled={busy !== null} onclick={() => run('redraw', () => redrawBracket(id))}>
									{busy === 'redraw' ? t.redrawing : ev.confirm}
								</button>
								<button class="secondary" type="button" onclick={() => (confirming = false)}>{ev.cancel}</button>
							</div>
						</div>
					{:else}
						<button class="secondary" type="button" disabled={busy !== null} onclick={() => (confirming = true)}>{t.redraw}</button>
					{/if}
				{/if}
			</div>
			{#if data.can_redraw && !confirming}<p class="hint">{t.redrawHint}</p>{/if}
		{/if}
		{#if failure}<p class="error" role="alert">{failure}</p>{/if}
	</section>
{/if}

<style>
	section {
		margin-top: 8px;
	}
	h3 {
		margin: 0 0 10px;
		font-size: 16px;
		font-weight: 700;
	}
	.scroller {
		margin-top: 12px;
		padding: 14px;
		overflow-x: auto;
		border: 1px solid var(--line);
		border-radius: 18px;
		background: var(--panel);
	}
	.scroller.placement {
		border-top-style: dashed;
	}
	.bracket {
		display: grid;
		grid-template-columns: repeat(var(--cols), minmax(190px, 1fr));
		gap: 0 28px;
		min-width: calc(var(--cols) * 190px + (var(--cols) - 1) * 28px);
	}
	.round {
		display: flex;
		flex-direction: column;
	}
	.round h4 {
		margin: 0 0 10px;
		font-size: 13px;
		font-weight: 600;
		color: var(--muted);
	}
	.stack {
		display: flex;
		flex-direction: column;
		justify-content: space-around;
		flex: 1;
		gap: 14px;
	}
	.held {
		margin-top: 12px;
		padding: 12px;
		border: 1px solid var(--warn);
		border-radius: 12px;
		background: var(--panel);
	}
	.held p {
		margin: 0 0 8px;
	}
	.warn {
		color: var(--warn);
		font-weight: 650;
	}
	.decision {
		display: grid;
		gap: 12px;
	}
	.decision .hint {
		margin: 4px 0 0;
	}
	.actions {
		margin-top: 12px;
	}
	.confirm {
		width: 100%;
	}
	.confirm p {
		margin: 0 0 8px;
		font-size: 14px;
	}
	p.error {
		margin-top: 12px;
	}
</style>
