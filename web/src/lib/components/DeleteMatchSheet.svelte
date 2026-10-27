<script lang="ts">
	// "Delete this match?" from a home list: for a match started by mistake. A finished match is
	// kept (it has a result), and only leaves the list it was tapped in.
	import { ApiError, deleteMatch } from '#lib/api.ts';
	import { strings } from '#lib/strings.ts';

	let {
		label,
		scorerToken,
		onclose
	}: { label: string; scorerToken: string; onclose: (removed: boolean) => void } = $props();

	const s = strings.deleteMatch;
	let dialog: HTMLDialogElement | undefined = $state();
	let busy = $state(false);
	let failure = $state<string | null>(null);
	let finished = $state(false);
	let removed = false;

	$effect(() => {
		dialog?.showModal();
	});

	async function remove() {
		if (busy) return;
		busy = true;
		failure = null;
		try {
			const result = await deleteMatch(scorerToken);
			removed = true;
			if (result === 'finished') finished = true;
			else dialog?.close();
		} catch (e) {
			if (e instanceof ApiError && e.code === 'event_match') failure = s.eventMatch;
			else failure = e instanceof ApiError ? e.message : strings.networkError;
		} finally {
			busy = false;
		}
	}
</script>

<dialog bind:this={dialog} onclose={() => onclose(removed)} aria-labelledby="delete-title">
	<div class="sheet">
		<h2 id="delete-title">{s.title}</h2>
		<p><strong>{label}</strong></p>
		{#if finished}
			<p class="ok" role="status">{s.finished}</p>
		{:else}
			<p class="hint">{s.hint}</p>
		{/if}
		{#if failure}<p class="error" role="alert">{failure}</p>{/if}
		<div class="actions">
			{#if !finished}
				<button class="danger" type="button" disabled={busy} onclick={remove}>
					{busy ? s.deleting : s.delete}
				</button>
			{/if}
			<form method="dialog">
				<button class="secondary" type="submit">{finished ? s.close : s.cancel}</button>
			</form>
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
	.danger {
		border: 1px solid var(--warn);
		background: transparent;
		color: var(--warn);
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
