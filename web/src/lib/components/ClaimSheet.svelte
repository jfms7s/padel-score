<script lang="ts">
	// "That's me" on a match or an event's team: what the entry is (name, coordinator, how many
	// matches, no scores), then "Send claim". The coordinator has to approve before anything is
	// linked.
	import { createClaim, linkMessage, previewClaim, previewText } from '#lib/linking.ts';
	import { strings } from '#lib/strings.ts';
	import type { ClaimPreview, ClaimTarget } from '#lib/types.ts';

	let { target, onclose }: { target: ClaimTarget; onclose: (sent: boolean) => void } = $props();

	const s = strings.linking.sheet;
	let dialog: HTMLDialogElement | undefined = $state();
	let preview = $state<ClaimPreview | null>(null);
	let failure = $state<string | null>(null);
	let done = $state<string | null>(null);
	let busy = $state(false);

	$effect(() => {
		dialog?.showModal();
	});

	$effect(() => {
		let live = true;
		previewClaim(target).then(
			(p) => {
				if (live) preview = p;
			},
			(e: unknown) => {
				if (live) failure = linkMessage(e);
			}
		);
		return () => {
			live = false;
		};
	});

	async function send() {
		if (busy || !preview) return;
		busy = true;
		failure = null;
		try {
			const claim = await createClaim(target);
			done = claim.status === 'approved' ? s.linked : s.sent(preview.coordinator_name);
		} catch (e) {
			failure = linkMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

<dialog bind:this={dialog} onclose={() => onclose(done !== null)} aria-labelledby="claim-title">
	<div class="sheet">
		<h2 id="claim-title">{strings.linking.sheet.title}</h2>
		{#if preview}
			<p><strong>{s.is(preview.name, preview.coordinator_name)}</strong></p>
			<p class="hint">{previewText(preview)}</p>
			{#if done}
				<p class="ok" role="status">{done}</p>
			{:else}
				<p class="hint">{s.needsApproval(preview.coordinator_name)}</p>
			{/if}
		{:else if !failure}
			<p class="hint">{s.loading}</p>
		{/if}
		{#if failure}<p class="error" role="alert">{failure}</p>{/if}
		<div class="actions">
			{#if preview && !done}
				<button class="primary" type="button" disabled={busy} onclick={send}>{busy ? s.sending : s.send}</button>
			{/if}
			<form method="dialog"><button class="secondary" type="submit">{s.close}</button></form>
		</div>
	</div>
</dialog>

<style>
	dialog {
		width: min(100vw - 32px, 28rem);
		max-height: calc(100dvh - 32px);
		padding: 0;
		border: 1px solid var(--line);
		border-radius: 18px;
		background: var(--panel);
		color: var(--ink);
	}
	dialog::backdrop {
		background: rgba(5, 11, 18, 0.55);
	}
	.sheet {
		display: flex;
		flex-direction: column;
		gap: 10px;
		padding: 20px 18px;
	}
	h2 {
		margin: 0;
		font-size: 22px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	p {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.hint {
		font-size: 14px;
		color: var(--muted);
	}
	.ok {
		color: var(--ink);
		font-weight: 650;
	}
	.error {
		color: var(--warn);
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.actions form {
		display: contents;
	}
	button {
		min-height: var(--tap);
		padding: 0 18px;
		border-radius: 12px;
		font-weight: 650;
		font-size: 16px;
	}
	.primary {
		border: 0;
		background: var(--ink);
		color: var(--glass);
	}
	.secondary {
		border: 1px solid var(--line);
		background: transparent;
		color: var(--ink);
	}
	button:disabled {
		opacity: 0.6;
	}
</style>
