<script lang="ts">
	// What waits for the signed-in user: swap and time requests, partner invites, claims to
	// approve and the notifications opt-in. Draws nothing when there is none of them.
	import RequestCard from '#lib/components/boards/RequestCard.svelte';
	import { strings } from '#lib/strings.ts';
	import type { Calendar, WaitingItem } from '#lib/types.ts';

	let {
		cal,
		claims,
		pushCard,
		onanswer,
		onresend,
		onpush,
		onnotnow,
		busy,
		error,
		pushFailure,
		note
	}: {
		cal: Calendar;
		claims: number;
		pushCard: boolean;
		onanswer: (id: string, accept: boolean) => void;
		onresend: (w: Extract<WaitingItem, { kind: 'partner' }>) => void;
		onpush: () => void;
		onnotnow: () => void;
		busy: boolean;
		error: string | null;
		pushFailure: string | null;
		note: string | null;
	} = $props();

	const t = strings.calendar;
	const h = strings.home;
	const empty = $derived(cal.waiting.length === 0 && claims === 0 && !pushCard);
</script>

{#if !empty}
	<section aria-labelledby="waiting">
		<h2 id="waiting">{h.waiting}</h2>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<ul>
			{#each cal.waiting as w (w.kind === 'partner' ? `p:${w.partner_token}` : w.request_id)}
				<li>
					{#if w.kind === 'swap'}
						<RequestCard
							kind="swap"
							from={w.from_name}
							current={w.my_game}
							proposed={w.their_game}
							{busy}
							onanswer={(accept) => onanswer(w.request_id, accept)}
						/>
					{:else if w.kind === 'time'}
						<RequestCard
							kind="time"
							from={w.from_name}
							current={w.game}
							proposed={w.new}
							{busy}
							onanswer={(accept) => onanswer(w.request_id, accept)}
						/>
					{:else}
						<div class="card">
							<h3>{t.partnerWaiting(w.event_name)}</h3>
							<p class="hint">{t.partnerHint}</p>
							<button type="button" class="secondary" onclick={() => onresend(w)}>{t.partnerResend}</button>
						</div>
					{/if}
				</li>
			{/each}
			{#if claims > 0}
				<li>
					<a class="card claims" href="/players">
						<span>{h.claims(claims)}</span>
						<span class="review">{h.review}</span>
					</a>
				</li>
			{/if}
			{#if pushCard}
				<li>
					<div class="card">
						<h3>{t.pushTitle}</h3>
						<p class="hint">{t.pushHint}</p>
						{#if pushFailure}<p class="error" role="alert">{pushFailure}</p>{/if}
						<div class="actions">
							<button type="button" class="primary" onclick={onpush}>{t.pushOn}</button>
							<button type="button" class="secondary" onclick={onnotnow}>{t.pushNotNow}</button>
						</div>
					</div>
				</li>
			{/if}
		</ul>
		{#if note}<p class="ok" role="status">{note}</p>{/if}
	</section>
{/if}

<style>
	h2 {
		margin: 24px 0 8px;
		font-size: 18px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	h3 {
		margin: 0 0 4px;
		font-size: 16px;
		overflow-wrap: anywhere;
	}
	.hint {
		margin: 0 0 8px;
		color: var(--muted);
	}
	.error {
		color: var(--warn);
	}
	.ok {
		color: var(--muted);
	}
	ul {
		list-style: none;
		margin: 0 0 8px;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	.card {
		padding: 12px 14px;
		border: 1px solid var(--line);
		border-radius: 14px;
		background: var(--panel);
	}
	.claims {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		min-height: var(--tap);
		color: var(--ink);
		font-weight: 650;
		text-decoration: none;
	}
	.review {
		color: var(--a);
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	button {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		min-height: var(--tap);
		padding: 0 18px;
		border-radius: 14px;
		font-size: 16px;
		font-weight: 650;
	}
	.primary {
		border: 0;
		background: var(--ink);
		color: var(--glass);
	}
	.secondary {
		border: 1px solid var(--line);
		background: var(--panel);
		color: var(--ink);
	}
</style>
