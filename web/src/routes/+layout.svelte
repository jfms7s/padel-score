<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import { bootSession, session } from '#lib/session.ts';
	import { strings } from '#lib/strings.ts';

	let { children }: { children: Snippet } = $props();

	// Once, quietly: signed out is a normal answer here.
	$effect(() => {
		void bootSession();
	});

	// The fence and spectator screens are full-screen and anonymous-friendly: no header there.
	const bare = $derived(/^\/(s|v)\//.test(page.url.pathname));
	const onLogin = $derived(page.url.pathname === '/login');
</script>

<svelte:head>
	<title>{strings.appName}</title>
</svelte:head>

{#if !bare}
	<header>
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

<style>
	header {
		max-width: 28rem;
		margin: 0 auto;
		padding: 4px 16px 0;
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
