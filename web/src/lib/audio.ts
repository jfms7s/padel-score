// Browsers keep audio locked until the user interacts with the page. The fence unlocks it on
// the first tap so that a later sound (the non-stop horn) can play without one.

let context: AudioContext | null = null;

/** Create and resume the shared AudioContext; call it from a user gesture. Never throws. */
export function unlockAudio(): AudioContext | null {
	try {
		const Ctor =
			globalThis.AudioContext ??
			(globalThis as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
		if (!Ctor) return null;
		context ??= new Ctor();
		if (context.state === 'suspended') void context.resume().catch(() => {});
		return context;
	} catch {
		return null;
	}
}
