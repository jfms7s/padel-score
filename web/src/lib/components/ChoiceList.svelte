<script lang="ts">
	// The ways to start something, as cards: on the New page and in the New sheet.
	import { choices, type ChoiceKey } from '#lib/create.ts';

	let { onpick }: { onpick?: () => void } = $props();

	const BADGE: Record<ChoiceKey, string> = {
		match: 'var(--ink)',
		later: 'var(--b)',
		board: 'var(--a)',
		event: 'var(--ev)'
	};
</script>

<div class="list">
	{#each choices() as choice (choice.key)}
		<a class="opt" href={choice.href} onclick={() => onpick?.()}>
			<span class="badge" style:background={BADGE[choice.key]} aria-hidden="true"></span>
			<span class="text">
				<span class="title">{choice.title}</span>
				<span class="line">{choice.line}</span>
			</span>
		</a>
	{/each}
</div>

<style>
	.list {
		display: flex;
		flex-direction: column;
		gap: 10px;
	}
	.opt {
		display: flex;
		align-items: center;
		gap: 14px;
		min-height: var(--tap);
		padding: 12px 14px;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
		color: var(--ink);
		text-decoration: none;
	}
	.badge {
		flex: none;
		width: 38px;
		height: 38px;
		border-radius: 12px;
	}
	.text {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
	}
	.title {
		font-weight: 750;
	}
	.line {
		color: var(--muted);
		font-size: 14px;
	}
</style>
