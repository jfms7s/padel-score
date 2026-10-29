<script lang="ts">
	// The ＋ New sheet: the four ways to start something. A bottom sheet on a phone, a centred
	// dialog from 700px. Native <dialog> gives Escape, focus trapping and focus back on ＋.
	import ChoiceList from '#lib/components/ChoiceList.svelte';
	import { strings } from '#lib/strings.ts';

	let { onclose }: { onclose: () => void } = $props();

	const s = strings.create;
	let dialog: HTMLDialogElement | undefined = $state();

	$effect(() => {
		dialog?.showModal();
	});

	// A tap on the backdrop lands on the dialog itself, not on the sheet inside it.
	function backdrop(e: MouseEvent) {
		if (e.target === dialog) dialog?.close();
	}
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog bind:this={dialog} onclose={onclose} onclick={backdrop} aria-labelledby="new-title">
	<div class="sheet">
		<header>
			<h2 id="new-title">{s.title}</h2>
			<form method="dialog">
				<button class="secondary" type="submit">{s.close}</button>
			</form>
		</header>
		<ChoiceList onpick={() => dialog?.close()} />
	</div>
</dialog>

<style>
	/* Below 700px a bottom sheet. 100% (not 100vw): a modal dialog is fixed to the viewport, so
	   100% is the viewport without any scrollbar, and the sheet adds no horizontal page scroll. */
	dialog {
		inset: 0;
		margin: auto 0 0;
		width: 100%;
		max-width: 100%;
		max-height: calc(100dvh - 32px);
		padding: 0;
		border: 1px solid var(--line);
		border-bottom: 0;
		border-radius: 20px 20px 0 0;
		background: var(--panel);
		color: var(--ink);
	}
	dialog::backdrop {
		background: rgba(5, 11, 18, 0.55);
	}
	.sheet {
		display: flex;
		flex-direction: column;
		gap: 14px;
		padding: 18px 18px calc(18px + env(safe-area-inset-bottom));
	}
	header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}
	h2 {
		margin: 0;
		font-size: 22px;
		font-weight: 750;
		font-variation-settings: 'wdth' 85;
	}
	form {
		display: contents;
	}
	button {
		min-height: var(--tap);
		padding: 0 18px;
		border-radius: 12px;
		font-weight: 650;
		font-size: 16px;
	}
	.secondary {
		border: 1px solid var(--line);
		background: transparent;
		color: var(--ink);
	}
	@media (min-width: 700px) {
		dialog {
			margin: auto;
			width: min(100vw - 32px, 560px);
			border-bottom: 1px solid var(--line);
			border-radius: 18px;
		}
		.sheet {
			padding-bottom: 18px;
		}
		.sheet :global(.list) {
			display: grid;
			grid-template-columns: 1fr 1fr;
		}
	}
</style>
