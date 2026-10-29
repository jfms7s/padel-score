<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { beforeNavigate } from '$app/navigation';
	import { page, updated } from '$app/state';
	import { updatesHeld } from '#lib/holds.svelte.ts';
	import { bootSession, session } from '#lib/session.ts';
	import AppRail from '#lib/components/AppRail.svelte';
	import { chromeOf } from '#lib/nav.ts';
	import { safeSessionStorage } from '#lib/storage.ts';
	import { strings } from '#lib/strings.ts';
	import { SHOWN_MS, markReload, mayReload, reloadNow, updateScreen } from '#lib/update.ts';

	let { children }: { children: Snippet } = $props();

	// Once, quietly: signed out is a normal answer here.
	$effect(() => {
		void bootSession();
	});

	// The fence, court and spectator screens are full-screen and anonymous-friendly: no header there.
	const bare = $derived(chromeOf(page.url.pathname) === 'bare');
	const railed = $derived(!bare && $session.status === 'signed_in');
	const onLogin = $derived(page.url.pathname === '/login');

	// A new deploy: SvelteKit notices it (on focus, on return and every minute); switch to it
	// when nothing here can be lost. Until then, a small button reloads on demand.
	let hidden = $state(false);
	let shownAt = -Infinity;
	const screen = $derived(updateScreen(page.url.pathname));

	function visibility() {
		hidden = document.visibilityState === 'hidden';
		if (!hidden) shownAt = performance.now();
	}

	$effect(() => {
		hidden = document.visibilityState === 'hidden';
	});

	$effect(() => {
		if (!updated.current) return;
		const held = updatesHeld();
		const justShown = performance.now() - shownAt < SHOWN_MS;
		if (!reloadNow({ screen, held, hidden, justShown })) return;
		const store = safeSessionStorage();
		if (!mayReload(store, Date.now())) return;
		markReload(store, Date.now());
		location.reload();
	});

	// The app's next in-app navigation loads the new version instead.
	beforeNavigate(({ willUnload, to, cancel }) => {
		if (!updated.current || willUnload || !to?.url || updatesHeld()) return;
		cancel();
		location.href = to.url.href;
	});
</script>

<svelte:document onvisibilitychange={visibility} />

<svelte:head>
	<title>{strings.appName}</title>
</svelte:head>

{#if railed && $session.status === 'signed_in'}
	<AppRail pathname={page.url.pathname} name={$session.me.display_name} />
{/if}

<div class:has-rail={railed}>
	{#if !bare}
		<header class:railed class="page-wide">
			{#if page.url.pathname === '/'}
				<span></span>
			{:else}
				<a class="home" href="/">{strings.appName}</a>
			{/if}
			<nav aria-label={strings.auth.header}>
				{#if $session.status === 'signed_in'}
					<a href="/account">{$session.me.display_name}</a>
				{:else if $session.status === 'signed_out' && !onLogin}
					<a href="/login">{strings.auth.signIn}</a>
				{/if}
			</nav>
		</header>
	{/if}

	{@render children()}
</div>

{#if updated.current && !updatesHeld()}
	<button type="button" class="updated" onclick={() => location.reload()}>{strings.update.ready}</button>
{/if}

<style>
	@media (min-width: 1100px) {
		.has-rail {
			padding-left: 72px;
		}
		header.railed {
			display: none;
		}
	}
	header {
		padding-top: 4px;
		padding-bottom: 0;
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}
	a {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--ink);
		text-decoration: none;
		font-weight: 650;
		overflow-wrap: anywhere;
	}
	.home {
		font-variation-settings: 'wdth' 75;
		font-weight: 800;
	}
	.updated {
		position: fixed;
		left: 50%;
		top: max(8px, env(safe-area-inset-top));
		transform: translateX(-50%);
		max-width: calc(100vw - 32px);
		min-height: 36px;
		padding: 0 14px;
		border: 0;
		border-radius: 999px;
		background: var(--ink);
		color: var(--glass);
		font: inherit;
		font-size: 14px;
		font-weight: 650;
		box-shadow: 0 2px 10px rgb(0 0 0 / 0.25);
		z-index: 20;
	}
	nav a {
		padding: 0 4px;
		text-decoration: underline;
		text-underline-offset: 3px;
	}
</style>
