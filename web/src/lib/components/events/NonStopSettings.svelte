<!--
	Owned by phase 8 (the non-stop). The non-stop's own settings on the new-event form. It renders nothing
	until phase 8 fills it in.
-->
<script lang="ts">
	// Unused until then; typed so KindSettings' call is checked.
	let _props: {
		settings: Record<string, unknown>;
		onchange: (settings: Record<string, unknown>) => void;
	} = $props();
</script>
