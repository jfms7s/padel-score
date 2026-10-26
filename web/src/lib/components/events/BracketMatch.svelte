<!--
	Owned by phase 7 (the tournament). One match of the bracket: two rows (team and games), and under
	them where it plays. A finished match links to its match view.
-->
<script lang="ts">
	import { matchViewPath } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import { sideScore, slotText, whereText, type BracketNode, type TournamentData } from '#lib/tournament.ts';
	import type { EventDetail, EventView } from '#lib/types.ts';

	let { data, node, event }: { data: TournamentData; node: BracketNode; event: EventDetail | EventView } = $props();

	const t = strings.tournament;
	const match = $derived(node.match_id === null ? undefined : event.matches.find((m) => m.id === node.match_id));
	const where = $derived(whereText(node, event.matches, event.courts));
	const sides = $derived(
		([0, 1] as const).map((i) => {
			const slot = slotText(data, node, i, event.teams);
			const side = i === 0 ? 'a' : 'b';
			const state = !match || match.winner === null ? '' : match.winner === side ? 'win' : 'lose';
			return { ...slot, state, score: match ? sideScore(match, side) : '' };
		})
	);
	const href = $derived(match?.state === 'finished' ? matchViewPath(match.spectator_token) : null);
</script>

<svelte:element
	this={href ? 'a' : 'div'}
	class="match"
	class:held={node.held}
	href={href ?? undefined}
>
	{#each sides as s, i (i)}
		<div class="t {s.state}" class:tbd={!s.known}>
			<span class="who">{s.text}</span><span class="s">{s.score}</span>
		</div>
	{/each}
	{#if href}<span class="visually-hidden">{t.matchLink(node.label)}</span>{/if}
	{#if node.held}
		<div class="where held">{t.held}</div>
	{:else if where}
		<div class="where" class:wait={!where.live}>
			{#if where.live}<span class="live-dot" aria-hidden="true"></span>{/if}{where.text}
		</div>
	{/if}
</svelte:element>

<style>
	.match {
		display: block;
		border: 1px solid var(--line);
		border-radius: 12px;
		overflow: hidden;
		background: var(--glass);
		font-size: 14px;
		color: var(--ink);
		text-decoration: none;
	}
	a.match:hover {
		border-color: var(--a);
	}
	.match.held {
		border-color: var(--warn);
	}
	.t {
		display: flex;
		justify-content: space-between;
		padding: 7px 10px;
		gap: 8px;
	}
	.t + .t {
		border-top: 1px solid var(--line);
	}
	.who {
		overflow-wrap: anywhere;
	}
	.t.win {
		font-weight: 700;
	}
	.t.lose {
		color: var(--muted);
	}
	.t.tbd {
		color: var(--muted);
		font-style: italic;
	}
	.s {
		font-variation-settings: 'wdth' 70;
		font-feature-settings: 'tnum';
		letter-spacing: 0.02em;
		white-space: nowrap;
	}
	.where {
		font-size: 12px;
		padding: 5px 10px;
		background: var(--a-soft);
		color: var(--ink);
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.where.wait {
		background: transparent;
		color: var(--muted);
		border-top: 1px solid var(--line);
	}
	.where.held {
		background: transparent;
		color: var(--warn);
		font-weight: 650;
		border-top: 1px solid var(--warn);
	}
	.live-dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--warn);
		animation: pulse 1.6s infinite;
	}
	@keyframes pulse {
		50% {
			opacity: 0.25;
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.live-dot {
			animation: none;
		}
	}
</style>
