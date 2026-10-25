<script lang="ts">
	// The explicit choice after offline scoring met a server that moved meanwhile: both scores
	// side by side, and someone picks one. It can't be dismissed: nothing is merged silently.
	import { strings } from '#lib/strings.ts';
	import type { Conflict } from '#lib/queue.ts';
	import type { MatchView } from '#lib/types.ts';
	import { fenceModel, type FenceModel } from '#lib/view.ts';

	let {
		conflict,
		resolving = false,
		onkeepserver,
		onkeepmine
	}: {
		conflict: Conflict | null;
		resolving?: boolean;
		onkeepserver: () => void;
		onkeepmine: () => void;
	} = $props();

	const q = strings.queue;
	let dialog: HTMLDialogElement | undefined = $state();

	const model = (v: MatchView | null): FenceModel | null => (v ? fenceModel(v) : null);
	const server = $derived(model(conflict?.server ?? null));
	const mine = $derived(model(conflict?.mine ?? null));

	$effect(() => {
		if (!dialog) return;
		if (conflict && !dialog.open) dialog.showModal();
		if (!conflict && dialog.open) dialog.close();
	});
</script>

{#snippet score(title: string, m: FenceModel | null)}
	<section class="side" aria-label={title}>
		<h3>{title}</h3>
		{#if m}
			<table>
				<tbody>
					{#each [m.a, m.b] as t, i (i)}
						<tr>
							<th scope="row">{t.name}</th>
							{#each t.sets as g, s (s)}<td class="games">{g}</td>{/each}
							<td class="points">{t.points}</td>
						</tr>
					{/each}
				</tbody>
			</table>
		{:else}
			<p class="hint">{q.mineUnknown}</p>
		{/if}
	</section>
{/snippet}

<dialog
	bind:this={dialog}
	aria-labelledby="conflict-title"
	aria-describedby="conflict-hint"
	oncancel={(e) => e.preventDefault()}
>
	<div class="sheet">
		<h2 id="conflict-title">{q.conflictTitle}</h2>
		<p id="conflict-hint" class="hint">{q.conflictHint}</p>
		<div class="sides">
			{@render score(q.server, server)}
			{@render score(q.mine, mine)}
		</div>
		<div class="choices">
			<button type="button" class="choice" disabled={resolving} onclick={onkeepserver}>{q.keepServer}</button>
			<button type="button" class="choice primary" disabled={resolving || !mine} onclick={onkeepmine}>
				{resolving ? q.saving : q.keepMine}
			</button>
		</div>
	</div>
</dialog>

<style>
	dialog {
		width: min(100vw - 32px, 40rem);
		max-height: calc(100dvh - 32px);
		padding: 0;
		border: 1px solid var(--line);
		border-radius: 18px;
		background: var(--panel);
		color: var(--ink);
	}
	dialog::backdrop {
		background: rgba(5, 11, 18, 0.55);
	}
	.sheet {
		padding: 20px 18px;
		display: flex;
		flex-direction: column;
		gap: 14px;
	}
	h2 {
		margin: 0;
		font-size: 22px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	h3 {
		margin: 0 0 8px;
		font-size: 15px;
		font-weight: 650;
	}
	.hint {
		margin: 0;
		font-size: 14px;
		color: var(--muted);
	}
	.sides {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
		gap: 12px;
	}
	.side {
		border: 1px solid var(--line);
		border-radius: 16px;
		padding: 14px;
		background: var(--glass);
		min-width: 0;
	}
	table {
		width: 100%;
		border-collapse: collapse;
		font-feature-settings: 'tnum';
	}
	th {
		text-align: left;
		font-weight: 600;
		padding: 4px 8px 4px 0;
		overflow-wrap: anywhere;
	}
	td {
		text-align: center;
		padding: 4px 6px;
		min-width: 1.6em;
	}
	.points {
		font-weight: 800;
		font-size: 20px;
		border-left: 1px solid var(--line);
	}
	.choices {
		display: flex;
		flex-wrap: wrap;
		gap: 10px;
	}
	.choice {
		flex: 1 1 12rem;
		min-height: var(--tap);
		border-radius: 14px;
		padding: 0 16px;
		font-weight: 650;
		border: 1px solid var(--line);
		background: transparent;
		color: var(--ink);
	}
	.choice.primary {
		border: 0;
		background: var(--ink);
		color: var(--glass);
	}
	.choice:disabled {
		opacity: 0.6;
	}
</style>
