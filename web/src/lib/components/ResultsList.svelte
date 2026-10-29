<script lang="ts">
	// Finished matches as rows: cards on a phone, a table from 700px. The winning side's names take
	// that side's colour; a row with a link opens the final view.
	import type { ResultRowData } from '#lib/results.ts';
	import { strings } from '#lib/strings.ts';

	let { rows }: { rows: ResultRowData[] } = $props();

	const t = strings.results;
	const CHIPS = { won: t.won, lost: t.lost, draw: t.draw, recorded: t.recordedChip };
</script>

{#snippet chip(row: ResultRowData)}
	<span class="chip {row.chip}">{CHIPS[row.chip]}</span>
{/snippet}

{#snippet open(row: ResultRowData, label: string)}
	{#if row.href}
		<a class="open" href={row.href}>{label}</a>
	{:else}
		{label}
	{/if}
{/snippet}

<ul class="cards" aria-label={t.title}>
	{#each rows as row (row.id)}
		<li class="card">
			<div class="teams">
				{#if row.event}<span class="event">{row.event}</span>{/if}
				<span class="line a" class:win={row.winner === 'a'}>{row.a}</span>
				<span class="line b" class:win={row.winner === 'b'}>{row.b}</span>
				{#if row.href}<a class="cover" href={row.href} aria-label="{row.a} v {row.b}, {row.score}"></a>{/if}
			</div>
			<div class="side">
				<span class="num score">{row.score}</span>
				<time>{row.date}</time>
				{@render chip(row)}
			</div>
		</li>
	{/each}
</ul>

<div class="scroll">
	<table class="grid" aria-label={t.title}>
		<thead>
			<tr>
				<th>{t.columns.date}</th>
				<th>{t.columns.teamA}</th>
				<th>{t.columns.teamB}</th>
				<th class="right">{t.columns.score}</th>
				<th><span class="visually-hidden">{t.columns.result}</span></th>
			</tr>
		</thead>
		<tbody>
			{#each rows as row (row.id)}
				<tr>
					<td class="date">{row.date}</td>
					<td class="line a" class:win={row.winner === 'a'}>{@render open(row, row.a)}</td>
					<td class="line b" class:win={row.winner === 'b'}>{@render open(row, row.b)}</td>
					<td class="right"><span class="num score">{row.score}</span></td>
					<td>{@render chip(row)}</td>
				</tr>
			{/each}
		</tbody>
	</table>
</div>

<style>
	.cards {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	.card {
		position: relative;
		display: flex;
		justify-content: space-between;
		gap: 12px;
		min-height: var(--tap);
		padding: 10px 14px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
	}
	.teams {
		display: flex;
		flex-direction: column;
		min-width: 0;
	}
	.event {
		color: var(--muted);
		font-size: 13px;
		overflow-wrap: anywhere;
	}
	.line {
		overflow-wrap: anywhere;
	}
	.line.win {
		font-weight: 750;
	}
	.line.win.a {
		color: var(--a);
	}
	.line.win.b {
		color: var(--b);
	}
	.cover {
		position: absolute;
		inset: 0;
		border-radius: 12px;
	}
	.cover:focus-visible,
	.open:focus-visible {
		outline: 2px solid var(--ink);
		outline-offset: 2px;
	}
	.side {
		flex: none;
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: 4px;
	}
	.score {
		font-size: 24px;
		line-height: 1;
		white-space: nowrap;
		/* The condensed numerals' space is too narrow to separate sets. */
		word-spacing: 0.35em;
	}
	time {
		color: var(--muted);
		font-size: 13px;
	}
	.chip {
		padding: 0 8px;
		border: 1px solid var(--line);
		border-radius: 999px;
		color: var(--muted);
		font-size: 13px;
		font-weight: 650;
		line-height: 22px;
	}
	.chip.won {
		border-color: transparent;
		background: var(--b-soft);
		color: var(--ink);
	}
	.chip.draw {
		border-color: transparent;
		background: var(--a-soft);
		color: var(--ink);
	}
	.scroll {
		display: none;
	}
	.open {
		color: inherit;
		text-decoration: none;
	}
	.open:hover {
		text-decoration: underline;
	}
	@media (min-width: 700px) {
		.cards {
			display: none;
		}
		.scroll {
			display: block;
			overflow-x: auto;
		}
		.grid {
			width: 100%;
			border-collapse: collapse;
		}
		th,
		td {
			padding: 10px 12px;
			border-bottom: 1px solid var(--line);
			text-align: left;
			vertical-align: middle;
		}
		th {
			color: var(--muted);
			font-size: 13px;
			font-weight: 650;
		}
		.right {
			text-align: right;
		}
		td {
			overflow-wrap: anywhere;
		}
		.date {
			color: var(--muted);
			white-space: nowrap;
		}
	}
</style>
