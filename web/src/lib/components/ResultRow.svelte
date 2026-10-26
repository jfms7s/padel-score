<script lang="ts">
	// One finished match in a list (history, a player's page): the date, the event and match
	// label for an event match, both teams with the winner in bold and picked players linked to
	// their page, the score, and the final view.
	import { shortDate } from '#lib/auth.ts';
	import { scoreText } from '#lib/players.ts';
	import { strings } from '#lib/strings.ts';
	import type { HistoryEntry, Outcome, Team } from '#lib/types.ts';

	let { entry, outcome }: { entry: HistoryEntry; outcome?: Outcome } = $props();

	const t = strings.history;
	const TEAMS: Team[] = ['a', 'b'];
	const names = (team: Team) => strings.teamName(...entry.teams[team]);
</script>

<li class="result">
	<div class="top">
		<time datetime={new Date(entry.finished_at * 1000).toISOString()}>{shortDate(entry.finished_at)}</time>
		{#if outcome}
			<span class="outcome {outcome}">{t.outcome[outcome]}</span>
		{:else if entry.winner === null}
			<span class="outcome draw">{t.draw}</span>
		{/if}
	</div>
	{#if entry.event}<p class="event">{t.eventMatch(entry.event.name, entry.event.label)}</p>{/if}
	<div class="teams">
		{#each TEAMS as team (team)}
			<p class="team" class:won={entry.winner === team}>
				{#each entry.teams[team] as name, i (i)}
					{@const id = entry.players[team][i]}
					{#if i > 0}<span class="sep">{' / '}</span>{/if}
					{#if id}
						<a href="/players/{encodeURIComponent(id)}">{name}</a>
					{:else}
						<span>{name}</span>
					{/if}
				{/each}
				{#if entry.winner === team}<span class="visually-hidden">, {t.winner}</span>{/if}
			</p>
		{/each}
	</div>
	<div class="bottom">
		<span class="score">{scoreText(entry.score)}</span>
		<a
			class="final"
			href="/v/{encodeURIComponent(entry.spectator_token)}"
			aria-label={t.finalViewLabel(names('a'), names('b'))}>{t.finalView}</a
		>
	</div>
</li>

<style>
	.result {
		display: flex;
		flex-direction: column;
		gap: 4px;
		min-width: 0;
		padding: 8px 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
	}
	.top,
	.bottom {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 4px 12px;
	}
	time {
		color: var(--muted);
		font-size: 13px;
	}
	.outcome {
		padding: 0 8px;
		border-radius: 999px;
		font-size: 13px;
		font-weight: 650;
		line-height: 24px;
	}
	.outcome.win {
		background: var(--b-soft);
		color: var(--ink);
	}
	.outcome.loss {
		border: 1px solid var(--line);
		color: var(--muted);
	}
	.outcome.draw {
		background: var(--a-soft);
		color: var(--ink);
	}
	.event {
		margin: 0;
		font-size: 14px;
		font-weight: 650;
		color: var(--muted);
		overflow-wrap: anywhere;
	}
	.teams {
		display: flex;
		flex-direction: column;
	}
	.team {
		margin: 0;
		overflow-wrap: anywhere;
		font-weight: 400;
	}
	.team.won {
		font-weight: 750;
	}
	.team a {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: inherit;
		text-decoration: underline;
		text-decoration-color: var(--line);
		text-underline-offset: 3px;
	}
	.team span:not(.visually-hidden, .sep) {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
	}
	.sep {
		color: var(--muted);
		white-space: pre;
	}
	.score {
		font-size: 17px;
		font-weight: 700;
		font-variant-numeric: tabular-nums;
	}
	.final {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
</style>
