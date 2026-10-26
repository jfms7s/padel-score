<!--
	Owned by phase 7 (the tournament). The tournament's own settings on the new-event form. It renders nothing
	until phase 7 fills it in.
-->
<script lang="ts">
	// Unused until then; typed so KindSettings' call is checked.
	let _props: {
		settings: Record<string, unknown>;
		onchange: (settings: Record<string, unknown>) => void;
	} = $props();
</script>
