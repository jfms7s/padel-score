<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import { bootSession, session } from '#lib/session.ts';
	import AppRail from '#lib/components/AppRail.svelte';
	import { isBare } from '#lib/nav.ts';
	import { strings } from '#lib/strings.ts';

	let { children }: { children: Snippet } = $props();

	// Once, quietly: signed out is a normal answer here.
	$effect(() => {
		void bootSession();
	});

	// The fence, court and spectator screens are full-screen and anonymous-friendly: no header there.
	const bare = $derived(isBare(page.url.pathname));
	const railed = $derived(!bare && $session.status === 'signed_in');
	const onLogin = $derived(page.url.pathname === '/login');
</script>

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
	nav a {
		padding: 0 4px;
		text-decoration: underline;
		text-underline-offset: 3px;
	}
</style>
