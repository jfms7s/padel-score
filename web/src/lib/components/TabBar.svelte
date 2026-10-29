<script lang="ts">
	import { ICONS } from '#lib/navIcons.ts';
	import { navItems, type NavItem } from '#lib/nav.ts';
	import { strings } from '#lib/strings.ts';

	let { pathname, onnew }: { pathname: string; onnew?: () => void } = $props();

	const items = $derived(navItems(pathname));
</script>

{#snippet face(item: NavItem)}
				<svg
				viewBox="0 0 24 24"
				width="24"
				height="24"
				fill="none"
				stroke="currentColor"
				stroke-width="1.8"
				stroke-linecap="round"
				stroke-linejoin="round"
				aria-hidden="true"
			>
				<path d={ICONS[item.key]} />
			</svg>
			<span class:sr-only={item.key === 'new'}>{item.label}</span>
{/snippet}

<nav aria-label={strings.nav.label} class="tabbar">
	{#each items as item (item.key)}
		{#if item.key === 'new' && onnew}
			<button type="button" class="plus" onclick={onnew}>
				{@render face(item)}
			</button>
		{:else}
			<a
				href={item.href}
				class:current={item.current}
				class:plus={item.key === 'new'}
				aria-current={item.current ? 'page' : undefined}
			>
				{@render face(item)}
			</a>
		{/if}
	{/each}
</nav>

<style>
	.tabbar {
		position: fixed;
		inset: auto 0 0 0;
		z-index: 10;
		height: calc(60px + env(safe-area-inset-bottom));
		padding-bottom: env(safe-area-inset-bottom);
		display: grid;
		grid-template-columns: repeat(5, 1fr);
		align-items: center;
		background: var(--glass);
		border-top: 1px solid var(--line);
	}
	a,
	button {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 2px;
		min-height: var(--tap);
		padding: 0;
		border: 0;
		background: none;
		font: inherit;
		color: var(--muted);
		text-decoration: none;
	}
	a span,
	button span {
		font-size: 11px;
		font-weight: 650;
		line-height: 1.1;
	}
	a.current {
		color: var(--ink);
	}
	a.current span {
		font-weight: 750;
	}
	.plus {
		justify-self: center;
		width: 48px;
		height: 48px;
		border-radius: 50%;
		background: var(--ink);
		color: var(--glass);
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip-path: inset(50%);
		white-space: nowrap;
	}
	@media (min-width: 700px) {
		.tabbar {
			display: none;
		}
	}
</style>
