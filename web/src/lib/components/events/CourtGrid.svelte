<!--
	The grid the court tiles sit in, on the desk's Live tab and the spectator page. A tablet fits
	up to three courts in a row, more in two columns; from 1100 px the tiles fill the width.
-->
<script lang="ts">
	import type { Snippet } from 'svelte';

	let { count, children }: { count: number; children: Snippet } = $props();
</script>

<div class="court-grid" style:--cols={count <= 3 ? count : 2}>
	{@render children()}
</div>

<style>
	.court-grid {
		display: grid;
		gap: 12px;
		grid-template-columns: 1fr;
	}
	@media (min-width: 700px) {
		.court-grid {
			grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
		}
	}
	@media (min-width: 1100px) {
		.court-grid {
			grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		}
	}
</style>
