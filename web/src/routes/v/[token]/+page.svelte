<script lang="ts">
	// A match's viewing link: the read-only scoreboard.
	import { page } from '$app/state';
	import Viewer from '#lib/components/Viewer.svelte';

	const token = $derived(page.params.token ?? '');
</script>

<Viewer {token} />
