<!--
	The Play later fields of New match: the day and time (required), a place and who can find the
	match (link only or public). The page holds the draft (see scheduled.ts `LaterDraft`) and
	saves it; the four places are the team fields above. Times are read in the device's zone.
-->
<script lang="ts">
	import { browserTimeZone } from '#lib/dates.ts';
	import type { LaterDraft } from '#lib/scheduled.ts';
	import { strings } from '#lib/strings.ts';
	import type { Listing } from '#lib/types.ts';

	let { draft = $bindable() }: { draft: LaterDraft } = $props();

	const t = strings.scheduled;
	const uid = $props.id();
	const LISTINGS: Listing[] = ['private', 'public'];
	const MAX_PLACE_LENGTH = 80;
	const zone = browserTimeZone();
</script>

<fieldset>
	<legend>{t.fieldsTitle}</legend>
	<div class="pair">
		<label>
			{t.date}
			<input type="date" required bind:value={draft.date} />
		</label>
		<label>
			{t.time}
			<input type="time" required bind:value={draft.time} />
		</label>
	</div>
	<p class="hint">{t.timeZoneNote(zone)}</p>
	<label>
		{t.place}
		<input bind:value={draft.place} placeholder={t.placePlaceholder} autocomplete="off" maxlength={MAX_PLACE_LENGTH} />
	</label>
	<div role="radiogroup" aria-labelledby="{uid}-listing" aria-describedby="{uid}-listing-hint">
		<span class="label" id="{uid}-listing">{t.listing}</span>
		<div class="options">
			{#each LISTINGS as listing (listing)}
				<label class="option">
					<input type="radio" name="{uid}-listing" value={listing} bind:group={draft.listing} />
					<span>{t.listings[listing]}</span>
				</label>
			{/each}
		</div>
		<p class="hint" id="{uid}-listing-hint">{t.listingHints[draft.listing]}</p>
	</div>
</fieldset>

<style>
	fieldset {
		display: flex;
		flex-direction: column;
		gap: 14px;
		min-width: 0;
		margin: 0;
		padding: 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
	}
	legend {
		padding: 0 6px;
		font-size: 13px;
		font-weight: 650;
		color: var(--muted);
	}
	label:not(.option) {
		display: flex;
		flex-direction: column;
		gap: 6px;
		min-width: 0;
		font-size: 13px;
		color: var(--muted);
	}
	input:not([type='radio']) {
		min-height: var(--tap);
		width: 100%;
		padding: 0 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
		font-size: 16px;
		color: var(--ink);
	}
	input:not([type='radio']):focus-visible {
		outline: 2px solid var(--a);
		outline-offset: 2px;
	}
	.pair {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 10px;
	}
	.hint {
		margin: 6px 0 0;
		font-size: 12px;
		color: var(--muted);
	}
	.label {
		display: block;
		font-size: 13px;
		color: var(--muted);
		margin-bottom: 6px;
	}
	.options {
		display: flex;
		gap: 8px;
	}
	.option {
		flex: 1;
		display: flex;
		align-items: center;
		gap: 8px;
		min-height: var(--tap);
		padding: 0 12px;
		border: 1px solid var(--line);
		border-radius: 12px;
		background: var(--panel);
	}
	.option input {
		width: 20px;
		height: 20px;
		min-height: 0;
		margin: 0;
	}
	.option:has(input:checked) {
		border-color: var(--ink);
		font-weight: 650;
	}
	.option:has(input:focus-visible) {
		outline: 2px solid var(--a);
		outline-offset: 2px;
	}
</style>
