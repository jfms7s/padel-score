<script lang="ts">
	// The scorer's fence screen for a match's scoring link (see Fence).
	import { page } from '$app/state';
	import Fence from '#lib/components/Fence.svelte';

	const token = $derived(page.params.token ?? '');
</script>

<Fence {token} />
