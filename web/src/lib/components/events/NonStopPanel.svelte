<!--
	Owned by phase 8 (the non-stop). The non-stop's part of the event screen, for the coordinator or a
	spectator. It renders nothing until phase 8 fills it in.
-->
<script lang="ts">
	import type { EventDetail, EventView } from '#lib/types.ts';

	// Unused until then; typed so KindPanel's call is checked.
	let _props: { event: EventDetail | EventView; audience: 'coordinator' | 'spectator' } = $props();
</script>
