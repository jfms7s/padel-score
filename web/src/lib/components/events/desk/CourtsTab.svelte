<!--
	The desk's Courts & phones tab: each court's scoring and viewing QR codes with a new scoring
	code and Next match, the whole event's viewing code, and Add court.
-->
<script lang="ts">
	import QrCode from '#lib/components/QrCode.svelte';
	import { courtLinks, courtStatusText, eventLink } from '#lib/events.ts';
	import { strings } from '#lib/strings.ts';
	import type { CourtOut, EventDetail, EventMatch } from '#lib/types.ts';

	let {
		detail,
		matches,
		origin,
		busy,
		failure,
		copied,
		copyFailed,
		oncopy,
		onrotate,
		onnext,
		onaddcourt
	}: {
		detail: EventDetail;
		/** The matches in their order. */
		matches: EventMatch[];
		origin: string;
		busy: boolean;
		failure: string | undefined;
		/** The key of the link just copied. */
		copied: string | null;
		copyFailed: boolean;
		oncopy: (key: string, link: string) => void;
		onrotate: (court: CourtOut) => Promise<boolean>;
		onnext: (court: CourtOut) => void;
		onaddcourt: () => void;
	} = $props();

	const t = strings.events;

	const status = $derived(detail.event.status);
	const courts = $derived(detail.courts);
	const teams = $derived(detail.teams);

	let rotating = $state<string | null>(null);

	const matchById = (mid: string | null) => (mid === null ? undefined : matches.find((m) => m.id === mid));

	async function confirmRotate(court: CourtOut) {
		if (await onrotate(court)) rotating = null;
	}
</script>

<section aria-labelledby="h-courts">
	<h2 id="h-courts">{t.courts}</h2>
	<!-- Once finished, scoring links give 410: only the viewing QR stays (and no rotate). -->
	<div class="cards grid-auto" style:--min="300px">
		{#each courts as court (court.id)}
			{@const links = courtLinks(origin, court)}
			{@const current = matchById(court.current)}
			{@const items = [
				...(status === 'finished' ? [] : [{ key: 'scoring', label: t.scoring, qr: t.scoringQr(court.name), link: links.scoring }]),
				{ key: 'viewing', label: t.viewing, qr: t.viewingQr(court.name), link: links.viewing }
			]}
			<article class="card" aria-labelledby="court-{court.id}">
				<h3 id="court-{court.id}">{court.name}</h3>
				<p class="st">{courtStatusText(court, matches, teams)}</p>
				<div class="qrs">
					{#each items as item (item.key)}
						{@const copyKey = `${court.id}-${item.key}`}
						<div class="qr">
							<figure>
								<QrCode text={item.link} label={item.qr} />
								<figcaption>{item.label}</figcaption>
							</figure>
							<button class="secondary small" type="button" aria-label={t.copyLabel(court.name, item.label)} onclick={() => oncopy(copyKey, item.link)}>
								{copied === copyKey ? t.copied : t.copy}
							</button>
						</div>
					{/each}
				</div>
				{#if status !== 'finished'}
					<div class="actions">
						{#if status === 'running' && current?.state === 'finished'}
							<button class="primary" type="button" disabled={busy} onclick={() => onnext(court)}>{t.nextMatch}</button>
						{/if}
						{#if rotating === court.id}
							<div class="confirm" role="group" aria-label={t.rotate}>
								<p>{t.rotateConfirm}</p>
								<div class="actions">
									<button class="danger" type="button" disabled={busy} onclick={() => confirmRotate(court)}>{t.confirm}</button>
									<button class="secondary" type="button" onclick={() => (rotating = null)}>{t.cancel}</button>
								</div>
							</div>
						{:else}
							<button class="secondary" type="button" disabled={busy} onclick={() => (rotating = court.id)}>{t.rotate}</button>
						{/if}
					</div>
				{/if}
			</article>
		{/each}

		<article class="card" aria-labelledby="court-whole">
			<h3 id="court-whole">{t.wholeEvent}</h3>
			<p class="st">{t.wholeEventHint}</p>
			<div class="qrs">
				<div class="qr">
					<figure>
						<QrCode text={eventLink(origin, detail.spectator_token)} label={t.wholeEventQr} />
						<figcaption>{t.viewing}</figcaption>
					</figure>
					<button
						class="secondary small"
						type="button"
						aria-label={t.copyLabel(t.wholeEventPlace, t.viewing)}
						onclick={() => oncopy('whole', eventLink(origin, detail.spectator_token))}
					>
						{copied === 'whole' ? t.copied : t.copy}
					</button>
				</div>
			</div>
		</article>
	</div>
	{#if copyFailed}<p class="hint" role="alert">{t.copyFailed}</p>{/if}
	{#if status !== 'finished'}
		<button class="secondary wide" type="button" disabled={busy} onclick={onaddcourt}>{t.addCourt}</button>
	{/if}
	{#if failure}<p class="error" role="alert">{failure}</p>{/if}
</section>

<style>
	section {
		margin-top: 8px;
	}
	.cards {
		display: grid;
		gap: 12px;
	}
	.card {
		border: 1px solid var(--line);
		border-radius: 16px;
		padding: 16px;
		background: var(--glass);
		min-width: 0;
	}
	.card h3 {
		margin: 0 0 2px;
		font-size: 18px;
		overflow-wrap: anywhere;
	}
	.card .st {
		font-size: 13px;
		color: var(--muted);
		margin-bottom: 12px;
		overflow-wrap: anywhere;
	}
	.qrs {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 14px;
		margin-bottom: 12px;
	}
	.qr {
		margin: 0;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		min-width: 0;
		text-align: center;
		font-size: 13px;
		color: var(--muted);
	}
	.qr figure {
		margin: 0;
		width: 100%;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 4px;
	}
	.qr :global(.code) {
		width: 100%;
		max-width: 180px;
	}
	.small {
		width: 100%;
		padding: 0 8px !important;
		font-size: 14px !important;
	}
	.confirm {
		width: 100%;
	}
	.confirm p {
		margin: 0 0 8px;
		font-size: 14px;
	}
	.wide {
		width: 100%;
		max-width: 28rem;
		margin-top: 12px;
	}
	p.error {
		margin-top: 12px;
	}
</style>
