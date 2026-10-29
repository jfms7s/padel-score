<script lang="ts">
	import { page } from '$app/state';
	import { parentOf } from '#lib/nav.ts';
	import { strings } from '#lib/strings.ts';

	const parent = $derived(parentOf(page.url.pathname));
</script>

{#if parent}<a class="back" href={parent.href}>{strings.nav.back(parent.label)}</a>{/if}

<style>
	.back {
		display: inline-flex;
		align-items: center;
		min-height: var(--tap);
		color: var(--muted);
	}
</style>
