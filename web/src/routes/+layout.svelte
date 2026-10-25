<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { strings } from '#lib/strings.ts';

	let { children }: { children: Snippet } = $props();
</script>

<svelte:head>
	<title>{strings.appName}</title>
</svelte:head>

{@render children()}
