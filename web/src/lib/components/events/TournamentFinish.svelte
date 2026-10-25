<!--
	Owned by phase 7 (the tournament). The Finish screen once the tournament computes its own places
	(`places_computed` is true): it may preview them, and it finishes with none. Until phase 7
	flips `computes_places`, KindFinish never shows this; the stub just confirms.
-->
<script lang="ts">
	import { strings } from '#lib/strings.ts';
	import type { EventDetail, Place } from '#lib/types.ts';

	const t = strings.events;

	let {
		busy,
		onfinish,
		oncancel
	}: {
		event: EventDetail;
		busy: boolean;
		onfinish: (places?: Place[]) => void;
		oncancel: () => void;
	} = $props();
</script>

<p class="hint">{t.finishComputedHint}</p>
<div class="actions">
	<button class="primary" type="button" disabled={busy} onclick={() => onfinish()}>{busy ? t.finishing : t.finish}</button>
	<button class="secondary" type="button" onclick={oncancel}>{t.cancel}</button>
</div>
