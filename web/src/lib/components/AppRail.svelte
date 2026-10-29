<script lang="ts">
	import { initials, navItems } from '#lib/nav.ts';
	import { ICONS } from '#lib/navIcons.ts';
	import { strings } from '#lib/strings.ts';

	let { pathname, name }: { pathname: string; name: string } = $props();

	const items = $derived(navItems(pathname));
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
		<a
			href={item.href}
			class:current={item.current}
			class:plus={item.key === 'new'}
			aria-current={item.current ? 'page' : undefined}
		>
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
		width: 64px;
		display: none;
		flex-direction: column;
		align-items: stretch;
		gap: 4px;
		padding: 12px 6px;
		background: var(--glass);
		border-right: 1px solid var(--line);
		z-index: 10;
	}
	@media (min-width: 700px) {
		nav {
			display: flex;
		}
	}
	@media (min-width: 1100px) {
		nav {
			width: 72px;
		}
		a span {
			font-size: 11px;
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
		font-size: 10px;
		font-weight: 650;
		line-height: 1.1;
		text-align: center;
	}
	a.current {
		background: var(--a-soft);
	}
	a.plus svg {
		box-sizing: content-box;
		padding: 8px;
		border-radius: 12px;
		background: var(--ink);
		color: var(--glass);
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
