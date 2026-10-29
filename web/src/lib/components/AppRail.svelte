<script lang="ts">
	import { initials, navItems, type NavKey } from '#lib/nav.ts';
	import { strings } from '#lib/strings.ts';

	let { pathname, name }: { pathname: string; name: string } = $props();

	const items = $derived(navItems(pathname));
	const ICONS: Record<NavKey, string> = {
		home: 'M4 11l8-7 8 7v9a1 1 0 0 1-1 1h-4v-6H9v6H5a1 1 0 0 1-1-1z',
		organise:
			'M7 4h10v4a5 5 0 0 1-10 0zM5 5H3v2a3 3 0 0 0 4 3M19 5h2v2a3 3 0 0 1-4 3M12 13v4M8 20h8',
		new: 'M12 5v14M5 12h14',
		results: 'M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z',
		people:
			'M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM2.5 20a6.5 6.5 0 0 1 13 0M16 4.5a3.5 3.5 0 0 1 0 6.5M18 14a6.5 6.5 0 0 1 3.5 6'
	};
</script>

<nav aria-label={strings.nav.label}>
	<a class="logo" href="/" aria-label={strings.appName}>
		<svg viewBox="0 0 24 24" width="32" height="32" aria-hidden="true">
			<circle cx="12" cy="12" r="10" fill="var(--ball)" />
			<path
				d="M3.5 8.5c4 1 7 4 8.5 8.5M20.5 15.5c-4-1-7-4-8.5-8.5"
				fill="none"
				stroke="var(--on-ball)"
				stroke-width="1.5"
			/>
		</svg>
	</a>
	{#each items as item (item.key)}
		<a href={item.href} class:current={item.current} aria-current={item.current ? 'page' : undefined}>
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
			<span>{item.label}</span>
		</a>
	{/each}
	<a class="who" href="/account" aria-label={strings.nav.account} title={name}>
		<span class="circle">{initials(name)}</span>
	</a>
</nav>

<style>
	nav {
		position: fixed;
		inset: 0 auto 0 0;
		width: 72px;
		display: none;
		flex-direction: column;
		align-items: stretch;
		gap: 4px;
		padding: 12px 6px;
		background: var(--glass);
		border-right: 1px solid var(--line);
		z-index: 10;
	}
	@media (min-width: 1100px) {
		nav {
			display: flex;
		}
	}
	a {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 2px;
		min-height: var(--tap);
		padding: 6px 2px;
		border-radius: 10px;
		color: var(--ink);
		text-decoration: none;
	}
	a span {
		font-size: 11px;
		font-weight: 650;
		line-height: 1.1;
		text-align: center;
	}
	a.current {
		background: var(--a-soft);
	}
	.logo {
		margin-bottom: 8px;
	}
	.who {
		margin-top: auto;
	}
	.circle {
		display: grid;
		place-items: center;
		width: 36px;
		height: 36px;
		border-radius: 50%;
		background: var(--a);
		color: var(--on-team);
		font-size: 13px;
		font-weight: 800;
	}
</style>
