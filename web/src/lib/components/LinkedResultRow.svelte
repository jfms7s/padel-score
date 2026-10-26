<script lang="ts">
	// One match of a combined history. The full form names both teams and links to the final
	// view; the redacted form (another coordinator's match) shows only the date, outcome and score.
	import { shortDate } from '#lib/auth.ts';
	import { outcomeText } from '#lib/linking.ts';
	import { scoreText } from '#lib/players.ts';
	import { strings } from '#lib/strings.ts';
	import type { LinkedEntry, Team } from '#lib/types.ts';

	let { entry }: { entry: LinkedEntry } = $props();

	const t = strings.history;
	const TEAMS: Team[] = ['a', 'b'];
	/** The winning side from the player's: their own side on a win, the other on a loss. */
	const winner = $derived(
		entry.outcome === 'draw' ? null : entry.outcome === 'win' ? entry.side : entry.side === 'a' ? 'b' : 'a'
	);
</script>

<li class="result">
	<div class="top">
		<time datetime={new Date(entry.finished_at * 1000).toISOString()}>{shortDate(entry.finished_at)}</time>
		<span class="outcome {entry.outcome}">{outcomeText(entry)}</span>
	</div>
	{#if entry.event}
		<!-- A redacted event match keeps only its kind. -->
		<p class="event">
			{entry.redacted ? strings.events.kinds[entry.event.kind] : t.eventMatch(entry.event.name, entry.event.label)}
		</p>
	{/if}
	{#if entry.redacted}
		<p class="who hint">{strings.linking.anotherCoordinators}</p>
	{:else}
		<div class="teams">
			{#each TEAMS as team (team)}
				<p class="team" class:won={winner === team}>
					{strings.teamName(...(entry.teams[team] as [string, string]))}
					{#if winner === team}<span class="visually-hidden">, {t.winner}</span>{/if}
				</p>
			{/each}
		</div>
		<p class="hint who">{strings.linking.byCoordinator(entry.coordinator_name)}</p>
	{/if}
	<div class="bottom">
		<span class="score">{scoreText(entry.score)}</span>
		{#if !entry.redacted}
			<a
				class="final"
				href="/v/{encodeURIComponent(entry.spectator_token)}"
				aria-label={t.finalViewLabel(entry.teams.a.join(' / '), entry.teams.b.join(' / '))}>{t.finalView}</a
			>
		{/if}
	</div>
</li>

<style>
	.event {
		margin: 0;
		font-size: 14px;
		font-weight: 650;
		color: var(--muted);
		overflow-wrap: anywhere;
	}
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
	.who,
	.team {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.team.won {
		font-weight: 750;
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
