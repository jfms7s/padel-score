<!--
	The desk's tab bar: one link per tab, so a tab has its own URL. A click swaps the tab in
	place; ←/→ move along the bar. On a phone the bar scrolls sideways on its own.
-->
<script lang="ts">
	import type { DeskTab } from '#lib/desk.ts';
	import { strings } from '#lib/strings.ts';
	import type { EventKind } from '#lib/types.ts';

	let {
		tabs,
		active,
		kind,
		onselect
	}: {
		tabs: DeskTab[];
		active: DeskTab;
		kind: EventKind;
		onselect: (tab: DeskTab) => void;
	} = $props();

	const t = strings.desk.tabs;
	const links: Partial<Record<DeskTab, HTMLAnchorElement | null>> = {};

	const label = (tab: DeskTab) => (tab === 'kind' ? t.kind[kind] : t[tab]);

	function click(e: MouseEvent, tab: DeskTab) {
		// A new browser tab or window keeps the link's own behaviour.
		if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
		e.preventDefault();
		onselect(tab);
	}

	function key(e: KeyboardEvent, tab: DeskTab) {
		if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
		e.preventDefault();
		const at = tabs.indexOf(tab);
		const to = tabs[(at + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
		links[to]?.focus();
	}
</script>

<div class="tabs" role="tablist" aria-label={strings.desk.tabsLabel}>
	{#each tabs as tab (tab)}
		<a
			id="tab-{tab}"
			role="tab"
			href="?tab={tab}"
			bind:this={links[tab]}
			aria-selected={tab === active}
			aria-current={tab === active ? 'page' : undefined}
			aria-controls="desk-panel"
			tabindex={tab === active ? 0 : -1}
			onclick={(e) => click(e, tab)}
			onkeydown={(e) => key(e, tab)}
		>
			{label(tab)}
		</a>
	{/each}
</div>

<style>
	.tabs {
		display: flex;
		gap: 4px;
		margin: 12px 0 4px;
		overflow-x: auto;
		box-shadow: inset 0 -1px 0 var(--line);
		scrollbar-width: thin;
	}
	a {
		flex: none;
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		padding: 0 12px;
		border-bottom: 3px solid transparent;
		color: var(--muted);
		font-size: 15px;
		font-weight: 650;
		white-space: nowrap;
		text-decoration: none;
	}
	a:hover {
		color: var(--ink);
	}
	a[aria-selected='true'] {
		color: var(--ink);
		border-bottom-color: var(--ink);
	}
</style>
